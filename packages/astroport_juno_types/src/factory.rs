//! Wire-type mirror of `astroport::factory` subset needed by downstream
//! consumers to construct `CreatePair` calls and read pair-registry queries.

use crate::asset::{AssetInfo, PairInfo};
use cosmwasm_schema::{cw_serde, QueryResponses};
use cosmwasm_std::{Addr, Binary};
use std::fmt::{Display, Formatter, Result};

/// Available pair types in the factory's registry.
#[derive(Eq)]
#[cw_serde]
pub enum PairType {
    Xyk {},
    Stable {},
    Custom(String),
}

impl Display for PairType {
    fn fmt(&self, fmt: &mut Formatter) -> Result {
        match self {
            PairType::Xyk {} => fmt.write_str("xyk"),
            PairType::Stable {} => fmt.write_str("stable"),
            PairType::Custom(t) => fmt.write_str(format!("custom-{}", t).as_str()),
        }
    }
}

/// A pair-type configuration entry registered in the factory.
#[cw_serde]
pub struct PairConfig {
    pub code_id: u64,
    pub pair_type: PairType,
    pub total_fee_bps: u16,
    pub maker_fee_bps: u16,
    #[serde(default)]
    pub is_disabled: bool,
    #[serde(default)]
    pub is_generator_disabled: bool,
    #[serde(default)]
    pub permissioned: bool,
    #[serde(default)]
    pub whitelist: Option<Vec<String>>,
}

/// Tokenfactory tracker hook configuration. None ⇒ no tracker.
#[cw_serde]
pub struct TrackerConfig {
    pub code_id: u64,
    pub token_factory_addr: String,
}

/// Factory instantiate parameters.
#[cw_serde]
pub struct InstantiateMsg {
    pub pair_configs: Vec<PairConfig>,
    pub token_code_id: u64,
    pub fee_address: Option<String>,
    pub generator_address: Option<String>,
    pub owner: String,
    pub whitelist_code_id: u64,
    pub coin_registry_address: String,
    pub tracker_config: Option<TrackerConfig>,
}

/// The factory execute message surface downstream consumers need. This
/// crate intentionally exposes only the cross-contract-callable variants
/// — admin-side mutations (UpdateConfig, etc.) are not mirrored.
#[cw_serde]
pub enum ExecuteMsg {
    /// Create a new pair of `pair_type` over the two `asset_infos`. Pair
    /// init params are passed verbatim through to the pair contract's
    /// `InstantiateMsg.init_params`.
    CreatePair {
        pair_type: PairType,
        asset_infos: Vec<AssetInfo>,
        init_params: Option<Binary>,
    },
}

/// Factory query surface downstream consumers need.
#[cw_serde]
#[derive(QueryResponses)]
pub enum QueryMsg {
    #[returns(ConfigResponse)]
    Config {},
    #[returns(PairInfo)]
    Pair { asset_infos: Vec<AssetInfo> },
    #[returns(PairsResponse)]
    Pairs {
        start_after: Option<Vec<AssetInfo>>,
        limit: Option<u32>,
    },
    #[returns(FeeInfoResponse)]
    FeeInfo { pair_type: PairType },
}

#[cw_serde]
pub struct ConfigResponse {
    pub owner: Addr,
    pub pair_configs: Vec<PairConfig>,
    pub token_code_id: u64,
    pub fee_address: Option<Addr>,
    pub generator_address: Option<Addr>,
    pub whitelist_code_id: u64,
    pub coin_registry_address: Addr,
}

#[cw_serde]
pub struct PairsResponse {
    pub pairs: Vec<PairInfo>,
}

#[cw_serde]
pub struct FeeInfoResponse {
    pub fee_address: Option<Addr>,
    pub total_fee_bps: u16,
    pub maker_fee_bps: u16,
}
