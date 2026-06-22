//! Smoke for the basic Astroport-Juno incentives flow:
//! deploy → CreatePair → ProvideLiquidity → SetupPools → set rate
//! → fund contract → Deposit LP into incentives → advance time →
//! ClaimRewards → assert balance increased.
//!
//! This is the test the gauge adapter's integration with this contract
//! will mechanically mirror.

use cosmwasm_std::{coin, Addr, Timestamp, Uint128};

use astroport::asset::{Asset, AssetInfo, PairInfo};
use astroport::factory::{ExecuteMsg as FactoryExecuteMsg, PairType, QueryMsg as FactoryQueryMsg};
use astroport::incentives::{
    ExecuteMsg as IncentivesExecuteMsg, GeneratorControllerUpdate, QueryMsg as IncentivesQueryMsg,
    EPOCHS_START,
};
use astroport::pair::ExecuteMsg as PairExecuteMsg;
use astroport_test::cw_multi_test::Executor;

use astroport_juno_integration_tests::{
    balance_of, deploy_incentives_addon, deploy_keep_set, fund, mock_app, KeepSetHandles,
    MOCK_USDC, UJUNO,
};

const ALICE: &str = "alice";
const LP_SEED: u128 = 100_000_000_000;
const REWARD_FUND_AMOUNT: u128 = 10_000_000;
const TOKENS_PER_SECOND: u128 = 100;

#[test]
fn incentives_setup_pools_internal_emission_accrues_to_lp() {
    let mut app = mock_app();

    // Advance to just past EPOCHS_START so schedule math has reasonable rps.
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
        None, // no spam fee for this test
    )
    .unwrap();

    // 1. CreatePair (ujuno, mock_usdc)
    let pair = create_pair(&mut app, &handles, UJUNO, MOCK_USDC);
    let lp_denom = lp_denom_of(&mut app, &handles, UJUNO, MOCK_USDC);

    // 2. Alice provides liquidity → has LP_SEED of lp_denom in her wallet
    let alice = app.api().addr_make(ALICE);
    fund(
        &mut app,
        &alice,
        vec![coin(LP_SEED, UJUNO), coin(LP_SEED, MOCK_USDC)],
    )
    .unwrap();
    provide_liquidity(&mut app, &pair, &alice, LP_SEED, LP_SEED);
    let alice_lp = balance_of(&app, &alice, &lp_denom);
    assert!(alice_lp > Uint128::zero(), "Alice received LP tokens");

    // 3. Owner sets the emission rate to TOKENS_PER_SECOND, then registers
    //    the pool for emissions via SetupPools with alloc_points = 1.
    //    Order matters: SetTokensPerSecond loops over ACTIVE_POOLS so it
    //    must be after SetupPools OR before; the contract handles both.
    app.execute_contract(
        handles.deployer.clone(),
        inc.incentives.clone(),
        &IncentivesExecuteMsg::SetupPools {
            pools: vec![(lp_denom.clone(), Uint128::new(1))],
        },
        &[],
    )
    .unwrap();
    app.execute_contract(
        handles.deployer.clone(),
        inc.incentives.clone(),
        &IncentivesExecuteMsg::SetTokensPerSecond {
            amount: Uint128::new(TOKENS_PER_SECOND),
        },
        &[],
    )
    .unwrap();

    // 4. Fund the incentives contract with the reward_token (ujuno).
    //    Without this, ClaimRewards would fail at the bank-send step.
    fund(
        &mut app,
        &inc.incentives,
        vec![coin(REWARD_FUND_AMOUNT, UJUNO)],
    )
    .unwrap();

    // 5. Alice deposits her LP into incentives.
    app.execute_contract(
        alice.clone(),
        inc.incentives.clone(),
        &IncentivesExecuteMsg::Deposit { recipient: None },
        &[coin(alice_lp.u128(), lp_denom.clone())],
    )
    .unwrap();

    // Confirm Deposit query reflects her stake.
    let deposited: Uint128 = app
        .wrap()
        .query_wasm_smart(
            inc.incentives.clone(),
            &IncentivesQueryMsg::Deposit {
                lp_token: lp_denom.clone(),
                user: alice.to_string(),
            },
        )
        .unwrap();
    assert_eq!(deposited, alice_lp);

    // 6. Advance time by 10s; she should have accrued exactly
    //    10 * TOKENS_PER_SECOND = 1000 ujuno (she's the only staker).
    let elapsed: u64 = 10;
    app.update_block(|b| {
        b.time = b.time.plus_seconds(elapsed);
        b.height += 1;
    });

    let pending: Vec<Asset> = app
        .wrap()
        .query_wasm_smart(
            inc.incentives.clone(),
            &IncentivesQueryMsg::PendingRewards {
                lp_token: lp_denom.clone(),
                user: alice.to_string(),
            },
        )
        .unwrap();
    assert_eq!(pending.len(), 1, "exactly one reward stream");
    assert_eq!(pending[0].info, AssetInfo::native(UJUNO));
    let expected = Uint128::new(elapsed as u128 * TOKENS_PER_SECOND);
    // ±1 tolerance: index = reward / total_lp involves Decimal256 division
    // that can floor by 1 unit when total_lp doesn't divide evenly.
    let diff = if pending[0].amount > expected {
        pending[0].amount - expected
    } else {
        expected - pending[0].amount
    };
    assert!(
        diff <= Uint128::new(1),
        "rps=100 × 10s ≈ 1000 ujuno accrued for the sole LP (got {}, expected {expected}±1)",
        pending[0].amount,
    );

    // 7. ClaimRewards — Alice's ujuno balance should grow by the pending amount.
    let alice_ujuno_before = balance_of(&app, &alice, UJUNO);
    app.execute_contract(
        alice.clone(),
        inc.incentives.clone(),
        &IncentivesExecuteMsg::ClaimRewards {
            lp_tokens: vec![lp_denom.clone()],
        },
        &[],
    )
    .unwrap();
    let alice_ujuno_after = balance_of(&app, &alice, UJUNO);
    let received = alice_ujuno_after - alice_ujuno_before;
    assert_eq!(
        received, pending[0].amount,
        "claim delivered exactly the pending amount (matches the prior query)"
    );
}

#[test]
fn incentives_generator_controller_can_call_setup_pools() {
    // The gauge adapter pattern: UpdateConfig sets generator_controller to
    // a dedicated address; that address (not just owner) can call SetupPools.
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
    let pair = create_pair(&mut app, &handles, UJUNO, MOCK_USDC);
    let lp_denom = lp_denom_of(&mut app, &handles, UJUNO, MOCK_USDC);
    let _ = pair; // keep pair to satisfy registration

    // Wire a "controller" address.
    let controller = app.api().addr_make("gauge_adapter_mock");
    app.execute_contract(
        handles.deployer.clone(),
        inc.incentives.clone(),
        &IncentivesExecuteMsg::UpdateConfig {
            generator_controller: GeneratorControllerUpdate::Set(controller.to_string()),
            guardian: None,
            incentivization_fee_info: None,
            token_transfer_gas_limit: None,
        },
        &[],
    )
    .unwrap();

    // Controller calls SetupPools — must succeed.
    app.execute_contract(
        controller.clone(),
        inc.incentives.clone(),
        &IncentivesExecuteMsg::SetupPools {
            pools: vec![(lp_denom.clone(), Uint128::new(1))],
        },
        &[],
    )
    .expect("generator_controller can call SetupPools");

    // A random address must NOT be able to.
    let interloper = app.api().addr_make("randomuser");
    app.execute_contract(
        interloper,
        inc.incentives.clone(),
        &IncentivesExecuteMsg::SetupPools {
            pools: vec![(lp_denom, Uint128::new(2))],
        },
        &[],
    )
    .expect_err("non-controller / non-owner must be rejected");
}

// =====================================================================
// helpers (local to this test target)
// =====================================================================

fn create_pair(
    app: &mut astroport_juno_integration_tests::TestApp,
    handles: &KeepSetHandles,
    denom_a: &str,
    denom_b: &str,
) -> Addr {
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
    app: &mut astroport_juno_integration_tests::TestApp,
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
    app: &mut astroport_juno_integration_tests::TestApp,
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
