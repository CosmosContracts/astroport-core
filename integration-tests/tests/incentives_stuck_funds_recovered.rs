//! **AUDIT REGRESSION GATE** for the CRITICAL stuck-funds fix.
//!
//! Upstream Astroport dispatched every reward transfer with `ReplyOn::Error`
//! and a single global reply id (`POST_TRANSFER_REPLY_ID`), then logged the
//! error and returned `Response::default()` — silently swallowing any failed
//! transfer. The user's `last_rewards_index` was already advanced inside
//! `claim_rewards` before the submessage was emitted, so a transfer failure
//! permanently destroyed the user's claim: the funds stayed in the contract
//! and the user had no further claim path. That was finding CRIT-1 in the
//! rc2 audit.
//!
//! Astroport-Juno's A1 fix:
//!
//! - Each transfer reserves a fresh reply id via `register_pending_transfer`,
//!   which writes `(asset_info_key, amount)` into `PENDING_REWARD_TRANSFERS`
//!   and uses `ReplyOn::Always`.
//! - The reply handler either GCs the pending entry on success, or — on
//!   failure — credits the failed amount into `ORPHANED_REWARDS` keyed by
//!   reward asset, so the DAO can recover via `ClaimOrphanedRewards`.
//!
//! This integration test forces a real on-chain transfer failure and
//! exercises the full recovery path:
//!
//! 1. Deploy the keep-set + incentives.
//! 2. Deploy a "controllable cw20" mock whose `Transfer` execute can be
//!    flipped between success and unconditional failure via a custom
//!    `SetFailTransfer { fail: bool }` admin message. `TransferFrom`
//!    always succeeds, so the funder's `Incentivize` call can pull
//!    reward tokens into the contract in the normal way.
//! 3. Funder incentivizes a pool with the controllable-cw20 as the reward.
//! 4. Alice stakes; time advances.
//! 5. Flip the cw20 into failure mode; Alice calls `ClaimRewards`.
//!    The outer tx must succeed (reply handler swallows the error), the
//!    user's pending rewards must drop to zero (the position advanced),
//!    and the failed amount must land in `ORPHANED_REWARDS`. The contract's
//!    cw20 balance must be unchanged because the `Transfer` reverted.
//! 6. Un-flip the cw20; DAO calls `ClaimOrphanedRewards`. Funds land in
//!    the DAO's (= owner's) cw20 balance — full recovery.

use cosmwasm_schema::{cw_serde, QueryResponses};
use cosmwasm_std::{
    coin, to_json_binary, Addr, Binary, Deps, DepsMut, Env, MessageInfo, Response, StdError,
    StdResult, Timestamp, Uint128,
};
use cw_storage_plus::Item;

use astroport::asset::{Asset, AssetInfo, PairInfo};
use astroport::factory::{ExecuteMsg as FactoryExecuteMsg, PairType, QueryMsg as FactoryQueryMsg};
use astroport::incentives::{
    ExecuteMsg as IncentivesExecuteMsg, InputSchedule, QueryMsg as IncentivesQueryMsg, EPOCHS_START,
};
use astroport::pair::ExecuteMsg as PairExecuteMsg;
use astroport_test::cw_multi_test::{ContractWrapper, Executor};

use astroport_juno_integration_tests::{
    balance_of, deploy_incentives_addon, deploy_keep_set, fund, mock_app, KeepSetHandles, TestApp,
    MOCK_USDC, UJUNO,
};

const ALICE: &str = "alice";
const FUNDER: &str = "funder";

const LP_SEED: u128 = 100_000_000_000;
/// Schedule reward amount in cw20 base units. Must be large enough that
/// `reward.amount / (end_ts - block_ts) >= 1`.
const REWARD_AMOUNT: u128 = 100_000_000;

#[test]
fn failed_reward_transfer_lands_in_orphaned_rewards_and_is_recoverable() {
    let mut app = mock_app();
    app.update_block(|b| {
        b.time = Timestamp::from_seconds(EPOCHS_START + 86400);
        b.height += 1;
    });

    let handles = deploy_keep_set(&mut app).unwrap();
    let inc = deploy_incentives_addon(
        &mut app,
        &handles,
        AssetInfo::NativeToken {
            denom: UJUNO.to_string(),
        },
        None,
    )
    .unwrap();

    // ---------------------------------------------------------------
    // 1. Deploy the controllable cw20 reward token.
    // ---------------------------------------------------------------
    let mock_cw20_code_id = app.store_code(Box::new(ContractWrapper::new_with_empty(
        mock_cw20::execute,
        mock_cw20::instantiate,
        mock_cw20::query,
    )));
    let funder = app.api().addr_make(FUNDER);
    let cw20_reward: Addr = app
        .instantiate_contract(
            mock_cw20_code_id,
            handles.deployer.clone(),
            &mock_cw20::InstantiateMsg {
                initial_balances: vec![(funder.to_string(), Uint128::new(REWARD_AMOUNT))],
            },
            &[],
            "controllable-cw20",
            None,
        )
        .unwrap();

    // ---------------------------------------------------------------
    // 2. Stand up a pool + stake Alice's LP.
    // ---------------------------------------------------------------
    let pair = create_pair(&mut app, &handles, UJUNO, MOCK_USDC);
    let lp_denom = lp_denom_of(&mut app, &handles, UJUNO, MOCK_USDC);

    let alice = app.api().addr_make(ALICE);
    fund(
        &mut app,
        &alice,
        vec![coin(LP_SEED, UJUNO), coin(LP_SEED, MOCK_USDC)],
    )
    .unwrap();
    provide_liquidity(&mut app, &pair, &alice, LP_SEED, LP_SEED);
    let alice_lp = balance_of(&app, &alice, &lp_denom);
    app.execute_contract(
        alice.clone(),
        inc.incentives.clone(),
        &IncentivesExecuteMsg::Deposit { recipient: None },
        &[coin(alice_lp.u128(), lp_denom.clone())],
    )
    .unwrap();

    // ---------------------------------------------------------------
    // 3. Funder allows + incentivizes pool with the controllable cw20.
    //    `TransferFrom` is not gated by the fail flag, so this works.
    // ---------------------------------------------------------------
    app.execute_contract(
        funder.clone(),
        cw20_reward.clone(),
        &mock_cw20::ExecuteMsg::IncreaseAllowance {
            spender: inc.incentives.to_string(),
            amount: Uint128::new(REWARD_AMOUNT),
        },
        &[],
    )
    .expect("IncreaseAllowance succeeds");

    app.execute_contract(
        funder.clone(),
        inc.incentives.clone(),
        &IncentivesExecuteMsg::Incentivize {
            lp_token: lp_denom.clone(),
            schedule: InputSchedule {
                reward: Asset {
                    info: AssetInfo::Token {
                        contract_addr: cw20_reward.clone(),
                    },
                    amount: Uint128::new(REWARD_AMOUNT),
                },
                duration_periods: 1,
            },
        },
        &[],
    )
    .expect("Incentivize pulls cw20 reward via TransferFrom");

    let inc_cw20_after_fund = query_cw20_balance(&app, &cw20_reward, &inc.incentives);
    assert_eq!(
        inc_cw20_after_fund,
        Uint128::new(REWARD_AMOUNT),
        "contract took custody of the reward tokens"
    );

    // ---------------------------------------------------------------
    // 4. Advance into the active schedule window.
    // ---------------------------------------------------------------
    app.update_block(|b| {
        b.time = b.time.plus_seconds(7 * 86400);
        b.height += 1;
    });

    // Sanity: Alice has accrued some pending reward in the cw20.
    let pending_before: Vec<Asset> = app
        .wrap()
        .query_wasm_smart(
            inc.incentives.clone(),
            &IncentivesQueryMsg::PendingRewards {
                lp_token: lp_denom.clone(),
                user: alice.to_string(),
            },
        )
        .unwrap();
    let pending_reward_amount = pending_before
        .iter()
        .find(|a| {
            matches!(&a.info, AssetInfo::Token { contract_addr } if contract_addr == &cw20_reward)
        })
        .map(|a| a.amount)
        .expect("alice has pending cw20 reward before claim");
    assert!(
        !pending_reward_amount.is_zero(),
        "non-zero cw20 reward should be pending before claim"
    );

    // ---------------------------------------------------------------
    // 5. Flip the cw20 into failure mode and call ClaimRewards.
    //    The reply handler must intercept the failure and route the
    //    amount to ORPHANED_REWARDS. The outer tx must succeed.
    // ---------------------------------------------------------------
    app.execute_contract(
        handles.deployer.clone(),
        cw20_reward.clone(),
        &mock_cw20::ExecuteMsg::SetFailTransfer { fail: true },
        &[],
    )
    .unwrap();

    let alice_cw20_before_claim = query_cw20_balance(&app, &cw20_reward, &alice);
    let inc_cw20_before_claim = query_cw20_balance(&app, &cw20_reward, &inc.incentives);

    let claim_res = app
        .execute_contract(
            alice.clone(),
            inc.incentives.clone(),
            &IncentivesExecuteMsg::ClaimRewards {
                lp_tokens: vec![lp_denom.clone()],
            },
            &[],
        )
        .expect("ClaimRewards outer tx must succeed even when the inner transfer reverts");

    // The reply handler emitted the `orphan_failed_transfer` action attribute.
    let saw_orphan_event = claim_res.events.iter().any(|ev| {
        ev.attributes
            .iter()
            .any(|a| a.key == "action" && a.value == "orphan_failed_transfer")
    });
    assert!(
        saw_orphan_event,
        "reply handler must emit orphan_failed_transfer attribute on failed transfer; \
         got events: {:?}",
        claim_res.events
    );

    // The contract's cw20 balance is unchanged — the cw20 Transfer reverted,
    // so the tokens never left.
    let inc_cw20_after_claim = query_cw20_balance(&app, &cw20_reward, &inc.incentives);
    assert_eq!(
        inc_cw20_after_claim, inc_cw20_before_claim,
        "failed cw20::Transfer must not have moved any tokens out of the incentives contract"
    );
    // Alice's balance is also unchanged for the same reason.
    let alice_cw20_after_claim = query_cw20_balance(&app, &cw20_reward, &alice);
    assert_eq!(
        alice_cw20_after_claim, alice_cw20_before_claim,
        "failed cw20::Transfer must not credit the user's balance"
    );

    // Alice's position advanced: pending rewards for the cw20 are now zero
    // (the claim consumed her unclaimed index even though the transfer failed —
    // the recovery path is via ORPHANED_REWARDS, not via re-claiming).
    let pending_after: Vec<Asset> = app
        .wrap()
        .query_wasm_smart(
            inc.incentives.clone(),
            &IncentivesQueryMsg::PendingRewards {
                lp_token: lp_denom.clone(),
                user: alice.to_string(),
            },
        )
        .unwrap();
    let pending_cw20_after = pending_after
        .iter()
        .find(|a| {
            matches!(&a.info, AssetInfo::Token { contract_addr } if contract_addr == &cw20_reward)
        })
        .map(|a| a.amount)
        .unwrap_or_default();
    assert!(
        pending_cw20_after < pending_reward_amount,
        "user position must have advanced past the failed claim — pending dropped from {} to {}",
        pending_reward_amount,
        pending_cw20_after
    );

    // ---------------------------------------------------------------
    // 6. Recovery: un-fail the cw20 and sweep orphaned rewards.
    //    ClaimOrphanedRewards is owner-gated and hard-binds receiver to
    //    config.owner (the deployer in this harness).
    // ---------------------------------------------------------------
    app.execute_contract(
        handles.deployer.clone(),
        cw20_reward.clone(),
        &mock_cw20::ExecuteMsg::SetFailTransfer { fail: false },
        &[],
    )
    .unwrap();

    let owner_cw20_before_sweep = query_cw20_balance(&app, &cw20_reward, &handles.deployer);

    app.execute_contract(
        handles.deployer.clone(),
        inc.incentives.clone(),
        &IncentivesExecuteMsg::ClaimOrphanedRewards {
            limit: None,
            // Argument is validated for bech32 shape but ignored for routing;
            // hard-binding to config.owner is the Astroport-Juno hardening of
            // the upstream owner-set-receiver backdoor (audit MEDIUM finding).
            receiver: handles.deployer.to_string(),
        },
        &[],
    )
    .expect("ClaimOrphanedRewards must succeed once the cw20 Transfer is unblocked");

    let owner_cw20_after_sweep = query_cw20_balance(&app, &cw20_reward, &handles.deployer);
    let recovered = owner_cw20_after_sweep - owner_cw20_before_sweep;
    assert!(
        recovered > Uint128::zero(),
        "DAO (config.owner) must receive the orphaned reward amount via the sweep"
    );

    // A second sweep call with nothing left orphaned must error — confirms
    // the recovery actually drained ORPHANED_REWARDS.
    let err = app
        .execute_contract(
            handles.deployer.clone(),
            inc.incentives.clone(),
            &IncentivesExecuteMsg::ClaimOrphanedRewards {
                limit: None,
                receiver: handles.deployer.to_string(),
            },
            &[],
        )
        .expect_err("ORPHANED_REWARDS must be drained after a successful sweep");
    assert!(
        err.root_cause().to_string().contains("orphaned"),
        "expected NoOrphanedRewards-style error, got: {}",
        err.root_cause()
    );
}

// =====================================================================
// helpers (local to this test target — copied from neighbouring tests)
// =====================================================================

fn create_pair(app: &mut TestApp, handles: &KeepSetHandles, denom_a: &str, denom_b: &str) -> Addr {
    let asset_infos = vec![
        AssetInfo::NativeToken {
            denom: denom_a.to_string(),
        },
        AssetInfo::NativeToken {
            denom: denom_b.to_string(),
        },
    ];
    app.execute_contract(
        handles.deployer.clone(),
        handles.factory.clone(),
        &FactoryExecuteMsg::CreatePair {
            pair_type: PairType::Xyk {},
            asset_infos: asset_infos.clone(),
            init_params: None,
        },
        &[],
    )
    .unwrap();
    let info: PairInfo = app
        .wrap()
        .query_wasm_smart(
            handles.factory.clone(),
            &FactoryQueryMsg::Pair { asset_infos },
        )
        .unwrap();
    info.contract_addr
}

fn lp_denom_of(
    app: &mut TestApp,
    handles: &KeepSetHandles,
    denom_a: &str,
    denom_b: &str,
) -> String {
    let info: PairInfo = app
        .wrap()
        .query_wasm_smart(
            handles.factory.clone(),
            &FactoryQueryMsg::Pair {
                asset_infos: vec![
                    AssetInfo::NativeToken {
                        denom: denom_a.to_string(),
                    },
                    AssetInfo::NativeToken {
                        denom: denom_b.to_string(),
                    },
                ],
            },
        )
        .unwrap();
    info.liquidity_token
}

fn provide_liquidity(
    app: &mut TestApp,
    pair: &Addr,
    sender: &Addr,
    a_amount: u128,
    b_amount: u128,
) {
    let assets = vec![
        Asset {
            info: AssetInfo::NativeToken {
                denom: UJUNO.to_string(),
            },
            amount: Uint128::new(a_amount),
        },
        Asset {
            info: AssetInfo::NativeToken {
                denom: MOCK_USDC.to_string(),
            },
            amount: Uint128::new(b_amount),
        },
    ];
    let mut funds = vec![coin(a_amount, UJUNO), coin(b_amount, MOCK_USDC)];
    funds.sort_by(|a, b| a.denom.cmp(&b.denom));
    app.execute_contract(
        sender.clone(),
        pair.clone(),
        &PairExecuteMsg::ProvideLiquidity {
            assets,
            slippage_tolerance: None,
            auto_stake: None,
            receiver: None,
            min_lp_to_receive: None,
        },
        &funds,
    )
    .unwrap();
}

fn query_cw20_balance(app: &TestApp, cw20: &Addr, who: &Addr) -> Uint128 {
    let resp: mock_cw20::BalanceResponse = app
        .wrap()
        .query_wasm_smart(
            cw20,
            &mock_cw20::QueryMsg::Balance {
                address: who.to_string(),
            },
        )
        .unwrap();
    resp.balance
}

// =====================================================================
// mock_cw20 — controllable failure-mode cw20 implementation.
//
// Implements only the subset of cw20 that astroport-incentives touches:
// `TransferFrom` (used by Incentivize to pull funder rewards) and
// `Transfer` (used by claim_rewards and claim_orphaned_rewards to push
// rewards out). `IncreaseAllowance` is required so the funder can grant
// the spender allowance before Incentivize. The custom `SetFailTransfer`
// message flips the Transfer behaviour without restarting the app —
// allowing the test to:
//
// 1. fund the contract (Transfer succeeds via TransferFrom),
// 2. fail the claim (Transfer rejects),
// 3. recover via ClaimOrphanedRewards (Transfer succeeds again).
//
// We could not reuse `cw20-base` for this because it has no failure
// switch; sub-classing via a wrapper would still need an extra ExecuteMsg
// variant, at which point the message enum is no longer cw20::ExecuteMsg.
// =====================================================================

mod mock_cw20 {
    use super::*;

    const STATE: Item<State> = Item::new("mock_cw20_state");

    #[cw_serde]
    pub struct State {
        pub balances: Vec<(String, Uint128)>,
        /// `(owner, spender) -> allowance`. Encoded as a sorted vec so it
        /// can round-trip through `cw_serde`-style storage without needing
        /// to pull in cw-storage-plus' `Map` for a two-key composite.
        pub allowances: Vec<((String, String), Uint128)>,
        pub fail_transfer: bool,
    }

    #[cw_serde]
    pub struct InstantiateMsg {
        pub initial_balances: Vec<(String, Uint128)>,
    }

    #[cw_serde]
    pub enum ExecuteMsg {
        // cw20 subset we actually exercise.
        Transfer {
            recipient: String,
            amount: Uint128,
        },
        TransferFrom {
            owner: String,
            recipient: String,
            amount: Uint128,
        },
        IncreaseAllowance {
            spender: String,
            amount: Uint128,
        },
        // Test-only switch. Anyone can flip this — there is no auth gate
        // because this contract only exists inside the test harness.
        SetFailTransfer {
            fail: bool,
        },
    }

    #[cw_serde]
    #[derive(QueryResponses)]
    pub enum QueryMsg {
        #[returns(BalanceResponse)]
        Balance { address: String },
    }

    #[cw_serde]
    pub struct BalanceResponse {
        pub balance: Uint128,
    }

    pub fn instantiate(
        deps: DepsMut,
        _env: Env,
        _info: MessageInfo,
        msg: InstantiateMsg,
    ) -> StdResult<Response> {
        STATE.save(
            deps.storage,
            &State {
                balances: msg.initial_balances,
                allowances: vec![],
                fail_transfer: false,
            },
        )?;
        Ok(Response::new())
    }

    pub fn execute(
        deps: DepsMut,
        _env: Env,
        info: MessageInfo,
        msg: ExecuteMsg,
    ) -> StdResult<Response> {
        let mut state = STATE.load(deps.storage)?;
        match msg {
            ExecuteMsg::Transfer { recipient, amount } => {
                if state.fail_transfer {
                    return Err(StdError::generic_err(
                        "mock_cw20: Transfer rejected in failure mode",
                    ));
                }
                transfer_inner(&mut state, info.sender.as_str(), &recipient, amount)?;
                STATE.save(deps.storage, &state)?;
                Ok(Response::new().add_attribute("action", "transfer"))
            }
            ExecuteMsg::TransferFrom {
                owner,
                recipient,
                amount,
            } => {
                // Spend allowance, then move the funds.
                spend_allowance(&mut state, &owner, info.sender.as_str(), amount)?;
                transfer_inner(&mut state, &owner, &recipient, amount)?;
                STATE.save(deps.storage, &state)?;
                Ok(Response::new().add_attribute("action", "transfer_from"))
            }
            ExecuteMsg::IncreaseAllowance { spender, amount } => {
                let key = (info.sender.to_string(), spender);
                let entry = state.allowances.iter_mut().find(|(k, _)| k == &key);
                match entry {
                    Some((_, existing)) => {
                        *existing = existing.checked_add(amount).map_err(StdError::overflow)?;
                    }
                    None => state.allowances.push((key, amount)),
                }
                STATE.save(deps.storage, &state)?;
                Ok(Response::new().add_attribute("action", "increase_allowance"))
            }
            ExecuteMsg::SetFailTransfer { fail } => {
                state.fail_transfer = fail;
                STATE.save(deps.storage, &state)?;
                Ok(Response::new()
                    .add_attribute("action", "set_fail_transfer")
                    .add_attribute("fail", fail.to_string()))
            }
        }
    }

    pub fn query(deps: Deps, _env: Env, msg: QueryMsg) -> StdResult<Binary> {
        match msg {
            QueryMsg::Balance { address } => {
                let state = STATE.load(deps.storage)?;
                let balance = state
                    .balances
                    .iter()
                    .find(|(addr, _)| addr == &address)
                    .map(|(_, b)| *b)
                    .unwrap_or_default();
                to_json_binary(&BalanceResponse { balance })
            }
        }
    }

    fn transfer_inner(
        state: &mut State,
        from: &str,
        to: &str,
        amount: Uint128,
    ) -> StdResult<()> {
        // Subtract from sender.
        let from_balance = state
            .balances
            .iter_mut()
            .find(|(addr, _)| addr == from)
            .ok_or_else(|| StdError::generic_err(format!("mock_cw20: no balance for {}", from)))?;
        from_balance.1 = from_balance
            .1
            .checked_sub(amount)
            .map_err(StdError::overflow)?;
        // Add to recipient.
        match state.balances.iter_mut().find(|(addr, _)| addr == to) {
            Some((_, b)) => *b = b.checked_add(amount).map_err(StdError::overflow)?,
            None => state.balances.push((to.to_string(), amount)),
        }
        Ok(())
    }

    fn spend_allowance(
        state: &mut State,
        owner: &str,
        spender: &str,
        amount: Uint128,
    ) -> StdResult<()> {
        let key = (owner.to_string(), spender.to_string());
        let allowance = state
            .allowances
            .iter_mut()
            .find(|(k, _)| k == &key)
            .ok_or_else(|| {
                StdError::generic_err(format!(
                    "mock_cw20: no allowance from {} to {}",
                    owner, spender
                ))
            })?;
        allowance.1 = allowance
            .1
            .checked_sub(amount)
            .map_err(StdError::overflow)?;
        Ok(())
    }

}
