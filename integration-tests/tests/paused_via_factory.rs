//! Confirms `pool_unpause_at` travels through `factory.CreatePair` to the
//! pair's `init_params` deserialization and lands on the swap gate. This
//! is the wire path the cw-abc graduation flow in `dao-contracts` will
//! invoke when it seeds a graduating pool (see
//! `memory/abc-graduation-architecture-astroport.md`).
//!
//! The pair-side unit tests already cover the gate behavior with
//! directly-instantiated pairs; this test asserts the field survives
//! the factory's `Binary` pass-through path verbatim.

use cosmwasm_std::{coin, to_json_binary, Addr, Uint128};

use astroport::asset::{Asset, AssetInfo, PairInfo};
use astroport::factory::{ExecuteMsg as FactoryExecuteMsg, PairType, QueryMsg as FactoryQueryMsg};
use astroport::pair::{ExecuteMsg as PairExecuteMsg, XYKPoolParams};
use astroport_pair::error::ContractError as PairContractError;
use astroport_test::cw_multi_test::Executor;

use astroport_juno_integration_tests::{deploy_keep_set, fund, mock_app, MOCK_USDC, UJUNO};

const LP: &str = "lpwallet";
const SNIPER: &str = "sniper";

const POOL_SEED: u128 = 100_000_000_000;
const PAUSE_SECONDS: u64 = 60;

#[test]
fn pool_unpause_at_threaded_through_factory_create_pair() {
    let mut app = mock_app();
    let handles = deploy_keep_set(&mut app).unwrap();

    let unpause_at = app.block_info().time.plus_seconds(PAUSE_SECONDS);

    // The cw-abc graduation flow will call factory.CreatePair with
    // init_params holding the XYKPoolParams { pool_unpause_at: ... }.
    // This test asserts that exact wire path lands on the swap gate.
    let init_params = to_json_binary(&XYKPoolParams {
        track_asset_balances: None,
        pool_unpause_at: Some(unpause_at),
    })
    .unwrap();

    let asset_infos = vec![
        AssetInfo::NativeToken {
            denom: UJUNO.to_string(),
        },
        AssetInfo::NativeToken {
            denom: MOCK_USDC.to_string(),
        },
    ];

    app.execute_contract(
        handles.deployer.clone(),
        handles.factory.clone(),
        &FactoryExecuteMsg::CreatePair {
            pair_type: PairType::Xyk {},
            asset_infos: asset_infos.clone(),
            init_params: Some(init_params),
        },
        &[],
    )
    .unwrap();

    let pair_info: PairInfo = app
        .wrap()
        .query_wasm_smart(
            handles.factory.clone(),
            &FactoryQueryMsg::Pair { asset_infos },
        )
        .unwrap();
    let pair = pair_info.contract_addr.clone();

    // --- 1. LP-side flow stays open during the pause window. ---
    let lp_addr = Addr::unchecked(LP);
    fund(
        &mut app,
        &lp_addr,
        vec![coin(POOL_SEED, UJUNO), coin(POOL_SEED, MOCK_USDC)],
    )
    .unwrap();

    let provide_msg = PairExecuteMsg::ProvideLiquidity {
        assets: vec![
            Asset {
                info: AssetInfo::NativeToken {
                    denom: UJUNO.to_string(),
                },
                amount: Uint128::new(POOL_SEED),
            },
            Asset {
                info: AssetInfo::NativeToken {
                    denom: MOCK_USDC.to_string(),
                },
                amount: Uint128::new(POOL_SEED),
            },
        ],
        slippage_tolerance: None,
        auto_stake: None,
        receiver: None,
        min_lp_to_receive: None,
    };
    let mut provide_funds = vec![coin(POOL_SEED, UJUNO), coin(POOL_SEED, MOCK_USDC)];
    provide_funds.sort_by(|a, b| a.denom.cmp(&b.denom));
    app.execute_contract(lp_addr.clone(), pair.clone(), &provide_msg, &provide_funds)
        .expect("ProvideLiquidity must succeed during the pause window");

    // --- 2. Swap from a hypothetical sniper rejects with PoolPaused. ---
    let sniper = Addr::unchecked(SNIPER);
    fund(&mut app, &sniper, vec![coin(1_000_000, UJUNO)]).unwrap();

    let swap_msg = PairExecuteMsg::Swap {
        offer_asset: Asset {
            info: AssetInfo::NativeToken {
                denom: UJUNO.to_string(),
            },
            amount: Uint128::new(1_000_000),
        },
        ask_asset_info: None,
        belief_price: None,
        max_spread: None,
        to: None,
    };

    let err = app
        .execute_contract(
            sniper.clone(),
            pair.clone(),
            &swap_msg,
            &[coin(1_000_000, UJUNO)],
        )
        .expect_err("Swap must fail during the pause window");
    assert_eq!(
        err.downcast::<PairContractError>().unwrap(),
        PairContractError::PoolPaused { unpause_at },
        "the PoolPaused error must carry the exact unpause_at that \
         was passed through factory.CreatePair's init_params"
    );

    // --- 3. After unpause_at elapses, the same Swap succeeds. ---
    app.update_block(|b| {
        b.time = b.time.plus_seconds(PAUSE_SECONDS + 1);
        b.height += 1;
    });

    app.execute_contract(
        sniper.clone(),
        pair.clone(),
        &swap_msg,
        &[coin(1_000_000, UJUNO)],
    )
    .expect("Swap must succeed after unpause_at elapses");
}
