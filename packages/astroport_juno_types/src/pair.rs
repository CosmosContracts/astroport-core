//! Wire-type mirror of `astroport::pair` subset needed by downstream
//! consumers. Includes the Juno-specific `pool_unpause_at` MEV-protection
//! field on `XYKPoolParams`. See planning/02-juno-patches.md.

use crate::asset::{Asset, AssetInfo};
use crate::factory::PairType;
use cosmwasm_schema::{cw_serde, QueryResponses};
use cosmwasm_std::{Addr, Binary, Decimal, Timestamp, Uint128};
use cw20::Cw20ReceiveMsg;

/// XYK pool initialization parameters, decoded by the pair contract from
/// the `InstantiateMsg.init_params: Option<Binary>` field.
#[cw_serde]
pub struct XYKPoolParams {
    /// Snapshot LP-side asset balances per block. Cannot be disabled
    /// after instantiation if enabled.
    pub track_asset_balances: Option<bool>,
    /// MEV-protection pause window. While `block.time < pool_unpause_at`,
    /// swaps revert with `PoolPaused`. LP entry points remain callable
    /// during the pause. None ⇒ no pause enforced.
    #[serde(default)]
    pub pool_unpause_at: Option<Timestamp>,
}

/// Optional fee-share configuration on a pair.
#[cw_serde]
pub struct FeeShareConfig {
    pub bps: u16,
    pub recipient: Addr,
}

/// Pair instantiate parameters. Constructed indirectly through the
/// factory's `CreatePair` — never instantiated by downstream contracts.
#[cw_serde]
pub struct InstantiateMsg {
    pub asset_infos: Vec<AssetInfo>,
    pub token_code_id: u64,
    pub factory_addr: String,
    pub init_params: Option<Binary>,
    pub pair_type: PairType,
}

/// The pair execute message surface downstream consumers may need to
/// drive. Admin-side mutations are not mirrored.
#[cw_serde]
pub enum ExecuteMsg {
    Receive(Cw20ReceiveMsg),
    ProvideLiquidity {
        assets: Vec<Asset>,
        slippage_tolerance: Option<Decimal>,
        auto_stake: Option<bool>,
        receiver: Option<String>,
        min_lp_to_receive: Option<Uint128>,
    },
    WithdrawLiquidity {
        assets: Vec<Asset>,
        min_assets_to_receive: Option<Vec<Asset>>,
    },
    Swap {
        offer_asset: Asset,
        ask_asset_info: Option<AssetInfo>,
        belief_price: Option<Decimal>,
        max_spread: Option<Decimal>,
        to: Option<String>,
    },
}

/// The cw20 receive-hook message — used when swapping a cw20 token in.
#[cw_serde]
pub enum Cw20HookMsg {
    Swap {
        ask_asset_info: Option<AssetInfo>,
        belief_price: Option<Decimal>,
        max_spread: Option<Decimal>,
        to: Option<String>,
    },
    WithdrawLiquidity {
        assets: Vec<Asset>,
        min_assets_to_receive: Option<Vec<Asset>>,
    },
}

/// Pair query surface. The simulation-side queries are what the UI and
/// router consume most heavily.
#[cw_serde]
#[derive(QueryResponses)]
pub enum QueryMsg {
    #[returns(crate::asset::PairInfo)]
    Pair {},
    #[returns(PoolResponse)]
    Pool {},
    #[returns(SimulationResponse)]
    Simulation {
        offer_asset: Asset,
        ask_asset_info: Option<AssetInfo>,
    },
    #[returns(ReverseSimulationResponse)]
    ReverseSimulation {
        offer_asset_info: Option<AssetInfo>,
        ask_asset: Asset,
    },
}

#[cw_serde]
pub struct PoolResponse {
    pub assets: Vec<Asset>,
    pub total_share: Uint128,
}

#[cw_serde]
pub struct SimulationResponse {
    pub return_amount: Uint128,
    pub spread_amount: Uint128,
    pub commission_amount: Uint128,
}

#[cw_serde]
pub struct ReverseSimulationResponse {
    pub offer_amount: Uint128,
    pub spread_amount: Uint128,
    pub commission_amount: Uint128,
}
