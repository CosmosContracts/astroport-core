//! **AUDIT REGRESSION GATE** for the P2.5 cw20-LP strip.
//!
//! After stripping `ExecuteMsg::Receive(Cw20ReceiveMsg)` and the
//! `Cw20Msg::{Deposit, DepositFor}` LP-side hook variants, the cw20-as-
//! REWARD-token path must remain functional. Juno has real cw20 tokens
//! in circulation (RAW, NETA, MARBLE, legacy projects); projects must
//! be able to incentivize a pool with their own cw20.
//!
//! This test exercises the full cw20-reward path:
//!
//! 1. Deploy a cw20 token; mint to a 3rd-party funder.
//! 2. Funder calls `cw20::IncreaseAllowance(spender = incentives, ...)`.
//! 3. Funder calls `incentives::Incentivize { schedule { reward: Asset {
//!    info: AssetInfo::Token { contract_addr: <cw20> }, ... } } }`.
//!    The contract pulls reward tokens via `cw20::TransferFrom`.
//! 4. Advance time; LP claims; contract dispatches `cw20::Transfer`
//!    to the LP for the accrued reward.
//!
//! If the cw20-LP strip accidentally broke the reward-side, one of
//! these three calls would fail. This test is the gate.

use cosmwasm_std::{coin, Addr, Timestamp, Uint128};

use astroport::asset::{Asset, AssetInfo, PairInfo};
use astroport::factory::{ExecuteMsg as FactoryExecuteMsg, PairType, QueryMsg as FactoryQueryMsg};
use astroport::incentives::{ExecuteMsg as IncentivesExecuteMsg, InputSchedule, EPOCHS_START};
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
fn cw20_as_external_reward_token_survives_lp_strip() {
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

    // Deploy a separate cw20 reward token (not the keep-set's
    // token_code_id, just so the test is fully self-contained).
    let cw20_code_id = app.store_code(Box::new(ContractWrapper::new_with_empty(
        cw20_base::contract::execute,
        cw20_base::contract::instantiate,
        cw20_base::contract::query,
    )));
    let funder = app.api().addr_make(FUNDER);
    let cw20_reward: Addr = app
        .instantiate_contract(
            cw20_code_id,
            handles.deployer.clone(),
            &cw20_base::msg::InstantiateMsg {
                name: "Project Token".to_string(),
                symbol: "PROJ".to_string(),
                decimals: 6,
                initial_balances: vec![cw20::Cw20Coin {
                    address: funder.to_string(),
                    amount: Uint128::new(REWARD_AMOUNT),
                }],
                mint: None,
                marketing: None,
            },
            &[],
            "project-cw20",
            None,
        )
        .unwrap();

    // 1. Stand up a pool + stake Alice's LP.
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

    // 2. Funder grants allowance for the reward to the incentives contract.
    app.execute_contract(
        funder.clone(),
        cw20_reward.clone(),
        &cw20::Cw20ExecuteMsg::IncreaseAllowance {
            spender: inc.incentives.to_string(),
            amount: Uint128::new(REWARD_AMOUNT),
            expires: None,
        },
        &[],
    )
    .expect("IncreaseAllowance succeeds");

    // 3. Funder calls Incentivize. Contract pulls cw20 via TransferFrom.
    //    NOTE: We pass the cw20 contract address as the canonical Addr
    //    string — addr_validate must accept it (it's a real bech32
    //    contract address from MockAddressGenerator).
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
    .expect("Incentivize with cw20 reward succeeds (AUDIT GATE — strip didn't break reward path)");

    // Contract should now hold the cw20 reward (pulled via TransferFrom).
    let inc_cw20: cw20::BalanceResponse = app
        .wrap()
        .query_wasm_smart(
            cw20_reward.clone(),
            &cw20::Cw20QueryMsg::Balance {
                address: inc.incentives.to_string(),
            },
        )
        .unwrap();
    assert_eq!(
        inc_cw20.balance,
        Uint128::new(REWARD_AMOUNT),
        "TransferFrom moved cw20 reward to incentives contract"
    );

    let funder_cw20: cw20::BalanceResponse = app
        .wrap()
        .query_wasm_smart(
            cw20_reward.clone(),
            &cw20::Cw20QueryMsg::Balance {
                address: funder.to_string(),
            },
        )
        .unwrap();
    assert_eq!(
        funder_cw20.balance,
        Uint128::zero(),
        "funder's cw20 balance is zero after TransferFrom"
    );

    // 4. Advance time past schedule start.
    app.update_block(|b| {
        b.time = b.time.plus_seconds(7 * 86400);
        b.height += 1;
    });

    // 5. ClaimRewards — Alice receives cw20::Transfer from the contract.
    let alice_cw20_before: cw20::BalanceResponse = app
        .wrap()
        .query_wasm_smart(
            cw20_reward.clone(),
            &cw20::Cw20QueryMsg::Balance {
                address: alice.to_string(),
            },
        )
        .unwrap();

    app.execute_contract(
        alice.clone(),
        inc.incentives.clone(),
        &IncentivesExecuteMsg::ClaimRewards {
            lp_tokens: vec![lp_denom],
        },
        &[],
    )
    .expect("ClaimRewards delivers cw20 reward via cw20::Transfer");

    let alice_cw20_after: cw20::BalanceResponse = app
        .wrap()
        .query_wasm_smart(
            cw20_reward,
            &cw20::Cw20QueryMsg::Balance {
                address: alice.to_string(),
            },
        )
        .unwrap();
    assert!(
        alice_cw20_after.balance > alice_cw20_before.balance,
        "Alice's cw20 reward balance increased after claim (AUDIT GATE)"
    );
}

// =====================================================================
// helpers (local to this test target — duplicated from incentives_*.rs)
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
