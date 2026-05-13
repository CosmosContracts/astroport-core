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
use cosmwasm_std::{to_json_string, Addr, Binary, Decimal, Decimal256, Timestamp, Uint128};

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

// ===== incentives =====

#[test]
fn incentives_setup_pools_roundtrip() {
    // The load-bearing wire path for the DAO DAO gauge adapter — the
    // adapter dispatches this each epoch close.
    let shim = juno::incentives::ExecuteMsg::SetupPools {
        pools: vec![
            (
                "factory/juno1pool1addr/astroport/share".to_string(),
                Uint128::new(5000),
            ),
            (
                "factory/juno1pool2addr/astroport/share".to_string(),
                Uint128::new(5000),
            ),
        ],
    };
    let upstream: astroport::incentives::ExecuteMsg = roundtrip(&shim);
    match upstream {
        astroport::incentives::ExecuteMsg::SetupPools { pools } => {
            assert_eq!(pools.len(), 2);
            assert_eq!(pools[0].1, Uint128::new(5000));
            assert_eq!(pools[1].0, "factory/juno1pool2addr/astroport/share");
        }
        other => panic!("expected SetupPools, got {:?}", other),
    }
}

#[test]
fn incentives_incentivize_native_reward_roundtrip() {
    // A project funds a pool with their native token (e.g. their own
    // token-factory denom). 4-week schedule.
    let shim = juno::incentives::ExecuteMsg::Incentivize {
        lp_token: "factory/juno1pooladdr/astroport/share".to_string(),
        schedule: juno::incentives::InputSchedule {
            reward: juno::asset::Asset {
                info: juno::asset::AssetInfo::NativeToken {
                    denom: "factory/juno1projectaddr/project_token".to_string(),
                },
                amount: Uint128::new(1_000_000_000),
            },
            duration_periods: 4,
        },
    };
    let upstream: astroport::incentives::ExecuteMsg = roundtrip(&shim);
    match upstream {
        astroport::incentives::ExecuteMsg::Incentivize { lp_token, schedule } => {
            assert_eq!(lp_token, "factory/juno1pooladdr/astroport/share");
            assert_eq!(schedule.duration_periods, 4);
            assert_eq!(schedule.reward.amount, Uint128::new(1_000_000_000));
        }
        other => panic!("expected Incentivize, got {:?}", other),
    }
}

#[test]
fn incentives_incentivize_cw20_reward_roundtrip() {
    // A project funds a pool with their cw20 token. AUDIT-RELEVANT — this
    // is the wire shape that survived the cw20-LP strip; cw20-as-reward
    // must remain functional.
    let shim = juno::incentives::ExecuteMsg::Incentivize {
        lp_token: "factory/juno1pooladdr/astroport/share".to_string(),
        schedule: juno::incentives::InputSchedule {
            reward: juno::asset::Asset {
                info: juno::asset::AssetInfo::Token {
                    contract_addr: Addr::unchecked("juno1cw20projectaddr"),
                },
                amount: Uint128::new(500_000),
            },
            duration_periods: 8,
        },
    };
    let upstream: astroport::incentives::ExecuteMsg = roundtrip(&shim);
    match upstream {
        astroport::incentives::ExecuteMsg::Incentivize { schedule, .. } => {
            assert!(matches!(
                schedule.reward.info,
                astroport::asset::AssetInfo::Token { ref contract_addr }
                    if contract_addr.as_str() == "juno1cw20projectaddr"
            ));
        }
        other => panic!("expected Incentivize, got {:?}", other),
    }
}

#[test]
fn incentives_incentivize_many_roundtrip() {
    let shim = juno::incentives::ExecuteMsg::IncentivizeMany(vec![
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
    ]);
    let upstream: astroport::incentives::ExecuteMsg = roundtrip(&shim);
    match upstream {
        astroport::incentives::ExecuteMsg::IncentivizeMany(items) => {
            assert_eq!(items.len(), 2);
            assert_eq!(items[1].1.duration_periods, 2);
        }
        other => panic!("expected IncentivizeMany, got {:?}", other),
    }
}

#[test]
fn incentives_deposit_withdraw_claim_roundtrip() {
    for (shim, label) in [
        (
            juno::incentives::ExecuteMsg::Deposit {
                recipient: Some("juno1recipient".to_string()),
            },
            "Deposit",
        ),
        (
            juno::incentives::ExecuteMsg::Withdraw {
                lp_token: "factory/juno1pool/astroport/share".to_string(),
                amount: Uint128::new(100),
            },
            "Withdraw",
        ),
        (
            juno::incentives::ExecuteMsg::ClaimRewards {
                lp_tokens: vec![
                    "factory/juno1pool1/astroport/share".to_string(),
                    "factory/juno1pool2/astroport/share".to_string(),
                ],
            },
            "ClaimRewards",
        ),
    ] {
        // Each round-trips into upstream — discriminator-tagging on serde
        // is identical.
        let _upstream: astroport::incentives::ExecuteMsg = roundtrip(&shim);
        // Confirm the discriminator label is the same on both sides via
        // JSON inspection.
        let json = to_json_string(&shim).unwrap();
        assert!(
            json.contains(&format!("\"{}\":", to_camel_case(label))),
            "{label} discriminator missing in shim JSON: {json}"
        );
    }
}

fn to_camel_case(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut prev_upper = true;
    for ch in s.chars() {
        if ch.is_uppercase() {
            if !prev_upper && !out.is_empty() {
                out.push('_');
            }
            out.push(ch.to_ascii_lowercase());
            prev_upper = true;
        } else {
            out.push(ch);
            prev_upper = false;
        }
    }
    out
}

#[test]
fn incentives_reward_info_roundtrip() {
    // RewardInfo (internal variant).
    let shim_int = juno::incentives::RewardInfo {
        reward: juno::incentives::RewardType::Int(juno::asset::AssetInfo::NativeToken {
            denom: "ujuno".to_string(),
        }),
        rps: Decimal256::from_atomics(1_000u128, 0).unwrap(),
        index: Decimal256::zero(),
        orphaned: Decimal256::zero(),
    };
    let upstream_int: astroport::incentives::RewardInfo = roundtrip(&shim_int);
    assert!(matches!(
        upstream_int.reward,
        astroport::incentives::RewardType::Int(_)
    ));

    // RewardInfo (external variant).
    let shim_ext = juno::incentives::RewardInfo {
        reward: juno::incentives::RewardType::Ext {
            info: juno::asset::AssetInfo::Token {
                contract_addr: Addr::unchecked("juno1cw20addr"),
            },
            next_update_ts: 1_700_000_000,
        },
        rps: Decimal256::from_atomics(500u128, 0).unwrap(),
        index: Decimal256::from_atomics(42u128, 0).unwrap(),
        orphaned: Decimal256::zero(),
    };
    let upstream_ext: astroport::incentives::RewardInfo = roundtrip(&shim_ext);
    match upstream_ext.reward {
        astroport::incentives::RewardType::Ext { next_update_ts, .. } => {
            assert_eq!(next_update_ts, 1_700_000_000)
        }
        other => panic!("expected Ext variant, got {:?}", other),
    }
}

#[test]
fn incentives_pool_info_response_roundtrip() {
    let shim = juno::incentives::PoolInfoResponse {
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
    };
    let upstream: astroport::incentives::PoolInfoResponse = roundtrip(&shim);
    assert_eq!(upstream.total_lp, Uint128::new(1_000_000));
    assert_eq!(upstream.last_update_ts, 1_700_000_000);
    assert_eq!(upstream.rewards.len(), 1);
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
