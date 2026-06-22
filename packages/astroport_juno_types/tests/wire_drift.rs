//! Wire-format drift guard against `packages/astroport`.
//!
//! This test file is the canonical lock-down for the MIT shim's wire
//! surface. For every public type that's mirrored between the shim
//! (`astroport_juno_types`) and the GPL `astroport` crate, we run a
//! **bidirectional** JSON round-trip:
//!
//!   shim  ── serialize ──► JSON ── deserialize ──► GPL ── serialize ──► JSON'
//!   then assert JSON == JSON' (shim→GPL leg)
//!
//!   GPL   ── serialize ──► JSON ── deserialize ──► shim ── serialize ──► JSON'
//!   then assert JSON == JSON' (GPL→shim leg)
//!
//! Any silent divergence in field names, enum tagging, defaults, or
//! variant ordering will fail one of these assertions.
//!
//! Comparison is done on `serde_json::Value`, which normalizes
//! whitespace and key ordering — so a passing assertion means the two
//! crates emit equivalent JSON, not byte-identical strings.
//!
//! Intentional omissions: the shim deliberately strips admin-only
//! variants (factory: UpdateConfig/UpdatePairConfig/UpdateTrackerConfig/
//! Deregister/ProposeNewOwner/...; pair: UpdateConfig/Custom/...;
//! incentives: UpdateConfig/RemoveRewardFromPool/ClaimOrphanedRewards/
//! UpdateBlockedTokenslist/DeactivatePool/.../ownership rotation;
//! incentives::InstantiateMsg; pair::QueryMsg::QueryComputeD). Those
//! variants are NOT round-tripped — only the public, cross-contract-
//! callable surface is mirrored, and the shim is intentionally narrower
//! than the GPL crate. See per-module comments below.
//!
//! The dev-dep on `astroport` is scoped to this test target only — the
//! shipped library has no GPL linkage.

use astroport_juno_types as juno;
use cosmwasm_std::{Addr, Binary, Coin, Decimal, Decimal256, Timestamp, Uint128};
use serde::{de::DeserializeOwned, Serialize};
use serde_json::Value;

/// Bidirectional wire-equivalence assertion between a shim type `S` and
/// its GPL counterpart `U`.
///
/// Steps:
///   1. Serialize the shim fixture to JSON.
///   2. Deserialize that JSON into the GPL type (shim → GPL leg).
///   3. Re-serialize the GPL value to JSON; assert the two JSONs are
///      structurally equal (comparison on `serde_json::Value` ignores
///      key ordering and whitespace).
///   4. Deserialize the original JSON back into the shim type and
///      re-serialize; assert structural equality. This catches the case
///      where the shim type drops fields on deserialize.
///   5. The caller separately supplies a GPL-side fixture via
///      [`bidir_roundtrip_with_upstream`] to cover the reverse leg
///      (GPL fixture → shim → JSON-equal).
fn bidir_roundtrip<S, U>(shim_fixture: S, upstream_fixture: U)
where
    S: Serialize + DeserializeOwned,
    U: Serialize + DeserializeOwned,
{
    // ---- shim → GPL leg ----
    let shim_json = serde_json::to_string(&shim_fixture).expect("shim type serializes");
    let shim_value: Value = serde_json::from_str(&shim_json).expect("shim JSON parses");

    let as_upstream: U = serde_json::from_str(&shim_json).unwrap_or_else(|e| {
        panic!("upstream type failed to deserialize shim JSON: {e}\n{shim_json}")
    });
    let upstream_json = serde_json::to_string(&as_upstream).expect("upstream serializes");
    let upstream_value: Value = serde_json::from_str(&upstream_json).expect("upstream JSON parses");
    assert_eq!(
        shim_value, upstream_value,
        "shim → GPL → JSON drifted:\n  shim:     {shim_json}\n  upstream: {upstream_json}"
    );

    // Self-roundtrip on the shim side: catches "shim drops fields on
    // deserialize" (e.g. unknown-field-rename) bugs.
    let shim_reparsed: S =
        serde_json::from_str(&shim_json).expect("shim type re-deserializes its own JSON");
    let shim_json2 = serde_json::to_string(&shim_reparsed).expect("shim re-serializes");
    let shim_value2: Value = serde_json::from_str(&shim_json2).expect("shim JSON re-parses");
    assert_eq!(
        shim_value, shim_value2,
        "shim self-roundtrip drifted:\n  first:  {shim_json}\n  second: {shim_json2}"
    );

    // ---- GPL → shim leg ----
    let upstream_fixture_json =
        serde_json::to_string(&upstream_fixture).expect("upstream fixture serializes");
    let upstream_fixture_value: Value =
        serde_json::from_str(&upstream_fixture_json).expect("upstream fixture JSON parses");

    let as_shim: S = serde_json::from_str(&upstream_fixture_json).unwrap_or_else(|e| {
        panic!("shim type failed to deserialize upstream JSON: {e}\n{upstream_fixture_json}")
    });
    let shim_back_json = serde_json::to_string(&as_shim).expect("shim re-serializes upstream");
    let shim_back_value: Value =
        serde_json::from_str(&shim_back_json).expect("shim-from-upstream JSON parses");
    assert_eq!(
        upstream_fixture_value, shim_back_value,
        "GPL → shim → JSON drifted:\n  upstream: {upstream_fixture_json}\n  shim:     {shim_back_json}"
    );
}

// ============================================================
// asset
// ============================================================

#[test]
fn asset_info_native_bidir() {
    bidir_roundtrip(
        juno::asset::AssetInfo::NativeToken {
            denom: "ujuno".to_string(),
        },
        astroport::asset::AssetInfo::NativeToken {
            denom: "ujuno".to_string(),
        },
    );
}

#[test]
fn asset_info_token_bidir() {
    bidir_roundtrip(
        juno::asset::AssetInfo::Token {
            contract_addr: Addr::unchecked("juno1cw20token"),
        },
        astroport::asset::AssetInfo::Token {
            contract_addr: Addr::unchecked("juno1cw20token"),
        },
    );
}

#[test]
fn asset_bidir() {
    bidir_roundtrip(
        juno::asset::Asset {
            info: juno::asset::AssetInfo::NativeToken {
                denom: "ujuno".to_string(),
            },
            amount: Uint128::new(1_000_000),
        },
        astroport::asset::Asset {
            info: astroport::asset::AssetInfo::NativeToken {
                denom: "ujuno".to_string(),
            },
            amount: Uint128::new(1_000_000),
        },
    );
}

#[test]
fn pair_info_bidir() {
    bidir_roundtrip(
        juno::asset::PairInfo {
            asset_infos: vec![
                juno::asset::AssetInfo::NativeToken {
                    denom: "ujuno".to_string(),
                },
                juno::asset::AssetInfo::NativeToken {
                    denom: "ibc/USDC".to_string(),
                },
            ],
            contract_addr: Addr::unchecked("juno1pairaddr"),
            liquidity_token: "factory/juno1pairaddr/astroport/share".to_string(),
            pair_type: juno::factory::PairType::Xyk {},
        },
        astroport::asset::PairInfo {
            asset_infos: vec![
                astroport::asset::AssetInfo::NativeToken {
                    denom: "ujuno".to_string(),
                },
                astroport::asset::AssetInfo::NativeToken {
                    denom: "ibc/USDC".to_string(),
                },
            ],
            contract_addr: Addr::unchecked("juno1pairaddr"),
            liquidity_token: "factory/juno1pairaddr/astroport/share".to_string(),
            pair_type: astroport::factory::PairType::Xyk {},
        },
    );
}

// ============================================================
// factory
// ============================================================
//
// Intentional shim omissions on `factory::ExecuteMsg`:
//   - UpdateConfig, UpdateTrackerConfig, UpdatePairConfig, Deregister,
//     ProposeNewOwner, DropOwnershipProposal, ClaimOwnership.
// These are admin-only and not callable from downstream contracts; the
// MIT shim deliberately exposes only `CreatePair`.

#[test]
fn factory_pair_type_xyk_bidir() {
    bidir_roundtrip(
        juno::factory::PairType::Xyk {},
        astroport::factory::PairType::Xyk {},
    );
}

#[test]
fn factory_pair_type_stable_bidir() {
    bidir_roundtrip(
        juno::factory::PairType::Stable {},
        astroport::factory::PairType::Stable {},
    );
}

#[test]
fn factory_pair_type_custom_bidir() {
    bidir_roundtrip(
        juno::factory::PairType::Custom("concentrated".to_string()),
        astroport::factory::PairType::Custom("concentrated".to_string()),
    );
}

fn shim_pair_config() -> juno::factory::PairConfig {
    juno::factory::PairConfig {
        code_id: 42,
        pair_type: juno::factory::PairType::Xyk {},
        total_fee_bps: 30,
        maker_fee_bps: 0,
        is_disabled: false,
        is_generator_disabled: true,
        permissioned: false,
        whitelist: Some(vec!["juno1allowed".to_string()]),
    }
}

fn upstream_pair_config() -> astroport::factory::PairConfig {
    astroport::factory::PairConfig {
        code_id: 42,
        pair_type: astroport::factory::PairType::Xyk {},
        total_fee_bps: 30,
        maker_fee_bps: 0,
        is_disabled: false,
        is_generator_disabled: true,
        permissioned: false,
        whitelist: Some(vec!["juno1allowed".to_string()]),
    }
}

#[test]
fn factory_pair_config_bidir() {
    bidir_roundtrip(shim_pair_config(), upstream_pair_config());
}

#[test]
fn factory_tracker_config_bidir() {
    bidir_roundtrip(
        juno::factory::TrackerConfig {
            code_id: 100,
            token_factory_addr: "juno1tfaddr".to_string(),
        },
        astroport::factory::TrackerConfig {
            code_id: 100,
            token_factory_addr: "juno1tfaddr".to_string(),
        },
    );
}

#[test]
fn factory_instantiate_msg_bidir() {
    bidir_roundtrip(
        juno::factory::InstantiateMsg {
            pair_configs: vec![shim_pair_config()],
            token_code_id: 7,
            fee_address: Some("juno1maker".to_string()),
            generator_address: Some("juno1incentives".to_string()),
            owner: "juno1owner".to_string(),
            whitelist_code_id: 8,
            coin_registry_address: "juno1registry".to_string(),
            tracker_config: Some(juno::factory::TrackerConfig {
                code_id: 100,
                token_factory_addr: "juno1tfaddr".to_string(),
            }),
        },
        astroport::factory::InstantiateMsg {
            pair_configs: vec![upstream_pair_config()],
            token_code_id: 7,
            fee_address: Some("juno1maker".to_string()),
            generator_address: Some("juno1incentives".to_string()),
            owner: "juno1owner".to_string(),
            whitelist_code_id: 8,
            coin_registry_address: "juno1registry".to_string(),
            tracker_config: Some(astroport::factory::TrackerConfig {
                code_id: 100,
                token_factory_addr: "juno1tfaddr".to_string(),
            }),
        },
    );
}

#[test]
fn factory_execute_create_pair_bidir() {
    bidir_roundtrip(
        juno::factory::ExecuteMsg::CreatePair {
            pair_type: juno::factory::PairType::Xyk {},
            asset_infos: vec![
                juno::asset::AssetInfo::NativeToken {
                    denom: "ujuno".to_string(),
                },
                juno::asset::AssetInfo::NativeToken {
                    denom: "ibc/USDC".to_string(),
                },
            ],
            init_params: Some(Binary::from(b"opaque".to_vec())),
        },
        astroport::factory::ExecuteMsg::CreatePair {
            pair_type: astroport::factory::PairType::Xyk {},
            asset_infos: vec![
                astroport::asset::AssetInfo::NativeToken {
                    denom: "ujuno".to_string(),
                },
                astroport::asset::AssetInfo::NativeToken {
                    denom: "ibc/USDC".to_string(),
                },
            ],
            init_params: Some(Binary::from(b"opaque".to_vec())),
        },
    );
}

#[test]
fn factory_query_config_bidir() {
    bidir_roundtrip(
        juno::factory::QueryMsg::Config {},
        astroport::factory::QueryMsg::Config {},
    );
}

#[test]
fn factory_query_pair_bidir() {
    bidir_roundtrip(
        juno::factory::QueryMsg::Pair {
            asset_infos: vec![juno::asset::AssetInfo::NativeToken {
                denom: "ujuno".to_string(),
            }],
        },
        astroport::factory::QueryMsg::Pair {
            asset_infos: vec![astroport::asset::AssetInfo::NativeToken {
                denom: "ujuno".to_string(),
            }],
        },
    );
}

#[test]
fn factory_query_pairs_bidir() {
    bidir_roundtrip(
        juno::factory::QueryMsg::Pairs {
            start_after: Some(vec![juno::asset::AssetInfo::NativeToken {
                denom: "ujuno".to_string(),
            }]),
            limit: Some(10),
        },
        astroport::factory::QueryMsg::Pairs {
            start_after: Some(vec![astroport::asset::AssetInfo::NativeToken {
                denom: "ujuno".to_string(),
            }]),
            limit: Some(10),
        },
    );
}

#[test]
fn factory_query_fee_info_bidir() {
    bidir_roundtrip(
        juno::factory::QueryMsg::FeeInfo {
            pair_type: juno::factory::PairType::Xyk {},
        },
        astroport::factory::QueryMsg::FeeInfo {
            pair_type: astroport::factory::PairType::Xyk {},
        },
    );
}

#[test]
fn factory_query_blacklisted_pair_types_bidir() {
    bidir_roundtrip(
        juno::factory::QueryMsg::BlacklistedPairTypes {},
        astroport::factory::QueryMsg::BlacklistedPairTypes {},
    );
}

#[test]
fn factory_query_tracker_config_bidir() {
    bidir_roundtrip(
        juno::factory::QueryMsg::TrackerConfig {},
        astroport::factory::QueryMsg::TrackerConfig {},
    );
}

#[test]
fn factory_config_response_bidir() {
    bidir_roundtrip(
        juno::factory::ConfigResponse {
            owner: Addr::unchecked("juno1owner"),
            pair_configs: vec![shim_pair_config()],
            token_code_id: 7,
            fee_address: Some(Addr::unchecked("juno1maker")),
            generator_address: Some(Addr::unchecked("juno1incentives")),
            whitelist_code_id: 8,
            coin_registry_address: Addr::unchecked("juno1registry"),
        },
        astroport::factory::ConfigResponse {
            owner: Addr::unchecked("juno1owner"),
            pair_configs: vec![upstream_pair_config()],
            token_code_id: 7,
            fee_address: Some(Addr::unchecked("juno1maker")),
            generator_address: Some(Addr::unchecked("juno1incentives")),
            whitelist_code_id: 8,
            coin_registry_address: Addr::unchecked("juno1registry"),
        },
    );
}

#[test]
fn factory_pairs_response_bidir() {
    bidir_roundtrip(
        juno::factory::PairsResponse {
            pairs: vec![juno::asset::PairInfo {
                asset_infos: vec![juno::asset::AssetInfo::NativeToken {
                    denom: "ujuno".to_string(),
                }],
                contract_addr: Addr::unchecked("juno1pair"),
                liquidity_token: "factory/juno1pair/astroport/share".to_string(),
                pair_type: juno::factory::PairType::Xyk {},
            }],
        },
        astroport::factory::PairsResponse {
            pairs: vec![astroport::asset::PairInfo {
                asset_infos: vec![astroport::asset::AssetInfo::NativeToken {
                    denom: "ujuno".to_string(),
                }],
                contract_addr: Addr::unchecked("juno1pair"),
                liquidity_token: "factory/juno1pair/astroport/share".to_string(),
                pair_type: astroport::factory::PairType::Xyk {},
            }],
        },
    );
}

#[test]
fn factory_fee_info_response_bidir() {
    bidir_roundtrip(
        juno::factory::FeeInfoResponse {
            fee_address: Some(Addr::unchecked("juno1maker")),
            total_fee_bps: 30,
            maker_fee_bps: 5,
        },
        astroport::factory::FeeInfoResponse {
            fee_address: Some(Addr::unchecked("juno1maker")),
            total_fee_bps: 30,
            maker_fee_bps: 5,
        },
    );
}

// ============================================================
// pair
// ============================================================
//
// Intentional shim omissions on `pair::ExecuteMsg`:
//   - UpdateConfig, ProposeNewOwner, DropOwnershipProposal,
//     ClaimOwnership, Custom(Empty).
// These are admin-only / extension hooks. The shim ExecuteMsg is `enum`,
// not generic-over-`C`; downstream contracts never construct `Custom`.
//
// Intentional shim omissions on `pair::QueryMsg`:
//   - QueryComputeD. Internal stableswap-only debug query, not on any
//     critical path.

#[test]
fn pair_xyk_pool_params_full_bidir() {
    bidir_roundtrip(
        juno::pair::XYKPoolParams {
            track_asset_balances: Some(true),
            pool_unpause_at: Some(Timestamp::from_seconds(1_750_000_000)),
        },
        astroport::pair::XYKPoolParams {
            track_asset_balances: Some(true),
            pool_unpause_at: Some(Timestamp::from_seconds(1_750_000_000)),
        },
    );
}

#[test]
fn pair_xyk_pool_params_unpause_none_bidir() {
    bidir_roundtrip(
        juno::pair::XYKPoolParams {
            track_asset_balances: Some(false),
            pool_unpause_at: None,
        },
        astroport::pair::XYKPoolParams {
            track_asset_balances: Some(false),
            pool_unpause_at: None,
        },
    );
}

/// Explicit JSON-literal fixture: a v0.1.0 (pre-`pool_unpause_at`) caller
/// emits an XYKPoolParams payload that omits `pool_unpause_at` entirely.
/// The `#[serde(default)]` on the field must make the omission valid and
/// decode `pool_unpause_at` as `None`.
#[test]
fn pair_xyk_pool_params_unpause_omitted_default() {
    let legacy_json = r#"{"track_asset_balances":true}"#;
    let shim: juno::pair::XYKPoolParams =
        serde_json::from_str(legacy_json).expect("shim accepts legacy XYKPoolParams JSON");
    assert_eq!(shim.track_asset_balances, Some(true));
    assert_eq!(shim.pool_unpause_at, None);

    let upstream: astroport::pair::XYKPoolParams = serde_json::from_str(legacy_json)
        .expect("upstream accepts legacy XYKPoolParams JSON");
    assert_eq!(upstream.track_asset_balances, Some(true));
    assert_eq!(upstream.pool_unpause_at, None);
}

#[test]
fn pair_fee_share_config_bidir() {
    bidir_roundtrip(
        juno::pair::FeeShareConfig {
            bps: 250,
            recipient: Addr::unchecked("juno1feeshare"),
        },
        astroport::pair::FeeShareConfig {
            bps: 250,
            recipient: Addr::unchecked("juno1feeshare"),
        },
    );
}

#[test]
fn pair_instantiate_msg_bidir() {
    bidir_roundtrip(
        juno::pair::InstantiateMsg {
            asset_infos: vec![
                juno::asset::AssetInfo::NativeToken {
                    denom: "ujuno".to_string(),
                },
                juno::asset::AssetInfo::NativeToken {
                    denom: "ibc/USDC".to_string(),
                },
            ],
            token_code_id: 11,
            factory_addr: "juno1factory".to_string(),
            init_params: Some(Binary::from(b"x".to_vec())),
            pair_type: juno::factory::PairType::Xyk {},
        },
        astroport::pair::InstantiateMsg {
            asset_infos: vec![
                astroport::asset::AssetInfo::NativeToken {
                    denom: "ujuno".to_string(),
                },
                astroport::asset::AssetInfo::NativeToken {
                    denom: "ibc/USDC".to_string(),
                },
            ],
            token_code_id: 11,
            factory_addr: "juno1factory".to_string(),
            init_params: Some(Binary::from(b"x".to_vec())),
            pair_type: astroport::factory::PairType::Xyk {},
        },
    );
}

#[test]
fn pair_execute_provide_liquidity_bidir() {
    bidir_roundtrip(
        juno::pair::ExecuteMsg::ProvideLiquidity {
            assets: vec![juno::asset::Asset {
                info: juno::asset::AssetInfo::NativeToken {
                    denom: "ujuno".to_string(),
                },
                amount: Uint128::new(1_000_000),
            }],
            slippage_tolerance: Some(Decimal::percent(1)),
            auto_stake: Some(true),
            receiver: Some("juno1lp".to_string()),
            min_lp_to_receive: Some(Uint128::new(500_000)),
        },
        astroport::pair::ExecuteMsg::ProvideLiquidity {
            assets: vec![astroport::asset::Asset {
                info: astroport::asset::AssetInfo::NativeToken {
                    denom: "ujuno".to_string(),
                },
                amount: Uint128::new(1_000_000),
            }],
            slippage_tolerance: Some(Decimal::percent(1)),
            auto_stake: Some(true),
            receiver: Some("juno1lp".to_string()),
            min_lp_to_receive: Some(Uint128::new(500_000)),
        },
    );
}

#[test]
fn pair_execute_withdraw_liquidity_with_assets_bidir() {
    bidir_roundtrip(
        juno::pair::ExecuteMsg::WithdrawLiquidity {
            assets: vec![juno::asset::Asset {
                info: juno::asset::AssetInfo::NativeToken {
                    denom: "ujuno".to_string(),
                },
                amount: Uint128::new(100),
            }],
            min_assets_to_receive: Some(vec![juno::asset::Asset {
                info: juno::asset::AssetInfo::NativeToken {
                    denom: "ujuno".to_string(),
                },
                amount: Uint128::new(90),
            }]),
        },
        astroport::pair::ExecuteMsg::WithdrawLiquidity {
            assets: vec![astroport::asset::Asset {
                info: astroport::asset::AssetInfo::NativeToken {
                    denom: "ujuno".to_string(),
                },
                amount: Uint128::new(100),
            }],
            min_assets_to_receive: Some(vec![astroport::asset::Asset {
                info: astroport::asset::AssetInfo::NativeToken {
                    denom: "ujuno".to_string(),
                },
                amount: Uint128::new(90),
            }]),
        },
    );
}

/// Explicit JSON-literal fixture: a caller may omit the `assets` field on
/// `WithdrawLiquidity` (the legacy wire shape). The `#[serde(default)]`
/// must make the omission valid and decode `assets` as `vec![]`.
#[test]
fn pair_execute_withdraw_liquidity_assets_omitted_default() {
    let legacy_json = r#"{"withdraw_liquidity":{"min_assets_to_receive":null}}"#;

    let shim: juno::pair::ExecuteMsg =
        serde_json::from_str(legacy_json).expect("shim accepts legacy WithdrawLiquidity JSON");
    match shim {
        juno::pair::ExecuteMsg::WithdrawLiquidity {
            assets,
            min_assets_to_receive,
        } => {
            assert!(assets.is_empty(), "assets must default to empty vec");
            assert!(min_assets_to_receive.is_none());
        }
        other => panic!("expected WithdrawLiquidity, got {other:?}"),
    }

    let upstream: astroport::pair::ExecuteMsg = serde_json::from_str(legacy_json)
        .expect("upstream accepts legacy WithdrawLiquidity JSON");
    match upstream {
        astroport::pair::ExecuteMsg::WithdrawLiquidity {
            assets,
            min_assets_to_receive,
        } => {
            assert!(assets.is_empty());
            assert!(min_assets_to_receive.is_none());
        }
        other => panic!("expected WithdrawLiquidity, got {other:?}"),
    }
}

#[test]
fn pair_execute_swap_bidir() {
    bidir_roundtrip(
        juno::pair::ExecuteMsg::Swap {
            offer_asset: juno::asset::Asset {
                info: juno::asset::AssetInfo::NativeToken {
                    denom: "ujuno".to_string(),
                },
                amount: Uint128::new(123),
            },
            ask_asset_info: Some(juno::asset::AssetInfo::NativeToken {
                denom: "ibc/USDC".to_string(),
            }),
            belief_price: Some(Decimal::percent(50)),
            max_spread: Some(Decimal::percent(1)),
            to: Some("juno1to".to_string()),
        },
        astroport::pair::ExecuteMsg::Swap {
            offer_asset: astroport::asset::Asset {
                info: astroport::asset::AssetInfo::NativeToken {
                    denom: "ujuno".to_string(),
                },
                amount: Uint128::new(123),
            },
            ask_asset_info: Some(astroport::asset::AssetInfo::NativeToken {
                denom: "ibc/USDC".to_string(),
            }),
            belief_price: Some(Decimal::percent(50)),
            max_spread: Some(Decimal::percent(1)),
            to: Some("juno1to".to_string()),
        },
    );
}

#[test]
fn pair_cw20_hook_swap_bidir() {
    bidir_roundtrip(
        juno::pair::Cw20HookMsg::Swap {
            ask_asset_info: Some(juno::asset::AssetInfo::NativeToken {
                denom: "ibc/USDC".to_string(),
            }),
            belief_price: None,
            max_spread: Some(Decimal::percent(1)),
            to: None,
        },
        astroport::pair::Cw20HookMsg::Swap {
            ask_asset_info: Some(astroport::asset::AssetInfo::NativeToken {
                denom: "ibc/USDC".to_string(),
            }),
            belief_price: None,
            max_spread: Some(Decimal::percent(1)),
            to: None,
        },
    );
}

#[test]
fn pair_query_pool_bidir() {
    bidir_roundtrip(
        juno::pair::QueryMsg::Pool {},
        astroport::pair::QueryMsg::Pool {},
    );
}

#[test]
fn pair_query_config_bidir() {
    bidir_roundtrip(
        juno::pair::QueryMsg::Config {},
        astroport::pair::QueryMsg::Config {},
    );
}

#[test]
fn pair_query_pair_bidir() {
    bidir_roundtrip(
        juno::pair::QueryMsg::Pair {},
        astroport::pair::QueryMsg::Pair {},
    );
}

#[test]
fn pair_query_share_bidir() {
    bidir_roundtrip(
        juno::pair::QueryMsg::Share {
            amount: Uint128::new(1_000),
        },
        astroport::pair::QueryMsg::Share {
            amount: Uint128::new(1_000),
        },
    );
}

#[test]
fn pair_query_simulation_bidir() {
    bidir_roundtrip(
        juno::pair::QueryMsg::Simulation {
            offer_asset: juno::asset::Asset {
                info: juno::asset::AssetInfo::NativeToken {
                    denom: "ujuno".to_string(),
                },
                amount: Uint128::new(1_000),
            },
            ask_asset_info: Some(juno::asset::AssetInfo::NativeToken {
                denom: "ibc/USDC".to_string(),
            }),
        },
        astroport::pair::QueryMsg::Simulation {
            offer_asset: astroport::asset::Asset {
                info: astroport::asset::AssetInfo::NativeToken {
                    denom: "ujuno".to_string(),
                },
                amount: Uint128::new(1_000),
            },
            ask_asset_info: Some(astroport::asset::AssetInfo::NativeToken {
                denom: "ibc/USDC".to_string(),
            }),
        },
    );
}

#[test]
fn pair_query_reverse_simulation_bidir() {
    bidir_roundtrip(
        juno::pair::QueryMsg::ReverseSimulation {
            offer_asset_info: Some(juno::asset::AssetInfo::NativeToken {
                denom: "ujuno".to_string(),
            }),
            ask_asset: juno::asset::Asset {
                info: juno::asset::AssetInfo::NativeToken {
                    denom: "ibc/USDC".to_string(),
                },
                amount: Uint128::new(900),
            },
        },
        astroport::pair::QueryMsg::ReverseSimulation {
            offer_asset_info: Some(astroport::asset::AssetInfo::NativeToken {
                denom: "ujuno".to_string(),
            }),
            ask_asset: astroport::asset::Asset {
                info: astroport::asset::AssetInfo::NativeToken {
                    denom: "ibc/USDC".to_string(),
                },
                amount: Uint128::new(900),
            },
        },
    );
}

#[test]
fn pair_query_cumulative_prices_bidir() {
    bidir_roundtrip(
        juno::pair::QueryMsg::CumulativePrices {},
        astroport::pair::QueryMsg::CumulativePrices {},
    );
}

#[test]
fn pair_query_asset_balance_at_bidir() {
    bidir_roundtrip(
        juno::pair::QueryMsg::AssetBalanceAt {
            asset_info: juno::asset::AssetInfo::NativeToken {
                denom: "ujuno".to_string(),
            },
            block_height: 12_345u64.into(),
        },
        astroport::pair::QueryMsg::AssetBalanceAt {
            asset_info: astroport::asset::AssetInfo::NativeToken {
                denom: "ujuno".to_string(),
            },
            block_height: 12_345u64.into(),
        },
    );
}

#[test]
fn pair_query_observe_bidir() {
    bidir_roundtrip(
        juno::pair::QueryMsg::Observe { seconds_ago: 3_600 },
        astroport::pair::QueryMsg::Observe { seconds_ago: 3_600 },
    );
}

#[test]
fn pair_query_simulate_withdraw_bidir() {
    bidir_roundtrip(
        juno::pair::QueryMsg::SimulateWithdraw {
            lp_amount: Uint128::new(1_000),
        },
        astroport::pair::QueryMsg::SimulateWithdraw {
            lp_amount: Uint128::new(1_000),
        },
    );
}

#[test]
fn pair_query_simulate_provide_bidir() {
    bidir_roundtrip(
        juno::pair::QueryMsg::SimulateProvide {
            assets: vec![juno::asset::Asset {
                info: juno::asset::AssetInfo::NativeToken {
                    denom: "ujuno".to_string(),
                },
                amount: Uint128::new(1_000),
            }],
            slippage_tolerance: Some(Decimal::percent(1)),
        },
        astroport::pair::QueryMsg::SimulateProvide {
            assets: vec![astroport::asset::Asset {
                info: astroport::asset::AssetInfo::NativeToken {
                    denom: "ujuno".to_string(),
                },
                amount: Uint128::new(1_000),
            }],
            slippage_tolerance: Some(Decimal::percent(1)),
        },
    );
}

#[test]
fn pair_pool_response_bidir() {
    bidir_roundtrip(
        juno::pair::PoolResponse {
            assets: vec![juno::asset::Asset {
                info: juno::asset::AssetInfo::NativeToken {
                    denom: "ujuno".to_string(),
                },
                amount: Uint128::new(1_000_000),
            }],
            total_share: Uint128::new(500_000),
        },
        astroport::pair::PoolResponse {
            assets: vec![astroport::asset::Asset {
                info: astroport::asset::AssetInfo::NativeToken {
                    denom: "ujuno".to_string(),
                },
                amount: Uint128::new(1_000_000),
            }],
            total_share: Uint128::new(500_000),
        },
    );
}

#[test]
fn pair_simulation_response_bidir() {
    bidir_roundtrip(
        juno::pair::SimulationResponse {
            return_amount: Uint128::new(990),
            spread_amount: Uint128::new(5),
            commission_amount: Uint128::new(5),
        },
        astroport::pair::SimulationResponse {
            return_amount: Uint128::new(990),
            spread_amount: Uint128::new(5),
            commission_amount: Uint128::new(5),
        },
    );
}

#[test]
fn pair_reverse_simulation_response_bidir() {
    bidir_roundtrip(
        juno::pair::ReverseSimulationResponse {
            offer_amount: Uint128::new(1_010),
            spread_amount: Uint128::new(5),
            commission_amount: Uint128::new(5),
        },
        astroport::pair::ReverseSimulationResponse {
            offer_amount: Uint128::new(1_010),
            spread_amount: Uint128::new(5),
            commission_amount: Uint128::new(5),
        },
    );
}

// ============================================================
// router
// ============================================================
//
// Router public surface mirrors the GPL crate 1:1; no intentional
// omissions.

#[test]
fn router_swap_operation_native_swap_bidir() {
    bidir_roundtrip(
        juno::router::SwapOperation::NativeSwap {
            offer_denom: "ujuno".to_string(),
            ask_denom: "ibc/USDC".to_string(),
        },
        astroport::router::SwapOperation::NativeSwap {
            offer_denom: "ujuno".to_string(),
            ask_denom: "ibc/USDC".to_string(),
        },
    );
}

#[test]
fn router_swap_operation_astro_swap_bidir() {
    bidir_roundtrip(
        juno::router::SwapOperation::AstroSwap {
            offer_asset_info: juno::asset::AssetInfo::NativeToken {
                denom: "ujuno".to_string(),
            },
            ask_asset_info: juno::asset::AssetInfo::NativeToken {
                denom: "ibc/USDC".to_string(),
            },
        },
        astroport::router::SwapOperation::AstroSwap {
            offer_asset_info: astroport::asset::AssetInfo::NativeToken {
                denom: "ujuno".to_string(),
            },
            ask_asset_info: astroport::asset::AssetInfo::NativeToken {
                denom: "ibc/USDC".to_string(),
            },
        },
    );
}

#[test]
fn router_instantiate_msg_bidir() {
    bidir_roundtrip(
        juno::router::InstantiateMsg {
            astroport_factory: "juno1factory".to_string(),
        },
        astroport::router::InstantiateMsg {
            astroport_factory: "juno1factory".to_string(),
        },
    );
}

#[test]
fn router_execute_swap_operations_bidir() {
    bidir_roundtrip(
        juno::router::ExecuteMsg::ExecuteSwapOperations {
            operations: vec![juno::router::SwapOperation::AstroSwap {
                offer_asset_info: juno::asset::AssetInfo::NativeToken {
                    denom: "ujuno".to_string(),
                },
                ask_asset_info: juno::asset::AssetInfo::NativeToken {
                    denom: "ibc/USDC".to_string(),
                },
            }],
            minimum_receive: Some(Uint128::new(900)),
            to: Some("juno1recipient".to_string()),
            max_spread: Some(Decimal::percent(1)),
        },
        astroport::router::ExecuteMsg::ExecuteSwapOperations {
            operations: vec![astroport::router::SwapOperation::AstroSwap {
                offer_asset_info: astroport::asset::AssetInfo::NativeToken {
                    denom: "ujuno".to_string(),
                },
                ask_asset_info: astroport::asset::AssetInfo::NativeToken {
                    denom: "ibc/USDC".to_string(),
                },
            }],
            minimum_receive: Some(Uint128::new(900)),
            to: Some("juno1recipient".to_string()),
            max_spread: Some(Decimal::percent(1)),
        },
    );
}

#[test]
fn router_execute_swap_operation_bidir() {
    bidir_roundtrip(
        juno::router::ExecuteMsg::ExecuteSwapOperation {
            operation: juno::router::SwapOperation::NativeSwap {
                offer_denom: "ujuno".to_string(),
                ask_denom: "ibc/USDC".to_string(),
            },
            to: Some("juno1to".to_string()),
            max_spread: Some(Decimal::percent(1)),
            single: true,
        },
        astroport::router::ExecuteMsg::ExecuteSwapOperation {
            operation: astroport::router::SwapOperation::NativeSwap {
                offer_denom: "ujuno".to_string(),
                ask_denom: "ibc/USDC".to_string(),
            },
            to: Some("juno1to".to_string()),
            max_spread: Some(Decimal::percent(1)),
            single: true,
        },
    );
}

#[test]
fn router_cw20_hook_execute_swap_operations_bidir() {
    bidir_roundtrip(
        juno::router::Cw20HookMsg::ExecuteSwapOperations {
            operations: vec![juno::router::SwapOperation::AstroSwap {
                offer_asset_info: juno::asset::AssetInfo::NativeToken {
                    denom: "ujuno".to_string(),
                },
                ask_asset_info: juno::asset::AssetInfo::NativeToken {
                    denom: "ibc/USDC".to_string(),
                },
            }],
            minimum_receive: Some(Uint128::new(900)),
            to: Some("juno1to".to_string()),
            max_spread: Some(Decimal::percent(1)),
        },
        astroport::router::Cw20HookMsg::ExecuteSwapOperations {
            operations: vec![astroport::router::SwapOperation::AstroSwap {
                offer_asset_info: astroport::asset::AssetInfo::NativeToken {
                    denom: "ujuno".to_string(),
                },
                ask_asset_info: astroport::asset::AssetInfo::NativeToken {
                    denom: "ibc/USDC".to_string(),
                },
            }],
            minimum_receive: Some(Uint128::new(900)),
            to: Some("juno1to".to_string()),
            max_spread: Some(Decimal::percent(1)),
        },
    );
}

#[test]
fn router_query_config_bidir() {
    bidir_roundtrip(
        juno::router::QueryMsg::Config {},
        astroport::router::QueryMsg::Config {},
    );
}

#[test]
fn router_query_simulate_swap_operations_bidir() {
    bidir_roundtrip(
        juno::router::QueryMsg::SimulateSwapOperations {
            offer_amount: Uint128::new(1_000),
            operations: vec![juno::router::SwapOperation::NativeSwap {
                offer_denom: "ujuno".to_string(),
                ask_denom: "ibc/USDC".to_string(),
            }],
        },
        astroport::router::QueryMsg::SimulateSwapOperations {
            offer_amount: Uint128::new(1_000),
            operations: vec![astroport::router::SwapOperation::NativeSwap {
                offer_denom: "ujuno".to_string(),
                ask_denom: "ibc/USDC".to_string(),
            }],
        },
    );
}

#[test]
fn router_query_reverse_simulate_swap_operations_bidir() {
    bidir_roundtrip(
        juno::router::QueryMsg::ReverseSimulateSwapOperations {
            ask_amount: Uint128::new(900),
            operations: vec![juno::router::SwapOperation::NativeSwap {
                offer_denom: "ujuno".to_string(),
                ask_denom: "ibc/USDC".to_string(),
            }],
        },
        astroport::router::QueryMsg::ReverseSimulateSwapOperations {
            ask_amount: Uint128::new(900),
            operations: vec![astroport::router::SwapOperation::NativeSwap {
                offer_denom: "ujuno".to_string(),
                ask_denom: "ibc/USDC".to_string(),
            }],
        },
    );
}

#[test]
fn router_config_response_bidir() {
    bidir_roundtrip(
        juno::router::ConfigResponse {
            astroport_factory: "juno1factory".to_string(),
        },
        astroport::router::ConfigResponse {
            astroport_factory: "juno1factory".to_string(),
        },
    );
}

#[test]
fn router_simulate_swap_operations_response_bidir() {
    bidir_roundtrip(
        juno::router::SimulateSwapOperationsResponse {
            amount: Uint128::new(900),
        },
        astroport::router::SimulateSwapOperationsResponse {
            amount: Uint128::new(900),
        },
    );
}

#[test]
fn router_swap_response_data_bidir() {
    bidir_roundtrip(
        juno::router::SwapResponseData {
            return_amount: Uint128::new(900),
        },
        astroport::router::SwapResponseData {
            return_amount: Uint128::new(900),
        },
    );
}

// ============================================================
// incentives
// ============================================================
//
// Intentional shim omissions on `incentives::ExecuteMsg`:
//   - UpdateConfig, RemoveRewardFromPool, ClaimOrphanedRewards,
//     UpdateBlockedTokenslist, DeactivatePool, DeactivateBlockedPools,
//     ProposeNewOwner, DropOwnershipProposal, ClaimOwnership.
// These are admin-only and not on the downstream-callable path.
//
// Intentional shim omissions on `incentives::QueryMsg`:
//   - PoolStakers, BlockedTokensList, IsFeeExpected,
//     ExternalRewardSchedules, ListPools. Audit/admin queries — not on
//     the UI critical path.
//
// `incentives::InstantiateMsg` is also NOT mirrored on the shim side
// (instantiation is admin-only and downstream contracts never construct
// one). The reverse leg therefore has no bidir test for InstantiateMsg.
//
// `incentives::ScheduleResponse` (returned by the omitted
// `ExternalRewardSchedules` query) is not mirrored either.
//
// Renamed fields enforced by these tests: `reward_token` (was
// `astro_token` upstream) and `reward_per_second` (was
// `astro_per_second`). The GPL crate has been updated to use the new
// names on the Juno side, so this is a wire-match — but the names are
// load-bearing for UI introspection.

#[test]
fn incentives_input_schedule_bidir() {
    bidir_roundtrip(
        juno::incentives::InputSchedule {
            reward: juno::asset::Asset {
                info: juno::asset::AssetInfo::NativeToken {
                    denom: "ujuno".to_string(),
                },
                amount: Uint128::new(1_000_000),
            },
            duration_periods: 4,
        },
        astroport::incentives::InputSchedule {
            reward: astroport::asset::Asset {
                info: astroport::asset::AssetInfo::NativeToken {
                    denom: "ujuno".to_string(),
                },
                amount: Uint128::new(1_000_000),
            },
            duration_periods: 4,
        },
    );
}

#[test]
fn incentives_execute_setup_pools_bidir() {
    bidir_roundtrip(
        juno::incentives::ExecuteMsg::SetupPools {
            pools: vec![
                (
                    "factory/juno1pool1/astroport/share".to_string(),
                    Uint128::new(5_000),
                ),
                (
                    "factory/juno1pool2/astroport/share".to_string(),
                    Uint128::new(5_000),
                ),
            ],
        },
        astroport::incentives::ExecuteMsg::SetupPools {
            pools: vec![
                (
                    "factory/juno1pool1/astroport/share".to_string(),
                    Uint128::new(5_000),
                ),
                (
                    "factory/juno1pool2/astroport/share".to_string(),
                    Uint128::new(5_000),
                ),
            ],
        },
    );
}

#[test]
fn incentives_execute_claim_rewards_bidir() {
    bidir_roundtrip(
        juno::incentives::ExecuteMsg::ClaimRewards {
            lp_tokens: vec![
                "factory/juno1pool1/astroport/share".to_string(),
                "factory/juno1pool2/astroport/share".to_string(),
            ],
        },
        astroport::incentives::ExecuteMsg::ClaimRewards {
            lp_tokens: vec![
                "factory/juno1pool1/astroport/share".to_string(),
                "factory/juno1pool2/astroport/share".to_string(),
            ],
        },
    );
}

#[test]
fn incentives_execute_deposit_bidir() {
    bidir_roundtrip(
        juno::incentives::ExecuteMsg::Deposit {
            recipient: Some("juno1recipient".to_string()),
        },
        astroport::incentives::ExecuteMsg::Deposit {
            recipient: Some("juno1recipient".to_string()),
        },
    );
}

#[test]
fn incentives_execute_withdraw_bidir() {
    bidir_roundtrip(
        juno::incentives::ExecuteMsg::Withdraw {
            lp_token: "factory/juno1pool/astroport/share".to_string(),
            amount: Uint128::new(100),
        },
        astroport::incentives::ExecuteMsg::Withdraw {
            lp_token: "factory/juno1pool/astroport/share".to_string(),
            amount: Uint128::new(100),
        },
    );
}

#[test]
fn incentives_execute_set_tokens_per_second_bidir() {
    bidir_roundtrip(
        juno::incentives::ExecuteMsg::SetTokensPerSecond {
            amount: Uint128::new(1_000),
        },
        astroport::incentives::ExecuteMsg::SetTokensPerSecond {
            amount: Uint128::new(1_000),
        },
    );
}

#[test]
fn incentives_execute_incentivize_native_bidir() {
    bidir_roundtrip(
        juno::incentives::ExecuteMsg::Incentivize {
            lp_token: "factory/juno1pool/astroport/share".to_string(),
            schedule: juno::incentives::InputSchedule {
                reward: juno::asset::Asset {
                    info: juno::asset::AssetInfo::NativeToken {
                        denom: "factory/juno1project/proj".to_string(),
                    },
                    amount: Uint128::new(1_000_000_000),
                },
                duration_periods: 4,
            },
        },
        astroport::incentives::ExecuteMsg::Incentivize {
            lp_token: "factory/juno1pool/astroport/share".to_string(),
            schedule: astroport::incentives::InputSchedule {
                reward: astroport::asset::Asset {
                    info: astroport::asset::AssetInfo::NativeToken {
                        denom: "factory/juno1project/proj".to_string(),
                    },
                    amount: Uint128::new(1_000_000_000),
                },
                duration_periods: 4,
            },
        },
    );
}

#[test]
fn incentives_execute_incentivize_cw20_bidir() {
    // AUDIT-RELEVANT — cw20-as-reward must remain functional even though
    // the cw20-LP entry point was stripped in P2.5.
    bidir_roundtrip(
        juno::incentives::ExecuteMsg::Incentivize {
            lp_token: "factory/juno1pool/astroport/share".to_string(),
            schedule: juno::incentives::InputSchedule {
                reward: juno::asset::Asset {
                    info: juno::asset::AssetInfo::Token {
                        contract_addr: Addr::unchecked("juno1cw20projectaddr"),
                    },
                    amount: Uint128::new(500_000),
                },
                duration_periods: 8,
            },
        },
        astroport::incentives::ExecuteMsg::Incentivize {
            lp_token: "factory/juno1pool/astroport/share".to_string(),
            schedule: astroport::incentives::InputSchedule {
                reward: astroport::asset::Asset {
                    info: astroport::asset::AssetInfo::Token {
                        contract_addr: Addr::unchecked("juno1cw20projectaddr"),
                    },
                    amount: Uint128::new(500_000),
                },
                duration_periods: 8,
            },
        },
    );
}

#[test]
fn incentives_execute_incentivize_many_bidir() {
    bidir_roundtrip(
        juno::incentives::ExecuteMsg::IncentivizeMany(vec![
            (
                "factory/juno1pool1/astroport/share".to_string(),
                juno::incentives::InputSchedule {
                    reward: juno::asset::Asset {
                        info: juno::asset::AssetInfo::NativeToken {
                            denom: "ujuno".to_string(),
                        },
                        amount: Uint128::new(1_000),
                    },
                    duration_periods: 1,
                },
            ),
            (
                "factory/juno1pool2/astroport/share".to_string(),
                juno::incentives::InputSchedule {
                    reward: juno::asset::Asset {
                        info: juno::asset::AssetInfo::NativeToken {
                            denom: "ujuno".to_string(),
                        },
                        amount: Uint128::new(2_000),
                    },
                    duration_periods: 2,
                },
            ),
        ]),
        astroport::incentives::ExecuteMsg::IncentivizeMany(vec![
            (
                "factory/juno1pool1/astroport/share".to_string(),
                astroport::incentives::InputSchedule {
                    reward: astroport::asset::Asset {
                        info: astroport::asset::AssetInfo::NativeToken {
                            denom: "ujuno".to_string(),
                        },
                        amount: Uint128::new(1_000),
                    },
                    duration_periods: 1,
                },
            ),
            (
                "factory/juno1pool2/astroport/share".to_string(),
                astroport::incentives::InputSchedule {
                    reward: astroport::asset::Asset {
                        info: astroport::asset::AssetInfo::NativeToken {
                            denom: "ujuno".to_string(),
                        },
                        amount: Uint128::new(2_000),
                    },
                    duration_periods: 2,
                },
            ),
        ]),
    );
}

#[test]
fn incentives_query_config_bidir() {
    bidir_roundtrip(
        juno::incentives::QueryMsg::Config {},
        astroport::incentives::QueryMsg::Config {},
    );
}

#[test]
fn incentives_query_deposit_bidir() {
    bidir_roundtrip(
        juno::incentives::QueryMsg::Deposit {
            lp_token: "factory/juno1pool/astroport/share".to_string(),
            user: "juno1user".to_string(),
        },
        astroport::incentives::QueryMsg::Deposit {
            lp_token: "factory/juno1pool/astroport/share".to_string(),
            user: "juno1user".to_string(),
        },
    );
}

#[test]
fn incentives_query_pending_rewards_bidir() {
    bidir_roundtrip(
        juno::incentives::QueryMsg::PendingRewards {
            lp_token: "factory/juno1pool/astroport/share".to_string(),
            user: "juno1user".to_string(),
        },
        astroport::incentives::QueryMsg::PendingRewards {
            lp_token: "factory/juno1pool/astroport/share".to_string(),
            user: "juno1user".to_string(),
        },
    );
}

#[test]
fn incentives_query_reward_info_bidir() {
    bidir_roundtrip(
        juno::incentives::QueryMsg::RewardInfo {
            lp_token: "factory/juno1pool/astroport/share".to_string(),
        },
        astroport::incentives::QueryMsg::RewardInfo {
            lp_token: "factory/juno1pool/astroport/share".to_string(),
        },
    );
}

#[test]
fn incentives_query_pool_info_bidir() {
    bidir_roundtrip(
        juno::incentives::QueryMsg::PoolInfo {
            lp_token: "factory/juno1pool/astroport/share".to_string(),
        },
        astroport::incentives::QueryMsg::PoolInfo {
            lp_token: "factory/juno1pool/astroport/share".to_string(),
        },
    );
}

#[test]
fn incentives_query_active_pools_bidir() {
    bidir_roundtrip(
        juno::incentives::QueryMsg::ActivePools {},
        astroport::incentives::QueryMsg::ActivePools {},
    );
}

#[test]
fn incentives_reward_type_int_bidir() {
    bidir_roundtrip(
        juno::incentives::RewardType::Int(juno::asset::AssetInfo::NativeToken {
            denom: "ujuno".to_string(),
        }),
        astroport::incentives::RewardType::Int(astroport::asset::AssetInfo::NativeToken {
            denom: "ujuno".to_string(),
        }),
    );
}

#[test]
fn incentives_reward_type_ext_bidir() {
    bidir_roundtrip(
        juno::incentives::RewardType::Ext {
            info: juno::asset::AssetInfo::Token {
                contract_addr: Addr::unchecked("juno1cw20reward"),
            },
            next_update_ts: 1_700_000_000,
        },
        astroport::incentives::RewardType::Ext {
            info: astroport::asset::AssetInfo::Token {
                contract_addr: Addr::unchecked("juno1cw20reward"),
            },
            next_update_ts: 1_700_000_000,
        },
    );
}

#[test]
fn incentives_reward_info_int_bidir() {
    bidir_roundtrip(
        juno::incentives::RewardInfo {
            reward: juno::incentives::RewardType::Int(juno::asset::AssetInfo::NativeToken {
                denom: "ujuno".to_string(),
            }),
            rps: Decimal256::from_atomics(1_000u128, 0).unwrap(),
            index: Decimal256::zero(),
            orphaned: Decimal256::zero(),
        },
        astroport::incentives::RewardInfo {
            reward: astroport::incentives::RewardType::Int(
                astroport::asset::AssetInfo::NativeToken {
                    denom: "ujuno".to_string(),
                },
            ),
            rps: Decimal256::from_atomics(1_000u128, 0).unwrap(),
            index: Decimal256::zero(),
            orphaned: Decimal256::zero(),
        },
    );
}

#[test]
fn incentives_reward_info_ext_bidir() {
    bidir_roundtrip(
        juno::incentives::RewardInfo {
            reward: juno::incentives::RewardType::Ext {
                info: juno::asset::AssetInfo::Token {
                    contract_addr: Addr::unchecked("juno1cw20reward"),
                },
                next_update_ts: 1_700_000_000,
            },
            rps: Decimal256::from_atomics(500u128, 0).unwrap(),
            index: Decimal256::from_atomics(42u128, 0).unwrap(),
            orphaned: Decimal256::zero(),
        },
        astroport::incentives::RewardInfo {
            reward: astroport::incentives::RewardType::Ext {
                info: astroport::asset::AssetInfo::Token {
                    contract_addr: Addr::unchecked("juno1cw20reward"),
                },
                next_update_ts: 1_700_000_000,
            },
            rps: Decimal256::from_atomics(500u128, 0).unwrap(),
            index: Decimal256::from_atomics(42u128, 0).unwrap(),
            orphaned: Decimal256::zero(),
        },
    );
}

#[test]
fn incentives_pool_info_response_bidir() {
    bidir_roundtrip(
        juno::incentives::PoolInfoResponse {
            total_lp: Uint128::new(1_000_000),
            rewards: vec![juno::incentives::RewardInfo {
                reward: juno::incentives::RewardType::Int(juno::asset::AssetInfo::NativeToken {
                    denom: "ujuno".to_string(),
                }),
                rps: Decimal256::from_atomics(100u128, 0).unwrap(),
                index: Decimal256::zero(),
                orphaned: Decimal256::zero(),
            }],
            last_update_ts: 1_700_000_000,
        },
        astroport::incentives::PoolInfoResponse {
            total_lp: Uint128::new(1_000_000),
            rewards: vec![astroport::incentives::RewardInfo {
                reward: astroport::incentives::RewardType::Int(
                    astroport::asset::AssetInfo::NativeToken {
                        denom: "ujuno".to_string(),
                    },
                ),
                rps: Decimal256::from_atomics(100u128, 0).unwrap(),
                index: Decimal256::zero(),
                orphaned: Decimal256::zero(),
            }],
            last_update_ts: 1_700_000_000,
        },
    );
}

#[test]
fn incentives_config_bidir() {
    // Config was added by A6. Locks down both the rename
    // (astro_token → reward_token, astro_per_second → reward_per_second)
    // and the wire shape against the GPL crate.
    bidir_roundtrip(
        juno::incentives::Config {
            owner: Addr::unchecked("juno1owner"),
            factory: Addr::unchecked("juno1factory"),
            generator_controller: Some(Addr::unchecked("juno1gaugeadapter")),
            reward_token: juno::asset::AssetInfo::NativeToken {
                denom: "ujuno".to_string(),
            },
            reward_per_second: Uint128::new(1_000),
            total_alloc_points: Uint128::new(10_000),
            guardian: Some(Addr::unchecked("juno1guardian")),
            incentivization_fee_info: Some(juno::incentives::IncentivizationFeeInfo {
                fee_receiver: Addr::unchecked("juno1feereceiver"),
                fee: Coin {
                    denom: "ujuno".to_string(),
                    amount: Uint128::new(100),
                },
            }),
            token_transfer_gas_limit: Some(600_000),
        },
        astroport::incentives::Config {
            owner: Addr::unchecked("juno1owner"),
            factory: Addr::unchecked("juno1factory"),
            generator_controller: Some(Addr::unchecked("juno1gaugeadapter")),
            reward_token: astroport::asset::AssetInfo::NativeToken {
                denom: "ujuno".to_string(),
            },
            reward_per_second: Uint128::new(1_000),
            total_alloc_points: Uint128::new(10_000),
            guardian: Some(Addr::unchecked("juno1guardian")),
            incentivization_fee_info: Some(astroport::incentives::IncentivizationFeeInfo {
                fee_receiver: Addr::unchecked("juno1feereceiver"),
                fee: Coin {
                    denom: "ujuno".to_string(),
                    amount: Uint128::new(100),
                },
            }),
            token_transfer_gas_limit: Some(600_000),
        },
    );
}

#[test]
fn incentives_incentivization_fee_info_bidir() {
    bidir_roundtrip(
        juno::incentives::IncentivizationFeeInfo {
            fee_receiver: Addr::unchecked("juno1feereceiver"),
            fee: Coin {
                denom: "ujuno".to_string(),
                amount: Uint128::new(100),
            },
        },
        astroport::incentives::IncentivizationFeeInfo {
            fee_receiver: Addr::unchecked("juno1feereceiver"),
            fee: Coin {
                denom: "ujuno".to_string(),
                amount: Uint128::new(100),
            },
        },
    );
}
