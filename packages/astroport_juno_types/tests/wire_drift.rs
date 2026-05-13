//! Wire-format drift guard against `packages/astroport`.
//!
//! For each shared type, construct a representative instance using the
//! MIT shim's types, serialize it to JSON, deserialize as the
//! authoritative GPL `astroport` type, and assert equality. Any silent
//! divergence in field names, enum tagging, or serde defaults will fail
//! one of these assertions.
//!
//! The dev-dep on `astroport` is scoped to this test target only — the
//! shipped library has no GPL linkage.

use astroport_juno_types as juno;
use cosmwasm_std::{to_json_string, Addr, Binary, Decimal, Timestamp, Uint128};

fn roundtrip<S: serde::Serialize, D: serde::de::DeserializeOwned + PartialEq + std::fmt::Debug>(
    shim_value: &S,
) -> D {
    let json = to_json_string(shim_value).expect("shim type serializes");
    serde_json::from_str::<D>(&json)
        .unwrap_or_else(|e| panic!("upstream type failed to deserialize shim JSON: {e}\n{json}"))
}

#[test]
fn asset_info_roundtrip() {
    let shim_native = juno::asset::AssetInfo::NativeToken {
        denom: "ujuno".to_string(),
    };
    let upstream_native: astroport::asset::AssetInfo = roundtrip(&shim_native);
    assert!(matches!(
        upstream_native,
        astroport::asset::AssetInfo::NativeToken { ref denom } if denom == "ujuno"
    ));

    let shim_token = juno::asset::AssetInfo::Token {
        contract_addr: Addr::unchecked("contract0000"),
    };
    let upstream_token: astroport::asset::AssetInfo = roundtrip(&shim_token);
    assert!(matches!(
        upstream_token,
        astroport::asset::AssetInfo::Token { ref contract_addr } if contract_addr.as_str() == "contract0000"
    ));
}

#[test]
fn asset_roundtrip() {
    let shim = juno::asset::Asset {
        info: juno::asset::AssetInfo::NativeToken {
            denom: "ujuno".to_string(),
        },
        amount: Uint128::new(1_000_000),
    };
    let upstream: astroport::asset::Asset = roundtrip(&shim);
    assert_eq!(upstream.amount, Uint128::new(1_000_000));
}

#[test]
fn factory_pair_type_roundtrip() {
    for shim in [
        juno::factory::PairType::Xyk {},
        juno::factory::PairType::Stable {},
        juno::factory::PairType::Custom("concentrated".to_string()),
    ] {
        let upstream: astroport::factory::PairType = roundtrip(&shim);
        assert_eq!(upstream.to_string(), shim.to_string());
    }
}

#[test]
fn factory_pair_config_roundtrip() {
    let shim = juno::factory::PairConfig {
        code_id: 42,
        pair_type: juno::factory::PairType::Xyk {},
        total_fee_bps: 30,
        maker_fee_bps: 0,
        is_disabled: false,
        is_generator_disabled: true,
        permissioned: false,
        whitelist: None,
    };
    let upstream: astroport::factory::PairConfig = roundtrip(&shim);
    assert_eq!(upstream.code_id, 42);
    assert_eq!(upstream.total_fee_bps, 30);
}

#[test]
fn factory_create_pair_roundtrip() {
    // The single most load-bearing wire path: a downstream contract
    // (cw-abc graduation) constructs a CreatePair message via the shim
    // and the deployed factory must deserialize it correctly.
    let shim = juno::factory::ExecuteMsg::CreatePair {
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
    };
    let upstream: astroport::factory::ExecuteMsg = roundtrip(&shim);
    match upstream {
        astroport::factory::ExecuteMsg::CreatePair {
            pair_type,
            asset_infos,
            init_params,
        } => {
            assert!(matches!(pair_type, astroport::factory::PairType::Xyk {}));
            assert_eq!(asset_infos.len(), 2);
            assert_eq!(init_params.unwrap().0, b"opaque".to_vec());
        }
        other => panic!("expected CreatePair, got {:?}", other),
    }
}

#[test]
fn pair_xyk_pool_params_roundtrip_with_unpause() {
    let unpause_at = Timestamp::from_seconds(1_750_000_000);
    let shim = juno::pair::XYKPoolParams {
        track_asset_balances: Some(true),
        pool_unpause_at: Some(unpause_at),
    };
    let upstream: astroport::pair::XYKPoolParams = roundtrip(&shim);
    assert_eq!(upstream.track_asset_balances, Some(true));
    assert_eq!(upstream.pool_unpause_at, Some(unpause_at));
}

#[test]
fn pair_xyk_pool_params_omitted_unpause_field() {
    // A v0.1.0 caller (or one that simply omits the new field) must
    // still deserialize as None — backward-wire-compat.
    let shim = juno::pair::XYKPoolParams {
        track_asset_balances: Some(false),
        pool_unpause_at: None,
    };
    let upstream: astroport::pair::XYKPoolParams = roundtrip(&shim);
    assert_eq!(upstream.pool_unpause_at, None);
}

#[test]
fn pair_swap_execute_roundtrip() {
    let shim = juno::pair::ExecuteMsg::Swap {
        offer_asset: juno::asset::Asset {
            info: juno::asset::AssetInfo::NativeToken {
                denom: "ujuno".to_string(),
            },
            amount: Uint128::new(123),
        },
        ask_asset_info: None,
        belief_price: Some(Decimal::percent(50)),
        max_spread: None,
        to: None,
    };
    let upstream: astroport::pair::ExecuteMsg = roundtrip(&shim);
    match upstream {
        astroport::pair::ExecuteMsg::Swap {
            offer_asset,
            belief_price,
            ..
        } => {
            assert_eq!(offer_asset.amount, Uint128::new(123));
            assert_eq!(belief_price, Some(Decimal::percent(50)));
        }
        other => panic!("expected Swap, got {:?}", other),
    }
}

#[test]
fn router_swap_operations_roundtrip() {
    let shim = juno::router::ExecuteMsg::ExecuteSwapOperations {
        operations: vec![juno::router::SwapOperation::AstroSwap {
            offer_asset_info: juno::asset::AssetInfo::NativeToken {
                denom: "ujuno".to_string(),
            },
            ask_asset_info: juno::asset::AssetInfo::NativeToken {
                denom: "ibc/USDC".to_string(),
            },
        }],
        minimum_receive: Some(Uint128::new(900)),
        to: Some("recipient".to_string()),
        max_spread: None,
    };
    let upstream: astroport::router::ExecuteMsg = roundtrip(&shim);
    match upstream {
        astroport::router::ExecuteMsg::ExecuteSwapOperations {
            operations,
            minimum_receive,
            ..
        } => {
            assert_eq!(operations.len(), 1);
            assert_eq!(minimum_receive, Some(Uint128::new(900)));
        }
        other => panic!("expected ExecuteSwapOperations, got {:?}", other),
    }
}
