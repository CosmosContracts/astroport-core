use cosmwasm_std::{
    attr, ensure, wasm_execute, Addr, BankMsg, Deps, DepsMut, Env, MessageInfo, Order, ReplyOn,
    Response, StdError, StdResult, Storage, Uint128,
};
use itertools::Itertools;

use crate::error::ContractError;
use crate::reply::register_pending_transfer;
use crate::state::{
    Op, PoolInfo, UserInfo, ACTIVE_POOLS, BLOCKED_TOKENS, CONFIG, ORPHANED_REWARDS,
};
use astroport::asset::{
    determine_asset_info, pair_info_by_pool, AssetInfo, AssetInfoExt, PairInfo,
};
use astroport::common::LP_SUBDENOM;
use astroport::factory::PairType;
use astroport::incentives::{Config, IncentivesSchedule, InputSchedule, MAX_ORPHANED_REWARD_LIMIT};
use astroport::{factory, pair};

/// Claim all rewards and compose [`Response`] object containing all attributes and messages.
/// Mutates in-memory `PoolInfo` / `UserInfo` objects passed via `pool_tuples`;
/// the caller is responsible for `save`-ing those back to storage.
///
/// The function also registers per-transfer pending payloads in
/// `PENDING_REWARD_TRANSFERS` (via `register_pending_transfer`) so the reply
/// handler can route failed transfers into `ORPHANED_REWARDS`. This is the
/// only direct storage mutation `claim_rewards` performs.
///
/// Astroport-Juno change (P2.5): internal protocol rewards are paid directly
/// from this contract's own balance (a `BankMsg::Send` for native or
/// `cw20::Transfer` for cw20), not through a vesting contract. Upstream's
/// `astroport-vesting` dep was stripped — the DAO refunds this contract's
/// bank balance via `BankMsg::Send` when budget runs low.
pub fn claim_rewards(
    storage: &mut dyn Storage,
    config: &Config,
    env: Env,
    user: &Addr,
    pool_tuples: Vec<(&AssetInfo, &mut PoolInfo, &mut UserInfo)>,
) -> Result<Response, ContractError> {
    let mut attrs = vec![attr("action", "claim_rewards"), attr("user", user)];
    let mut external_rewards = vec![];
    let mut protocol_reward_amount = Uint128::zero();
    for (lp_token_asset, pool_info, pos) in pool_tuples {
        attrs.push(attr("claimed_position", lp_token_asset.to_string()));

        pool_info.update_rewards(storage, &env, lp_token_asset)?;

        // Claim outstanding rewards from finished schedules
        for finished_reward in pos.claim_finished_rewards(storage, lp_token_asset, pool_info)? {
            if !finished_reward.amount.is_zero() {
                attrs.push(attr("claimed_finished_reward", finished_reward.to_string()));
                external_rewards.push(finished_reward);
            }
        }

        // Reset user reward index for all finished schedules
        pos.reset_user_index(storage, lp_token_asset, pool_info)?;

        for (is_external, reward_asset) in pool_info.calculate_rewards(pos)? {
            attrs.push(attr("claimed_reward", reward_asset.to_string()));

            if !reward_asset.amount.is_zero() {
                if is_external {
                    external_rewards.push(reward_asset);
                } else {
                    protocol_reward_amount += reward_asset.amount;
                }
            }
        }

        // Sync user index with pool index. It removes all finished schedules from user info.
        pos.update_and_sync_position(Op::Noop, pool_info);
    }

    // Aggregating rewards by asset info.
    // This allows to reduce number of output messages thus reducing total gas cost.
    // Each transfer gets a unique reply id (via `register_pending_transfer`) and
    // `ReplyOn::Always` so the reply handler can either GC the entry on success
    // or credit `ORPHANED_REWARDS` on failure — the stuck-funds bug fix.
    let aggregated: Vec<(AssetInfo, Uint128)> = external_rewards
        .into_iter()
        .group_by(|asset| asset.info.clone())
        .into_iter()
        .map(|(info, assets)| {
            let amount: Uint128 = assets.into_iter().map(|asset| asset.amount).sum();
            (info, amount)
        })
        .collect();

    let mut messages = Vec::with_capacity(aggregated.len() + 1);
    for (info, amount) in aggregated {
        let reply_id = register_pending_transfer(storage, &info, amount)?;
        let submsg = info.with_balance(amount).into_submsg(
            user,
            Some((ReplyOn::Always, reply_id)),
            config.token_transfer_gas_limit,
        )?;
        messages.push(submsg);
    }

    // Pay protocol rewards directly from this contract's balance.
    // Native: BankMsg::Send. cw20: cw20::ExecuteMsg::Transfer.
    if !protocol_reward_amount.is_zero() {
        let reply_id =
            register_pending_transfer(storage, &config.reward_token, protocol_reward_amount)?;
        let reward_asset = config.reward_token.with_balance(protocol_reward_amount);
        let transfer = reward_asset.into_submsg(
            user,
            Some((ReplyOn::Always, reply_id)),
            config.token_transfer_gas_limit,
        )?;
        messages.push(transfer);
    }

    Ok(Response::new()
        .add_attributes(attrs)
        .add_submessages(messages))
}

/// Only factory can set the allocation points to zero for the specified pool.
/// Called from deregistration context in factory.
pub fn deactivate_pool(
    deps: DepsMut,
    info: MessageInfo,
    env: Env,
    lp_token: String,
) -> Result<Response, ContractError> {
    let mut config = CONFIG.load(deps.storage)?;

    if info.sender != config.factory {
        return Err(ContractError::Unauthorized {});
    }

    let lp_token_asset = determine_asset_info(&lp_token, deps.api)?;

    match PoolInfo::may_load(deps.storage, &lp_token_asset)? {
        Some(mut pool_info) if pool_info.is_active_pool() => {
            let mut active_pools = ACTIVE_POOLS.load(deps.storage)?;

            // `is_active_pool()` (state.rs) and `ACTIVE_POOLS` are two sides of
            // the same invariant: every pool whose internal-reward rps is
            // non-zero must appear in `ACTIVE_POOLS`. If they desync (e.g. via
            // a buggy migration), surface a typed error instead of unwrapping —
            // a panic at this point would brick `DeactivatePool` for the
            // affected pool with no way to recover other than another migration.
            let (ind, _) = active_pools
                .iter()
                .find_position(|(lp_asset, _)| lp_asset == &lp_token_asset)
                .ok_or_else(|| ContractError::ActivePoolInvariantBroken {
                    lp_token: lp_token.clone(),
                })?;
            let (_, alloc_points) = active_pools.swap_remove(ind);

            pool_info.update_rewards(deps.storage, &env, &lp_token_asset)?;
            pool_info.disable_internal_rewards();
            pool_info.save(deps.storage, &lp_token_asset)?;

            config.total_alloc_points = config.total_alloc_points.checked_sub(alloc_points)?;

            for (lp_asset, alloc_points) in &active_pools {
                let mut pool_info = PoolInfo::load(deps.storage, lp_asset)?;
                pool_info.update_rewards(deps.storage, &env, lp_asset)?;
                pool_info.set_internal_rewards(&config, *alloc_points);
                pool_info.save(deps.storage, lp_asset)?;
            }

            ACTIVE_POOLS.save(deps.storage, &active_pools)?;
            CONFIG.save(deps.storage, &config)?;

            Ok(Response::new().add_attributes([
                attr("action", "deactivate_pool"),
                attr("lp_token", lp_token),
            ]))
        }
        _ => Ok(Response::new()),
    }
}

/// Removes pools from active pools if their pair type is blocked.
pub fn deactivate_blocked_pools(deps: DepsMut, env: Env) -> Result<Response, ContractError> {
    let mut response = Response::new();
    let mut active_pools = ACTIVE_POOLS.load(deps.storage)?;
    let mut config = CONFIG.load(deps.storage)?;

    let blocked_pair_types: Vec<PairType> = deps
        .querier
        .query_wasm_smart(&config.factory, &factory::QueryMsg::BlacklistedPairTypes {})?;

    let mut to_remove = vec![];

    for (lp_token_asset, alloc_points) in &active_pools {
        let mut pool_info = PoolInfo::load(deps.storage, lp_token_asset)?;

        let pair_info = query_pair_info(deps.as_ref(), lp_token_asset)?;

        // check if pair type is blocked
        if blocked_pair_types.contains(&pair_info.pair_type) {
            pool_info.update_rewards(deps.storage, &env, lp_token_asset)?;
            pool_info.disable_internal_rewards();
            pool_info.save(deps.storage, lp_token_asset)?;

            config.total_alloc_points = config.total_alloc_points.checked_sub(*alloc_points)?;

            to_remove.push(lp_token_asset.clone());

            response.attributes.extend([
                attr("action", "deactivate_pool"),
                attr("lp_token", lp_token_asset.to_string()),
            ]);
        }
    }

    if !to_remove.is_empty() {
        active_pools.retain(|(lp_token_asset, _)| !to_remove.contains(lp_token_asset));

        for (lp_asset, alloc_points) in &active_pools {
            let mut pool_info = PoolInfo::load(deps.storage, lp_asset)?;
            pool_info.update_rewards(deps.storage, &env, lp_asset)?;
            pool_info.set_internal_rewards(&config, *alloc_points);
            pool_info.save(deps.storage, lp_asset)?;
        }

        ACTIVE_POOLS.save(deps.storage, &active_pools)?;
        CONFIG.save(deps.storage, &config)?;
    }

    Ok(response)
}

pub fn incentivize(
    deps: DepsMut,
    info: &mut MessageInfo,
    env: &Env,
    response: Response,
    lp_token: String,
    input: InputSchedule,
) -> Result<Response, ContractError> {
    let schedule = IncentivesSchedule::from_input(env, &input)?;

    let mut response = response.add_attributes([
        attr("action", "incentivize"),
        attr("lp_token", lp_token.clone()),
        attr("start_ts", env.block.time.seconds().to_string()),
        attr("end_ts", schedule.end_ts.to_string()),
        attr("reward", schedule.reward_info.to_string()),
    ]);

    let lp_token_asset = determine_asset_info(&lp_token, deps.api)?;

    // Prohibit reward schedules with blocked token
    if BLOCKED_TOKENS.has(deps.storage, &asset_info_key(&schedule.reward_info)) {
        return Err(ContractError::BlockedToken {
            token: schedule.reward_info.to_string(),
        });
    }

    let config = CONFIG.load(deps.storage)?;
    is_valid_pool(deps.as_ref(), &config, &lp_token_asset)?;

    let mut pool_info = PoolInfo::may_load(deps.storage, &lp_token_asset)?.unwrap_or_default();
    pool_info.update_rewards(deps.storage, env, &lp_token_asset)?;

    let rewards_number_before = pool_info.rewards.len();
    pool_info.incentivize(
        deps.storage,
        &lp_token_asset,
        &schedule,
        &config.reward_token,
    )?;

    // Check whether this is a new external reward token.
    // 3rd parties are encouraged to keep endless schedules without breaks even with the small rewards.
    // Otherwise, reward token will be removed from the pool info and go to outstanding rewards.
    // Next schedules with the same token will be considered as "new".
    // ASTRO rewards don't require incentivize fee.
    if rewards_number_before < pool_info.rewards.len()
        && schedule.reward_info != config.reward_token
    {
        // If fee set we expect to receive it
        if let Some(incentivization_fee_info) = &config.incentivization_fee_info {
            info.funds
                .iter_mut()
                .find(|coin| coin.denom == incentivization_fee_info.fee.denom)
                .and_then(|found| {
                    found.amount = found
                        .amount
                        .checked_sub(incentivization_fee_info.fee.amount)
                        .ok()?;
                    Some(())
                })
                .ok_or_else(|| ContractError::IncentivizationFeeExpected {
                    fee: incentivization_fee_info.fee.to_string(),
                    lp_token: lp_token.clone(),
                    new_reward_token: schedule.reward_info.to_string(),
                })?;

            // Send fee to fee receiver
            response = response.add_message(BankMsg::Send {
                to_address: incentivization_fee_info.fee_receiver.to_string(),
                amount: vec![incentivization_fee_info.fee.clone()],
            });
        }
    }

    // Assert that we received reward tokens
    match &schedule.reward_info {
        AssetInfo::Token { contract_addr } => {
            response = response.add_message(wasm_execute(
                contract_addr,
                &cw20::Cw20ExecuteMsg::TransferFrom {
                    owner: info.sender.to_string(),
                    recipient: env.contract.address.to_string(),
                    amount: input.reward.amount,
                },
                vec![],
            )?);
        }
        AssetInfo::NativeToken { denom } => {
            // Mutate funds array
            info.funds
                .iter_mut()
                .find(|coin| coin.denom.eq(denom))
                .and_then(|found| {
                    found.amount = found.amount.checked_sub(input.reward.amount).ok()?;
                    Some(())
                })
                .ok_or_else(|| ContractError::InsuffiicientRewardToken {
                    reward: input.reward.info.to_string(),
                    lp_token,
                })?;
        }
    }

    pool_info.save(deps.storage, &lp_token_asset)?;

    Ok(response)
}

pub fn incentivize_many(
    mut deps: DepsMut,
    mut info: MessageInfo,
    env: Env,
    incentives: Vec<(String, InputSchedule)>,
) -> Result<Response, ContractError> {
    let mut response = Response::default();
    for (lp_token, schedule) in incentives {
        response = incentivize(deps.branch(), &mut info, &env, response, lp_token, schedule)?;
    }

    for coin in info.funds {
        ensure!(
            coin.amount.is_zero(),
            StdError::generic_err(format!(
                "Supplied coins contain {} that is not in the input asset vector",
                &coin.denom
            ))
        );
    }

    Ok(response)
}

pub fn remove_reward_from_pool(
    deps: DepsMut,
    info: MessageInfo,
    env: Env,
    lp_token: String,
    reward: String,
    bypass_upcoming_schedules: bool,
    receiver: String,
) -> Result<Response, ContractError> {
    let config = CONFIG.load(deps.storage)?;
    if info.sender != config.owner {
        return Err(ContractError::Unauthorized {});
    }

    let lp_asset = determine_asset_info(&lp_token, deps.api)?;
    let reward_asset = determine_asset_info(&reward, deps.api)?;

    let mut pool_info = PoolInfo::load(deps.storage, &lp_asset)?;
    pool_info.update_rewards(deps.storage, &env, &lp_asset)?;
    let unclaimed = pool_info.deregister_reward(
        deps.storage,
        &lp_asset,
        &reward_asset,
        bypass_upcoming_schedules,
    )?;

    pool_info.save(deps.storage, &lp_asset)?;

    let mut response = Response::new();

    // Send unclaimed rewards. Use the per-transfer pending payload scheme so
    // a failed bank/cw20 send routes the funds into `ORPHANED_REWARDS` instead
    // of being lost — same orphan-on-failure invariant as `claim_rewards`.
    if !unclaimed.is_zero() {
        deps.api.addr_validate(&receiver)?;
        let reply_id = register_pending_transfer(deps.storage, &reward_asset, unclaimed)?;
        let transfer_msg = reward_asset.with_balance(unclaimed).into_submsg(
            receiver,
            Some((ReplyOn::Always, reply_id)),
            config.token_transfer_gas_limit,
        )?;
        response = response.add_submessage(transfer_msg);
    }

    Ok(response.add_attributes([
        attr("action", "remove_reward_from_pool"),
        attr("lp_token", lp_token),
        attr("reward", reward),
    ]))
}

/// Queries pair info corresponding to given LP token.
/// Handles both native and cw20 tokens. If the token is native, it must follow the following format:
/// factory/{lp_minter}/{token_name} where lp_minter is a valid bech32 address on the current chain.
pub fn query_pair_info(deps: Deps, lp_asset: &AssetInfo) -> StdResult<PairInfo> {
    match lp_asset {
        AssetInfo::Token { contract_addr } => pair_info_by_pool(&deps.querier, contract_addr),
        AssetInfo::NativeToken { denom } => {
            let lp_minter = get_pair_from_denom(deps, denom)?;
            deps.querier
                .query_wasm_smart(lp_minter, &pair::QueryMsg::Pair {})
        }
    }
}

pub fn get_pair_from_denom(deps: Deps, denom: &str) -> StdResult<Addr> {
    let parts = denom.split('/').collect_vec();
    if denom.starts_with("factory") && denom.ends_with(LP_SUBDENOM) {
        let lp_minter = parts[1];
        deps.api.addr_validate(lp_minter)
    } else {
        Err(StdError::generic_err(format!(
            "LP token {denom} doesn't follow token factory format: factory/{{lp_minter}}/{{token_name}}",
        )))
    }
}

/// Checks if the pool with the following asset infos is registered in the factory contract and
/// LP tokens address/denom matches the one registered in the factory.
///
/// Astroport-Juno hardening (audit findings HIGH x2):
///
/// rc3 first pass: the upstream native-token branch only validated the bech32
/// shape of the embedded lp-minter, which let any caller invent a
/// `factory/<some-valid-addr>/<lp-suffix>` denom and seed fake rewards. rc3
/// added a `pair::QueryMsg::Pair {}` round-trip against the alleged
/// lp-minter — but that query targets the attacker's own contract, which
/// can forge the response, so this gate alone is insufficient.
///
/// rc4 closure: after the pair-self-query, also consult the factory's PAIRS
/// registry (mirroring the cw20 branch at lines 478-503) and require that
/// (a) the factory has a pair registered for `pair_info.asset_infos` AND
/// (b) the registered `contract_addr` equals the lp-minter AND
/// (c) the registered `liquidity_token` equals the supplied denom.
/// Spoofing now requires subverting the factory registry, which is
/// owner-gated.
pub fn is_valid_pool(deps: Deps, config: &Config, lp_token: &AssetInfo) -> StdResult<()> {
    if let AssetInfo::NativeToken { denom } = lp_token {
        // Shape check first — fails fast for obvious non-LP denoms before we
        // pay for a cross-contract query.
        let lp_minter = get_pair_from_denom(deps, denom)?;

        // Round-trip the alleged pair contract. Any query failure (wrong addr,
        // non-pair contract, etc.) means this denom is not a registered LP.
        let pair_info: PairInfo = deps
            .querier
            .query_wasm_smart(&lp_minter, &pair::QueryMsg::Pair {})
            .map_err(|_| {
                StdError::generic_err(format!(
                    "LP token {denom} is not minted by a registered Astroport pair at {lp_minter}",
                ))
            })?;

        if pair_info.liquidity_token != *denom {
            return Err(StdError::generic_err(format!(
                "LP token {denom} doesn't match LP token registered in pair {}",
                pair_info.liquidity_token
            )));
        }

        // Factory cross-check: the lp-minter must be the address the factory
        // has registered for this asset pair. Without this, an attacker can
        // deploy a fake-pair contract that forges the self-query response.
        let registered: PairInfo = deps
            .querier
            .query_wasm_smart(
                &config.factory,
                &factory::QueryMsg::Pair {
                    asset_infos: pair_info.asset_infos.to_vec(),
                },
            )
            .map_err(|_| {
                StdError::generic_err(format!(
                    "The pair is not registered in factory: {}-{}",
                    pair_info.asset_infos[0], pair_info.asset_infos[1]
                ))
            })?;

        if registered.contract_addr != lp_minter {
            return Err(StdError::generic_err(format!(
                "LP token {denom} claims pair {lp_minter} but factory registers {} for this asset pair",
                registered.contract_addr
            )));
        }

        if registered.liquidity_token != *denom {
            return Err(StdError::generic_err(format!(
                "LP token {denom} doesn't match LP token registered in factory {}",
                registered.liquidity_token
            )));
        }

        Ok(())
    } else {
        // Full check that cw20 LP token is registered in the factory
        let pair_info = query_pair_info(deps, lp_token)?;
        deps.querier
            .query_wasm_smart::<PairInfo>(
                &config.factory,
                &factory::QueryMsg::Pair {
                    asset_infos: pair_info.asset_infos.to_vec(),
                },
            )
            .map_err(|_| {
                StdError::generic_err(format!(
                    "The pair is not registered: {}-{}",
                    pair_info.asset_infos[0], pair_info.asset_infos[1]
                ))
            })
            .map(|resp| {
                if resp.liquidity_token == lp_token.to_string() {
                    Ok(())
                } else {
                    Err(StdError::generic_err(format!(
                        "LP token {lp_token} doesn't match LP token registered in factory {}",
                        resp.liquidity_token
                    )))
                }
            })?
    }
}

/// Claim orphaned rewards on behalf of the protocol.
///
/// Astroport-Juno hardening (audit finding MEDIUM): the upstream behaviour
/// accepted an owner-supplied `receiver: String` and forwarded the funds to
/// it, which is a clean backdoor — the owner could siphon orphaned rewards
/// (deposited in good faith by external incentivizers) into any address by
/// proposing a single governance call. We now hard-bind the receiver to
/// `config.owner`, i.e. the DAO core address, and surface the supplied value
/// in an event attribute only when it disagrees so the caller is not silently
/// surprised.
///
/// Funder-trust assumption: by sending external rewards into this contract,
/// incentivizers extend their trust to `config.owner` (the DAO) — the same
/// principal who can reconfigure emissions and deregister rewards. Orphan
/// recovery therefore terminates at the DAO's treasury, which the funder has
/// already accepted as the protocol's fiduciary endpoint. Re-routing orphans
/// elsewhere would break that trust model.
pub fn claim_orphaned_rewards(
    deps: DepsMut,
    info: MessageInfo,
    limit: Option<u8>,
    receiver: String,
) -> Result<Response, ContractError> {
    let config = CONFIG.load(deps.storage)?;
    ensure!(info.sender == config.owner, ContractError::Unauthorized {});

    // Ignore the supplied receiver and route to the DAO core address.
    // The argument is still validated for bech32 shape and echoed back as an
    // attribute so callers do not pass garbage by mistake; the value itself
    // does not influence the funds flow.
    let supplied_receiver = deps.api.addr_validate(&receiver)?;
    let receiver = config.owner.clone();

    let limit = limit
        .unwrap_or(MAX_ORPHANED_REWARD_LIMIT)
        .min(MAX_ORPHANED_REWARD_LIMIT);

    let orphaned_rewards = ORPHANED_REWARDS
        .range(deps.storage, None, None, Order::Ascending)
        .take(limit as usize)
        .collect::<StdResult<Vec<_>>>()?;

    if orphaned_rewards.is_empty() {
        return Err(ContractError::NoOrphanedRewards {});
    }

    let mut messages = vec![];
    let mut attrs = vec![
        attr("action", "claim_orphaned_rewards"),
        attr("receiver", &receiver),
        attr("supplied_receiver", &supplied_receiver),
    ];

    for (reward_info_binary, amount) in orphaned_rewards {
        // Send orphaned rewards
        if !amount.is_zero() {
            ORPHANED_REWARDS.remove(deps.storage, &reward_info_binary);

            let reward_info = from_key_to_asset_info(reward_info_binary)?;
            let reward_asset = reward_info.with_balance(amount);

            attrs.push(attr("claimed_orphaned_reward", reward_asset.to_string()));

            let reply_id =
                register_pending_transfer(deps.storage, &reward_asset.info, reward_asset.amount)?;
            let transfer_msg = reward_asset.into_submsg(
                &receiver,
                Some((ReplyOn::Always, reply_id)),
                config.token_transfer_gas_limit,
            )?;
            messages.push(transfer_msg);
        }
    }

    Ok(Response::new()
        .add_attributes(attrs)
        .add_submessages(messages))
}

pub fn asset_info_key(asset_info: &AssetInfo) -> Vec<u8> {
    let mut bytes = vec![];
    match asset_info {
        AssetInfo::NativeToken { denom } => {
            bytes.push(0);
            bytes.extend_from_slice(denom.as_bytes());
        }
        AssetInfo::Token { contract_addr } => {
            bytes.push(1);
            bytes.extend_from_slice(contract_addr.as_bytes());
        }
    }

    bytes
}

pub fn from_key_to_asset_info(bytes: Vec<u8>) -> StdResult<AssetInfo> {
    match bytes[0] {
        0 => String::from_utf8(bytes[1..].to_vec())
            .map_err(StdError::invalid_utf8)
            .map(AssetInfo::native),
        1 => String::from_utf8(bytes[1..].to_vec())
            .map_err(StdError::invalid_utf8)
            .map(AssetInfo::cw20_unchecked),
        _ => Err(StdError::generic_err(
            "Failed to deserialize asset info key",
        )),
    }
}

#[cfg(test)]
mod unit_tests {
    use astroport::asset::AssetInfo;
    use cosmwasm_std::testing::mock_dependencies;
    use cosmwasm_std::{Reply, SubMsgResponse, SubMsgResult};

    use crate::reply::{
        register_pending_transfer, reply, FIRST_DYNAMIC_REPLY_ID, NEXT_REPLY_ID,
        PENDING_REWARD_TRANSFERS, POST_TRANSFER_REPLY_ID,
    };

    use super::*;

    #[test]
    fn test_asset_info_binary_key() {
        let asset_infos = vec![
            AssetInfo::native("uusd"),
            AssetInfo::cw20_unchecked("wasm1contractxxx"),
        ];

        for asset_info in asset_infos {
            let key = asset_info_key(&asset_info);
            assert_eq!(from_key_to_asset_info(key).unwrap(), asset_info);
        }
    }

    #[test]
    fn test_deserialize_asset_info_from_malformed_data() {
        let asset_infos = vec![
            AssetInfo::native("uusd"),
            AssetInfo::cw20_unchecked("wasm1contractxxx"),
        ];

        for asset_info in asset_infos {
            let mut key = asset_info_key(&asset_info);
            key[0] = 2;

            assert_eq!(
                from_key_to_asset_info(key).unwrap_err(),
                StdError::generic_err("Failed to deserialize asset info key")
            );
        }

        let key = vec![0, u8::MAX];
        assert_eq!(
            from_key_to_asset_info(key).unwrap_err().to_string(),
            "Cannot decode UTF8 bytes into string: invalid utf-8 sequence of 1 bytes from index 0"
        );
    }

    // ---------------------------------------------------------------------
    // Astroport-Juno audit remediation tests
    // ---------------------------------------------------------------------

    /// `register_pending_transfer` should hand out monotonic ids starting from
    /// `FIRST_DYNAMIC_REPLY_ID`, persist the (asset, amount) payload, and bump
    /// the counter so the next caller never gets a colliding id.
    #[test]
    fn pending_transfer_registration_is_monotonic() {
        let mut deps = mock_dependencies();
        let reward_a = AssetInfo::native("ujuno");
        let reward_b = AssetInfo::cw20_unchecked("juno1lpcontractxxx");

        let id_1 = register_pending_transfer(deps.as_mut().storage, &reward_a, 100u128.into())
            .expect("first registration");
        let id_2 = register_pending_transfer(deps.as_mut().storage, &reward_b, 200u128.into())
            .expect("second registration");

        assert_eq!(id_1, FIRST_DYNAMIC_REPLY_ID);
        assert_eq!(id_2, FIRST_DYNAMIC_REPLY_ID + 1);
        assert_eq!(
            NEXT_REPLY_ID.load(deps.as_ref().storage).unwrap(),
            FIRST_DYNAMIC_REPLY_ID + 2
        );

        let (key_1, amt_1) = PENDING_REWARD_TRANSFERS
            .load(deps.as_ref().storage, id_1)
            .unwrap();
        assert_eq!(key_1, asset_info_key(&reward_a));
        assert_eq!(amt_1, Uint128::new(100));

        let (key_2, amt_2) = PENDING_REWARD_TRANSFERS
            .load(deps.as_ref().storage, id_2)
            .unwrap();
        assert_eq!(key_2, asset_info_key(&reward_b));
        assert_eq!(amt_2, Uint128::new(200));
    }

    /// Critical audit finding: when a reward transfer fails, the funds must
    /// land in `ORPHANED_REWARDS` so the DAO can recover them. The legacy
    /// `ReplyOn::Error` + global id handler silently dropped the failure.
    #[test]
    fn failed_transfer_routes_to_orphaned_rewards() {
        let mut deps = mock_dependencies();
        let reward = AssetInfo::native("ujuno");
        let amount = Uint128::new(1_234);

        let reply_id = register_pending_transfer(deps.as_mut().storage, &reward, amount).unwrap();

        let env = cosmwasm_std::testing::mock_env();
        let response = reply(
            deps.as_mut(),
            env,
            Reply {
                id: reply_id,
                result: SubMsgResult::Err("bank send: insufficient funds".to_string()),
            },
        )
        .expect("reply handler succeeds even on transfer failure");

        // Pending entry is garbage-collected.
        assert!(PENDING_REWARD_TRANSFERS
            .may_load(deps.as_ref().storage, reply_id)
            .unwrap()
            .is_none());

        // Failed amount is now claimable as an orphaned reward.
        let orphaned = ORPHANED_REWARDS
            .load(deps.as_ref().storage, &asset_info_key(&reward))
            .unwrap();
        assert_eq!(orphaned, amount);

        // Structured attributes are present for off-chain monitoring.
        let attrs: std::collections::HashMap<_, _> = response
            .attributes
            .iter()
            .map(|a| (a.key.as_str(), a.value.as_str()))
            .collect();
        assert_eq!(attrs.get("action"), Some(&"orphan_failed_transfer"));
        assert_eq!(attrs.get("reward"), Some(&"ujuno"));
        assert_eq!(attrs.get("amount"), Some(&"1234"));
        assert!(attrs.contains_key("transfer_error"));
    }

    /// Successful transfers must also clean up their pending entry — using
    /// `ReplyOn::Always` means we always get a chance to GC and storage does
    /// not leak across the contract's lifetime.
    #[test]
    fn successful_transfer_cleans_up_pending_entry() {
        let mut deps = mock_dependencies();
        let reward = AssetInfo::native("ujuno");
        let amount = Uint128::new(42);

        let reply_id = register_pending_transfer(deps.as_mut().storage, &reward, amount).unwrap();

        let env = cosmwasm_std::testing::mock_env();
        reply(
            deps.as_mut(),
            env,
            Reply {
                id: reply_id,
                result: SubMsgResult::Ok(SubMsgResponse {
                    events: vec![],
                    data: None,
                }),
            },
        )
        .unwrap();

        assert!(PENDING_REWARD_TRANSFERS
            .may_load(deps.as_ref().storage, reply_id)
            .unwrap()
            .is_none());
        // No orphan credit on success.
        assert!(ORPHANED_REWARDS
            .may_load(deps.as_ref().storage, &asset_info_key(&reward))
            .unwrap()
            .is_none());
    }

    /// Legacy `POST_TRANSFER_REPLY_ID` branch keeps working — we only attach
    /// the `transfer_error` attribute and do not panic. This preserves
    /// compatibility with any in-flight submessage from before the migration.
    #[test]
    fn legacy_post_transfer_reply_id_is_still_accepted() {
        let mut deps = mock_dependencies();
        let env = cosmwasm_std::testing::mock_env();

        let response = reply(
            deps.as_mut(),
            env,
            Reply {
                id: POST_TRANSFER_REPLY_ID,
                result: SubMsgResult::Err("legacy".to_string()),
            },
        )
        .expect("legacy id still accepted");

        let attr = response
            .attributes
            .iter()
            .find(|a| a.key == "transfer_error")
            .expect("transfer_error attribute set");
        assert_eq!(attr.value, "legacy");
    }

    /// `ActivePoolInvariantBroken` should render a useful message identifying
    /// the desynced lp token so on-chain attribution can recover.
    #[test]
    fn active_pool_invariant_broken_error_message() {
        let err = ContractError::ActivePoolInvariantBroken {
            lp_token: "factory/juno1xxx/astroport/share".to_string(),
        };
        let msg = err.to_string();
        assert!(msg.contains("factory/juno1xxx/astroport/share"));
        assert!(msg.contains("ACTIVE_POOLS"));
    }

    /// `deactivate_pool` must return the typed `ActivePoolInvariantBroken`
    /// error (rather than panicking via `.unwrap()`) when `PoolInfo` claims
    /// to be active but the corresponding `ACTIVE_POOLS` entry is missing.
    /// Triggers the invariant break through the real production call site,
    /// not just the Display impl — closes the rc4 R5 finding that the prior
    /// test only guarded the error message format.
    #[test]
    fn deactivate_pool_returns_typed_error_when_invariant_broken() {
        use crate::state::PoolInfo;
        use astroport::asset::AssetInfo;
        use astroport::incentives::{Config, RewardInfo, RewardType};
        use cosmwasm_std::testing::{mock_env, mock_info};
        use cosmwasm_std::{Decimal256, Uint128};

        let mut deps = mock_dependencies();
        let env = mock_env();
        let factory = deps.api.addr_make("factory");
        let owner = deps.api.addr_make("owner");
        let reward_token = AssetInfo::native("ujuno");
        let lp_token = "factory/juno1lpxxx/astroport/share".to_string();
        let lp_asset = AssetInfo::native(&lp_token);

        // Seed a Config whose factory matches the simulated caller.
        CONFIG
            .save(
                deps.as_mut().storage,
                &Config {
                    owner,
                    factory: factory.clone(),
                    generator_controller: None,
                    reward_token: reward_token.clone(),
                    reward_per_second: Uint128::new(1000),
                    total_alloc_points: Uint128::new(100),
                    guardian: None,
                    incentivization_fee_info: None,
                    token_transfer_gas_limit: None,
                },
            )
            .unwrap();

        // Seed an "active" PoolInfo — non-zero internal rps so
        // `is_active_pool()` returns true and the deactivate_pool match
        // arm enters the `Some(_) if pool_info.is_active_pool()` branch.
        let mut pool_info = PoolInfo {
            total_lp: Uint128::zero(),
            rewards: vec![RewardInfo {
                reward: RewardType::Int(reward_token.clone()),
                rps: Decimal256::from_ratio(1u128, 1u128),
                index: Decimal256::zero(),
                orphaned: Decimal256::zero(),
            }],
            last_update_ts: env.block.time.seconds(),
            rewards_to_remove: Default::default(),
        };
        pool_info.save(deps.as_mut().storage, &lp_asset).unwrap();

        // Seed ACTIVE_POOLS with the desync invariant breach — empty list,
        // even though PoolInfo above claims to be active.
        ACTIVE_POOLS.save(deps.as_mut().storage, &vec![]).unwrap();

        // Call the production deactivate_pool from the factory. Pre-rc3
        // this would panic via `.unwrap()`; rc3 made it return the typed
        // error; this test pins that contract so a future refactor can't
        // silently re-introduce the unwrap.
        let info = mock_info(factory.as_str(), &[]);
        let err = deactivate_pool(deps.as_mut(), info, env, lp_token.clone())
            .expect_err("desynced state must surface the typed error, not panic");

        let lp_token_for_match = lp_token.clone();
        assert!(
            matches!(err, ContractError::ActivePoolInvariantBroken { lp_token: ref t } if t == &lp_token_for_match),
            "expected typed ActivePoolInvariantBroken, got: {err:?}"
        );
    }
}
