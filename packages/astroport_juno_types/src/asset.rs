//! Wire-type mirror of `astroport::asset` subset needed by downstream
//! consumers. Identical JSON serialization to the GPL `astroport` crate;
//! the drift gate (`scripts/check_juno_types_drift.sh`) enforces this.

use crate::factory::PairType;
use cosmwasm_schema::cw_serde;
use cosmwasm_std::{Addr, Uint128};

/// An asset (native coin or cw20 token) plus an amount.
#[cw_serde]
pub struct Asset {
    pub info: AssetInfo,
    pub amount: Uint128,
}

/// Discriminates between a native (bank-module) coin and a cw20 token.
#[cw_serde]
#[derive(Eq)]
pub enum AssetInfo {
    /// Non-native (cw20) token by contract address.
    Token { contract_addr: Addr },
    /// Native (bank) token by denom string.
    NativeToken { denom: String },
}

/// The factory's PairInfo response shape.
#[cw_serde]
pub struct PairInfo {
    pub asset_infos: Vec<AssetInfo>,
    pub contract_addr: Addr,
    pub liquidity_token: String,
    pub pair_type: PairType,
}
