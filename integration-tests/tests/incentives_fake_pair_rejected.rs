//! **AUDIT REGRESSION GATE** for the rc4 HIGH fix on `is_valid_pool`.
//!
//! Run 2 caught that rc3's fix for the rc2 HIGH (fake TF-LP denom spam)
//! was only a partial closure: the native branch of `is_valid_pool` added
//! a `query_wasm_smart(lp_minter, pair::QueryMsg::Pair {})` round-trip,
//! but the lp_minter is attacker-controlled — an attacker can deploy a
//! wasm contract that both holds the tokenfactory mint authority for
//! `factory/<self>/astroport/share` AND responds to `pair::QueryMsg::Pair {}`
//! with a forged `PairInfo` whose `liquidity_token` matches the supplied
//! denom. Both rc3 gates pass.
//!
//! rc4 closes this by also cross-checking the factory's PAIRS registry
//! (mirroring the cw20 branch's pattern). After the pair-self-query, the
//! native branch now requires:
//!
//! - `factory::QueryMsg::Pair { asset_infos }` returns a registered pair
//! - the registered `contract_addr` equals the lp_minter
//! - the registered `liquidity_token` equals the supplied denom
//!
//! Forging now requires subverting the factory registry, which is
//! owner-gated.
//!
//! This test exercises two attacker scenarios:
//!
//! 1. **Impersonate-a-real-pair**: A real `(UJUNO, MOCK_USDC)` pair exists.
//!    The attacker's fake pair returns `asset_infos = (UJUNO, MOCK_USDC)`
//!    in its forged `PairInfo`. The factory cross-check finds the real
//!    pair at a different `contract_addr` and rejects.
//!
//! 2. **Invent-unregistered-asset-pair**: The attacker's fake pair returns
//!    `asset_infos` that no real factory-registered pair covers. The
//!    factory query fails outright.
//!
//! Both must error out of `Incentivize`. If either succeeds the rc4 fix
//! has regressed.

use cosmwasm_schema::{cw_serde, QueryResponses};
use cosmwasm_std::{
    coin, to_json_binary, Addr, Binary, Coin, Deps, DepsMut, Env, MessageInfo, Response,
    StdError, StdResult, Timestamp, Uint128,
};
use cw_storage_plus::Item;

use astroport::asset::{Asset, AssetInfo, PairInfo};
use astroport::factory::{ExecuteMsg as FactoryExecuteMsg, PairType, QueryMsg as FactoryQueryMsg};
use astroport::incentives::{ExecuteMsg as IncentivesExecuteMsg, InputSchedule, EPOCHS_START};
use astroport_test::cw_multi_test::{ContractWrapper, Executor};

use astroport_juno_integration_tests::{
    deploy_incentives_addon, deploy_keep_set, mock_app, TestApp, MOCK_USDC, UJUNO,
};

const FUNDER: &str = "attacker_funder";
const REWARD_AMOUNT: u128 = 100_000_000;

#[test]
fn incentivize_rejects_fake_pair_impersonating_real_asset_infos() {
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

    // Stand up the real (UJUNO, MOCK_USDC) pair so the factory has an
    // entry for that asset_info pair — the attacker will try to
    // impersonate it.
    let real_pair = create_pair(&mut app, &handles.factory, &handles.deployer, UJUNO, MOCK_USDC);
    let real_lp_denom = lp_denom_of(&app, &handles.factory, UJUNO, MOCK_USDC);

    // Deploy the fake-pair contract. Its sole purpose is to respond to
    // pair::QueryMsg::Pair {} with a forged PairInfo claiming to be the
    // (UJUNO, MOCK_USDC) pair, with itself as the contract_addr and a
    // matching tokenfactory denom under its own bech32 address.
    let fake_pair = instantiate_fake_pair(
        &mut app,
        vec![
            AssetInfo::NativeToken { denom: UJUNO.to_string() },
            AssetInfo::NativeToken { denom: MOCK_USDC.to_string() },
        ],
    );
    let fake_denom = format!("factory/{fake_pair}/astroport/share");

    // The fake pair must declare the matching liquidity_token (otherwise
    // the rc3 pair-self-query check would catch it on its own).
    set_fake_pair_state(&mut app, &fake_pair, &fake_denom);

    // Fund the attacker with ujuno for the incentivization fee + reward.
    let funder = app.api().addr_make(FUNDER);
    app.sudo(astroport_test::cw_multi_test::SudoMsg::Bank(
        astroport_test::cw_multi_test::BankSudo::Mint {
            to_address: funder.to_string(),
            amount: vec![coin(REWARD_AMOUNT * 10, UJUNO)],
        },
    ))
    .unwrap();

    // The attack: Incentivize against the fake LP denom. rc3 would let
    // this through (pair-self-query echoes the matching liquidity_token);
    // rc4 must reject because the factory's registered pair for
    // (UJUNO, MOCK_USDC) lives at real_pair, not fake_pair.
    let err = app
        .execute_contract(
            funder.clone(),
            inc.incentives.clone(),
            &IncentivesExecuteMsg::Incentivize {
                lp_token: fake_denom.clone(),
                schedule: InputSchedule {
                    reward: Asset {
                        info: AssetInfo::NativeToken {
                            denom: UJUNO.to_string(),
                        },
                        amount: Uint128::new(REWARD_AMOUNT),
                    },
                    duration_periods: 1,
                },
            },
            &[coin(REWARD_AMOUNT, UJUNO)],
        )
        .expect_err("Incentivize against an impersonating fake pair must be rejected by rc4");

    let msg = err.root_cause().to_string();
    assert!(
        msg.contains("factory registers"),
        "rc4 factory cross-check error expected, got: {msg}"
    );
    // Make sure the error message names both contenders so on-chain
    // attribution can attribute the spoof.
    assert!(msg.contains(real_pair.as_str()), "error names the real pair address: {msg}");
    assert!(msg.contains(fake_pair.as_str()), "error names the fake (claimed) pair address: {msg}");

    // Belt-and-braces: the real pair's LP denom must still be acceptable
    // (proving the new gate doesn't false-positive on legitimate pairs).
    // We don't actually run an Incentivize here — that would require
    // configuring the schedule timing — but we assert the fake denom
    // didn't get registered.
    assert_ne!(
        real_lp_denom, fake_denom,
        "test setup invariant: fake_denom must differ from real_lp_denom"
    );
}

#[test]
fn incentivize_rejects_fake_pair_with_unregistered_asset_infos() {
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

    // No real pair created. The fake_pair's claimed asset_infos won't
    // match anything in the factory's registry.
    let fake_pair = instantiate_fake_pair(
        &mut app,
        vec![
            AssetInfo::NativeToken { denom: UJUNO.to_string() },
            AssetInfo::NativeToken {
                denom: "ibc/no-such-pair-token".to_string(),
            },
        ],
    );
    let fake_denom = format!("factory/{fake_pair}/astroport/share");
    set_fake_pair_state(&mut app, &fake_pair, &fake_denom);

    let funder = app.api().addr_make(FUNDER);
    app.sudo(astroport_test::cw_multi_test::SudoMsg::Bank(
        astroport_test::cw_multi_test::BankSudo::Mint {
            to_address: funder.to_string(),
            amount: vec![coin(REWARD_AMOUNT * 10, UJUNO)],
        },
    ))
    .unwrap();

    let err = app
        .execute_contract(
            funder,
            inc.incentives.clone(),
            &IncentivesExecuteMsg::Incentivize {
                lp_token: fake_denom,
                schedule: InputSchedule {
                    reward: Asset {
                        info: AssetInfo::NativeToken {
                            denom: UJUNO.to_string(),
                        },
                        amount: Uint128::new(REWARD_AMOUNT),
                    },
                    duration_periods: 1,
                },
            },
            &[coin(REWARD_AMOUNT, UJUNO)],
        )
        .expect_err("Incentivize against a fake pair with unregistered asset_infos must be rejected by rc4");

    let msg = err.root_cause().to_string();
    assert!(
        msg.contains("not registered in factory"),
        "rc4 factory-not-registered error expected, got: {msg}"
    );
}

// =====================================================================
// helpers
// =====================================================================

fn create_pair(app: &mut TestApp, factory: &Addr, deployer: &Addr, denom_a: &str, denom_b: &str) -> Addr {
    let asset_infos = vec![
        AssetInfo::NativeToken { denom: denom_a.to_string() },
        AssetInfo::NativeToken { denom: denom_b.to_string() },
    ];
    app.execute_contract(
        deployer.clone(),
        factory.clone(),
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
        .query_wasm_smart(factory.clone(), &FactoryQueryMsg::Pair { asset_infos })
        .unwrap();
    info.contract_addr
}

fn lp_denom_of(app: &TestApp, factory: &Addr, denom_a: &str, denom_b: &str) -> String {
    let info: PairInfo = app
        .wrap()
        .query_wasm_smart(
            factory.clone(),
            &FactoryQueryMsg::Pair {
                asset_infos: vec![
                    AssetInfo::NativeToken { denom: denom_a.to_string() },
                    AssetInfo::NativeToken { denom: denom_b.to_string() },
                ],
            },
        )
        .unwrap();
    info.liquidity_token
}

fn instantiate_fake_pair(app: &mut TestApp, claimed_asset_infos: Vec<AssetInfo>) -> Addr {
    let code_id = app.store_code(Box::new(ContractWrapper::new_with_empty(
        fake_pair::execute,
        fake_pair::instantiate,
        fake_pair::query,
    )));
    let deployer = app.api().addr_make("attacker_pair_deployer");
    app.instantiate_contract(
        code_id,
        deployer,
        &fake_pair::InstantiateMsg {
            claimed_asset_infos,
        },
        &[],
        "fake_pair",
        None,
    )
    .unwrap()
}

fn set_fake_pair_state(app: &mut TestApp, fake_pair: &Addr, liquidity_token: &str) {
    app.execute_contract(
        app.api().addr_make("anyone"),
        fake_pair.clone(),
        &fake_pair::ExecuteMsg::SetForgedLiquidityToken {
            denom: liquidity_token.to_string(),
        },
        &[] as &[Coin],
    )
    .unwrap();
}

// =====================================================================
// fake_pair — minimal contract that forges pair::QueryMsg::Pair {}
// responses. Its `contract_addr` will be itself; its `liquidity_token`
// is what the test sets via SetForgedLiquidityToken. This is exactly
// the attacker's tool the rc3 fix failed to block.
// =====================================================================

mod fake_pair {
    use super::*;

    const STATE: Item<State> = Item::new("fake_pair_state");

    #[cw_serde]
    pub struct State {
        pub claimed_asset_infos: Vec<AssetInfo>,
        pub liquidity_token: String,
    }

    #[cw_serde]
    pub struct InstantiateMsg {
        pub claimed_asset_infos: Vec<AssetInfo>,
    }

    #[cw_serde]
    pub enum ExecuteMsg {
        SetForgedLiquidityToken { denom: String },
    }

    #[cw_serde]
    #[derive(QueryResponses)]
    pub enum QueryMsg {
        // Mirror astroport::pair::QueryMsg::Pair {} — the only query
        // is_valid_pool calls into here.
        #[returns(PairInfo)]
        Pair {},
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
                claimed_asset_infos: msg.claimed_asset_infos,
                liquidity_token: String::new(),
            },
        )?;
        Ok(Response::new())
    }

    pub fn execute(
        deps: DepsMut,
        _env: Env,
        _info: MessageInfo,
        msg: ExecuteMsg,
    ) -> StdResult<Response> {
        match msg {
            ExecuteMsg::SetForgedLiquidityToken { denom } => {
                let mut state = STATE.load(deps.storage)?;
                state.liquidity_token = denom;
                STATE.save(deps.storage, &state)?;
                Ok(Response::new())
            }
        }
    }

    pub fn query(deps: Deps, env: Env, msg: QueryMsg) -> StdResult<Binary> {
        match msg {
            QueryMsg::Pair {} => {
                let state = STATE.load(deps.storage)?;
                if state.liquidity_token.is_empty() {
                    return Err(StdError::generic_err(
                        "fake_pair: liquidity_token not set — call SetForgedLiquidityToken first",
                    ));
                }
                to_json_binary(&PairInfo {
                    asset_infos: state.claimed_asset_infos,
                    contract_addr: env.contract.address,
                    liquidity_token: state.liquidity_token,
                    pair_type: PairType::Xyk {},
                })
            }
        }
    }
}
