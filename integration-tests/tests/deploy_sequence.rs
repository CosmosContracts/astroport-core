//! End-to-end smoke of the v1 keep-set deploy sequence. Mirrors
//! `planning/06-deploy-runbook.md` step-for-step; if this test passes,
//! the deploy script in P5 (`scripts/deploy/uni7.sh`) should round-trip
//! mechanically.

use cosmwasm_std::{coin, to_json_binary, Addr, Coin, Uint128};

use astroport::asset::{Asset, AssetInfo, PairInfo};
use astroport::common::LP_SUBDENOM;
use astroport::factory::{ExecuteMsg as FactoryExecuteMsg, PairType, QueryMsg as FactoryQueryMsg};
use astroport::pair::{ExecuteMsg as PairExecuteMsg, PoolResponse, QueryMsg as PairQueryMsg};
use astroport_test::cw_multi_test::Executor;

use astroport_juno_integration_tests::{
    balance_of, deploy_keep_set, fund, mock_app, MOCK_USDC, UJUNO,
};

const ALICE: &str = "alice";
const BOB: &str = "bob";

/// Each LP's initial seed per side of the pool. Chosen to be large
/// enough that the minimum-liquidity reserve (1000) is a rounding
/// detail, not a load-bearing fraction of the share math.
const LP_SEED: u128 = 100_000_000_000;
const SWAP_AMOUNT: u128 = 1_000_000_000;

#[test]
fn deploy_keep_set_create_xyk_pair_and_round_trip() {
    let mut app = mock_app();
    let handles = deploy_keep_set(&mut app).unwrap();

    // ---- Step 1: CreatePair, assert TF LP denom shape ----
    let alice = Addr::unchecked(ALICE);
    let bob = Addr::unchecked(BOB);

    app.execute_contract(
        handles.deployer.clone(),
        handles.factory.clone(),
        &FactoryExecuteMsg::CreatePair {
            pair_type: PairType::Xyk {},
            asset_infos: vec![
                AssetInfo::NativeToken {
                    denom: UJUNO.to_string(),
                },
                AssetInfo::NativeToken {
                    denom: MOCK_USDC.to_string(),
                },
            ],
            init_params: None,
        },
        &[],
    )
    .unwrap();

    let pair_info: PairInfo = app
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

    let pair = pair_info.contract_addr.clone();
    let lp_denom = pair_info.liquidity_token.clone();
    let expected_lp_denom = format!("factory/{}/{}", pair, LP_SUBDENOM);
    assert_eq!(
        lp_denom, expected_lp_denom,
        "TF LP denom shape must be factory/{{pair_addr}}/astroport/share — the UI relies on this"
    );

    // ---- Step 2: Alice + Bob each ProvideLiquidity ----
    fund(
        &mut app,
        &alice,
        vec![coin(LP_SEED, UJUNO), coin(LP_SEED, MOCK_USDC)],
    )
    .unwrap();
    fund(
        &mut app,
        &bob,
        vec![coin(LP_SEED, UJUNO), coin(LP_SEED, MOCK_USDC)],
    )
    .unwrap();

    provide_liquidity(&mut app, &pair, &alice, LP_SEED, LP_SEED);
    provide_liquidity(&mut app, &pair, &bob, LP_SEED, LP_SEED);

    let alice_lp = balance_of(&app, &alice, &lp_denom);
    let bob_lp = balance_of(&app, &bob, &lp_denom);
    assert!(alice_lp > Uint128::zero(), "alice received LP tokens");
    assert!(bob_lp > Uint128::zero(), "bob received LP tokens");

    // Total minted LP-supply equals sum of LP balances + the
    // MINIMUM_LIQUIDITY_AMOUNT reserve burned to the contract itself.
    let pool: PoolResponse = app
        .wrap()
        .query_wasm_smart(pair.clone(), &PairQueryMsg::Pool {})
        .unwrap();
    let total_minted = pool.total_share;
    let bucket = balance_of(&app, &pair, &lp_denom);
    assert_eq!(
        total_minted,
        alice_lp + bob_lp + bucket,
        "total LP supply == sum of LP balances + contract-side reserve"
    );

    // ---- Step 3: Alice swaps ujuno → mock_usdc ----
    fund(&mut app, &alice, vec![coin(SWAP_AMOUNT, UJUNO)]).unwrap();
    let alice_usdc_before = balance_of(&app, &alice, MOCK_USDC);

    app.execute_contract(
        alice.clone(),
        pair.clone(),
        &PairExecuteMsg::Swap {
            offer_asset: Asset {
                info: AssetInfo::NativeToken {
                    denom: UJUNO.to_string(),
                },
                amount: Uint128::new(SWAP_AMOUNT),
            },
            ask_asset_info: None,
            belief_price: None,
            max_spread: None,
            to: None,
        },
        &[coin(SWAP_AMOUNT, UJUNO)],
    )
    .unwrap();

    let alice_usdc_after = balance_of(&app, &alice, MOCK_USDC);
    let received = alice_usdc_after - alice_usdc_before;
    assert!(
        received > Uint128::zero(),
        "swap produced a non-zero USDC return"
    );

    // The pool was seeded balanced (200B/200B after 2 LPs); a 1B ujuno
    // swap with 30 bps fee should return ~ 1B * (200B / (200B + 1B)) *
    // 0.997, which is comfortably below the 1B offer amount.
    assert!(
        received < Uint128::new(SWAP_AMOUNT),
        "received less than offered (constant-product + 30 bps fee), got {received}"
    );

    // ---- Step 4: Alice WithdrawLiquidity of 25% ----
    let alice_burn = alice_lp / Uint128::new(4);
    let alice_ujuno_before = balance_of(&app, &alice, UJUNO);
    let alice_usdc_before_w = balance_of(&app, &alice, MOCK_USDC);

    app.execute_contract(
        alice.clone(),
        pair.clone(),
        &PairExecuteMsg::WithdrawLiquidity {
            assets: vec![],
            min_assets_to_receive: None,
        },
        &[Coin {
            denom: lp_denom.clone(),
            amount: alice_burn,
        }],
    )
    .unwrap();

    let alice_ujuno_after = balance_of(&app, &alice, UJUNO);
    let alice_usdc_after_w = balance_of(&app, &alice, MOCK_USDC);
    assert!(
        alice_ujuno_after > alice_ujuno_before,
        "WithdrawLiquidity returned ujuno"
    );
    assert!(
        alice_usdc_after_w > alice_usdc_before_w,
        "WithdrawLiquidity returned mock_usdc"
    );

    // Pool accounting: total_share after withdraw == previous total - burn.
    let pool_after: PoolResponse = app
        .wrap()
        .query_wasm_smart(pair.clone(), &PairQueryMsg::Pool {})
        .unwrap();
    assert_eq!(
        pool_after.total_share,
        total_minted - alice_burn,
        "total LP supply decreased by exactly the burned amount"
    );
}

#[test]
fn whitelist_post_neutron_strip_registers_under_expected_cw2_name() {
    // The keep-set deploy already instantiates the whitelist as part of
    // `deploy_keep_set`. This test makes the cw2 contract-name assertion
    // explicit: post-Neutron-strip, the contract must still register as
    // "astroport-whitelist" so downstream tooling that greps for that
    // string keeps working.
    use cosmwasm_std::from_json;

    let mut app = mock_app();
    let handles = deploy_keep_set(&mut app).unwrap();

    let raw = app
        .wrap()
        .query_wasm_raw(&handles.whitelist, b"contract_info")
        .unwrap()
        .expect("cw2 contract_info present after instantiate");
    let info: cw2::ContractVersion = from_json(&raw).unwrap();
    assert_eq!(
        info.contract, "astroport-whitelist",
        "post-strip whitelist must still register as 'astroport-whitelist' \
         in cw2 — downstream tooling matches on this name"
    );
}

// =====================================================================
// helpers
// =====================================================================

fn provide_liquidity(
    app: &mut astroport_test::modules::stargate::StargateApp,
    pair: &Addr,
    sender: &Addr,
    ujuno_amount: u128,
    usdc_amount: u128,
) {
    let msg = PairExecuteMsg::ProvideLiquidity {
        assets: vec![
            Asset {
                info: AssetInfo::NativeToken {
                    denom: UJUNO.to_string(),
                },
                amount: Uint128::new(ujuno_amount),
            },
            Asset {
                info: AssetInfo::NativeToken {
                    denom: MOCK_USDC.to_string(),
                },
                amount: Uint128::new(usdc_amount),
            },
        ],
        slippage_tolerance: None,
        auto_stake: None,
        receiver: None,
        min_lp_to_receive: None,
    };
    let funds = vec![
        coin(ujuno_amount, UJUNO),
        coin(usdc_amount, MOCK_USDC),
    ];
    // ProvideLiquidity expects funds sorted lexicographically by denom.
    let mut funds = funds;
    funds.sort_by(|a, b| a.denom.cmp(&b.denom));
    let _ = to_json_binary(&msg).unwrap(); // sanity-check serialization
    app.execute_contract(sender.clone(), pair.clone(), &msg, &funds)
        .unwrap_or_else(|e| panic!("ProvideLiquidity failed: {e}"));
}
