//! Third-party external incentivize with a NATIVE reward token (non-ujuno).
//! Verifies the permissionless `Incentivize` entry point works end-to-end
//! for a native reward token (e.g., a project's TF denom or an IBC denom).
//!
//! Wire path: funder calls `Incentivize { lp_token, schedule }` with
//! `info.funds` containing the reward amount. Contract registers the
//! schedule, deducts the fee (if configured), and on `ClaimRewards` the
//! LP receives the reward via `BankMsg::Send`.

use cosmwasm_std::{coin, Timestamp, Uint128};

use astroport::asset::{Asset, AssetInfo, PairInfo};
use astroport::factory::{ExecuteMsg as FactoryExecuteMsg, PairType, QueryMsg as FactoryQueryMsg};
use astroport::incentives::{
    ExecuteMsg as IncentivesExecuteMsg, InputSchedule, QueryMsg as IncentivesQueryMsg, EPOCHS_START,
};
use astroport::pair::ExecuteMsg as PairExecuteMsg;
use astroport_test::cw_multi_test::Executor;

use astroport_juno_integration_tests::{
    balance_of, deploy_incentives_addon, deploy_keep_set, fund, mock_app, KeepSetHandles, TestApp,
    MOCK_USDC, UJUNO,
};

const ALICE: &str = "alice";
const FUNDER: &str = "funder";

const LP_SEED: u128 = 100_000_000_000;
/// Mock 3rd-party native reward token (a project's own TF denom).
const PROJECT_REWARD: &str = "factory/juno1projectaddr/PROJ";
/// Schedule reward amount. Must be large enough that
/// `reward.amount / (end_ts - block_ts) >= 1` for rps validation.
const REWARD_AMOUNT: u128 = 100_000_000;

#[test]
fn third_party_native_external_incentive_accrues_and_claims() {
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

    // 2. Funder (a 3rd-party project) externally incentivizes the pool.
    //    No internal emissions configured — this asserts the external-only
    //    flow works in isolation.
    let funder = app.api().addr_make(FUNDER);
    let bank_admin = handles.deployer.clone();
    // Mint the project reward to the funder. We don't have a real
    // tokenfactory module for this denom; fund() relies on the deployer's
    // initial balance which only includes ujuno/USDC/ATOM, so we issue a
    // sudo Mint to the funder for this synthetic denom.
    app.sudo(astroport_test::cw_multi_test::SudoMsg::Bank(
        astroport_test::cw_multi_test::BankSudo::Mint {
            to_address: funder.to_string(),
            amount: vec![coin(REWARD_AMOUNT, PROJECT_REWARD)],
        },
    ))
    .unwrap();
    let _ = bank_admin; // keep referenced for clarity

    app.execute_contract(
        funder.clone(),
        inc.incentives.clone(),
        &IncentivesExecuteMsg::Incentivize {
            lp_token: lp_denom.clone(),
            schedule: InputSchedule {
                reward: Asset {
                    info: AssetInfo::NativeToken {
                        denom: PROJECT_REWARD.to_string(),
                    },
                    amount: Uint128::new(REWARD_AMOUNT),
                },
                duration_periods: 1,
            },
        },
        &[coin(REWARD_AMOUNT, PROJECT_REWARD)],
    )
    .expect("Incentivize with native reward succeeds");

    // The contract must now hold the reward tokens (it transferred them
    // in atomically via info.funds).
    let inc_bal = balance_of(&app, &inc.incentives, PROJECT_REWARD);
    assert_eq!(
        inc_bal,
        Uint128::new(REWARD_AMOUNT),
        "incentives contract took custody of the funder's reward tokens"
    );

    // 3. Advance time past the schedule's next_epoch_start_ts so accrual
    //    starts; advance some way INTO the schedule so meaningful reward
    //    has accumulated; advance even further would just emit more.
    //
    //    The schedule aligns to weekly epochs. After "1 day after
    //    EPOCHS_START", the next epoch start is EPOCHS_START + EPOCH_LENGTH
    //    (= +6 days). We advance there + 1 day into the active window.
    app.update_block(|b| {
        b.time = b.time.plus_seconds(7 * 86400);
        b.height += 1;
    });

    // 4. ClaimRewards — Alice should receive PROJECT_REWARD.
    let alice_reward_before = balance_of(&app, &alice, PROJECT_REWARD);
    app.execute_contract(
        alice.clone(),
        inc.incentives.clone(),
        &IncentivesExecuteMsg::ClaimRewards {
            lp_tokens: vec![lp_denom.clone()],
        },
        &[],
    )
    .unwrap();
    let alice_reward_after = balance_of(&app, &alice, PROJECT_REWARD);
    let received = alice_reward_after - alice_reward_before;
    assert!(
        received > Uint128::zero(),
        "Alice accrued some PROJECT_REWARD from the external schedule"
    );

    // 5. Sanity check: ListPools shows the pool registered (the contract
    //    discovers the pool via the first Incentivize, not via SetupPools).
    let _: Vec<String> = app
        .wrap()
        .query_wasm_smart(
            inc.incentives.clone(),
            &IncentivesQueryMsg::ListPools {
                start_after: None,
                limit: None,
            },
        )
        .unwrap();
}

// =====================================================================
// helpers (local to this test target)
// =====================================================================

fn create_pair(
    app: &mut TestApp,
    handles: &KeepSetHandles,
    denom_a: &str,
    denom_b: &str,
) -> cosmwasm_std::Addr {
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
    pair: &cosmwasm_std::Addr,
    sender: &cosmwasm_std::Addr,
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
