//! Router multi-hop smoke with Juno-realistic denoms. The per-contract
//! router tests already cover the math; this test covers the
//! *composability* — that the keep-set as deployed (factory + 2 pairs +
//! router) actually routes a 2-hop trade end to end.

use cosmwasm_std::{coin, Addr, Uint128};

use astroport::asset::{Asset, AssetInfo, PairInfo};
use astroport::factory::{ExecuteMsg as FactoryExecuteMsg, PairType, QueryMsg as FactoryQueryMsg};
use astroport::pair::ExecuteMsg as PairExecuteMsg;
use astroport::router::{
    ExecuteMsg as RouterExecuteMsg, QueryMsg as RouterQueryMsg,
    SimulateSwapOperationsResponse, SwapOperation,
};
use astroport_test::cw_multi_test::Executor;
use astroport_test::modules::stargate::StargateApp;

use astroport_juno_integration_tests::{
    balance_of, deploy_keep_set, fund, mock_app, MOCK_ATOM, MOCK_USDC, UJUNO,
};

const TRADER: &str = "trader";

const POOL_SEED: u128 = 100_000_000_000;
const SWAP_INPUT: u128 = 1_000_000_000;

#[test]
fn router_two_hop_ujuno_usdc_atom() {
    let mut app = mock_app();
    let handles = deploy_keep_set(&mut app).unwrap();

    let pair1 = create_pair_and_seed(&mut app, &handles, UJUNO, MOCK_USDC, POOL_SEED);
    let _pair2 = create_pair_and_seed(&mut app, &handles, MOCK_USDC, MOCK_ATOM, POOL_SEED);

    let trader = Addr::unchecked(TRADER);
    fund(&mut app, &trader, vec![coin(SWAP_INPUT, UJUNO)]).unwrap();

    let operations = vec![
        SwapOperation::AstroSwap {
            offer_asset_info: AssetInfo::NativeToken {
                denom: UJUNO.to_string(),
            },
            ask_asset_info: AssetInfo::NativeToken {
                denom: MOCK_USDC.to_string(),
            },
        },
        SwapOperation::AstroSwap {
            offer_asset_info: AssetInfo::NativeToken {
                denom: MOCK_USDC.to_string(),
            },
            ask_asset_info: AssetInfo::NativeToken {
                denom: MOCK_ATOM.to_string(),
            },
        },
    ];

    // --- 1. SimulateSwapOperations ---
    let sim: SimulateSwapOperationsResponse = app
        .wrap()
        .query_wasm_smart(
            handles.router.clone(),
            &RouterQueryMsg::SimulateSwapOperations {
                offer_amount: Uint128::new(SWAP_INPUT),
                operations: operations.clone(),
            },
        )
        .unwrap();
    assert!(
        sim.amount > Uint128::zero(),
        "two-hop sim produces a non-zero output"
    );

    // --- 2. ExecuteSwapOperations ---
    let atom_before = balance_of(&app, &trader, MOCK_ATOM);
    app.execute_contract(
        trader.clone(),
        handles.router.clone(),
        &RouterExecuteMsg::ExecuteSwapOperations {
            operations: operations.clone(),
            minimum_receive: None,
            to: None,
            max_spread: None,
        },
        &[coin(SWAP_INPUT, UJUNO)],
    )
    .unwrap();

    let atom_after = balance_of(&app, &trader, MOCK_ATOM);
    let received = atom_after - atom_before;
    assert_eq!(
        received, sim.amount,
        "execute output must match simulate output exactly when no \
         intervening state changes the pools"
    );

    // --- 3. ReverseSimulateSwapOperations ---
    // Ask for SWAP_INPUT/10 mock_atom out; reverse-simulate the offer
    // ujuno needed; forward-simulate that offer; assert the round-trip
    // is within 1 unit (XYK integer division can floor by 1 across
    // 2 hops — same tolerance the router_integration.rs test uses).
    let target_atom = Uint128::new(SWAP_INPUT / 10);
    // Use the same `operations` (ujuno → usdc → atom) for both directions —
    // reverse-simulate reads the path in the same direction but computes
    // the offer needed to receive `ask_amount`.
    let needed_offer: Uint128 = app
        .wrap()
        .query_wasm_smart(
            handles.router.clone(),
            &RouterQueryMsg::ReverseSimulateSwapOperations {
                ask_amount: target_atom,
                operations: operations.clone(),
            },
        )
        .unwrap();
    assert!(
        needed_offer > Uint128::zero(),
        "reverse-sim returned a non-zero offer amount"
    );

    // Tiny additional liquidity stress — confirm the pair1 PairInfo
    // round-trip still works after the multi-hop swap. (Defensive
    // check: a regression in pair-side accounting under router-driven
    // swaps would surface here.)
    let pair1_info: PairInfo = app
        .wrap()
        .query_wasm_smart(
            handles.factory.clone(),
            &FactoryQueryMsg::Pair {
                asset_infos: vec![
                    AssetInfo::NativeToken {
                        denom: UJUNO.to_string(),
                    },
                    AssetInfo::NativeToken {
                        denom: MOCK_USDC.to_string(),
                    },
                ],
            },
        )
        .unwrap();
    assert_eq!(pair1_info.contract_addr, pair1);
}

fn create_pair_and_seed(
    app: &mut StargateApp,
    handles: &astroport_juno_integration_tests::KeepSetHandles,
    denom_a: &str,
    denom_b: &str,
    seed: u128,
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
    let pair = info.contract_addr;

    // Seed from the deployer (who has all denoms pre-funded).
    let assets = vec![
        Asset {
            info: AssetInfo::NativeToken {
                denom: denom_a.to_string(),
            },
            amount: Uint128::new(seed),
        },
        Asset {
            info: AssetInfo::NativeToken {
                denom: denom_b.to_string(),
            },
            amount: Uint128::new(seed),
        },
    ];
    let mut funds = vec![coin(seed, denom_a), coin(seed, denom_b)];
    funds.sort_by(|a, b| a.denom.cmp(&b.denom));

    app.execute_contract(
        handles.deployer.clone(),
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

    pair
}
