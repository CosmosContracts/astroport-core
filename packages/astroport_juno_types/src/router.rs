//! Wire-type mirror of `astroport::router` subset needed by downstream
//! consumers to construct multi-hop swap operations.

use crate::asset::AssetInfo;
use cosmwasm_schema::{cw_serde, QueryResponses};
use cosmwasm_std::{Decimal, Uint128};
use cw20::Cw20ReceiveMsg;

pub const MAX_SWAP_OPERATIONS: usize = 50;

#[cw_serde]
pub struct InstantiateMsg {
    pub astroport_factory: String,
}

#[cw_serde]
pub enum SwapOperation {
    NativeSwap {
        offer_denom: String,
        ask_denom: String,
    },
    AstroSwap {
        offer_asset_info: AssetInfo,
        ask_asset_info: AssetInfo,
    },
}

#[cw_serde]
pub enum ExecuteMsg {
    Receive(Cw20ReceiveMsg),
    ExecuteSwapOperations {
        operations: Vec<SwapOperation>,
        minimum_receive: Option<Uint128>,
        to: Option<String>,
        max_spread: Option<Decimal>,
    },
    ExecuteSwapOperation {
        operation: SwapOperation,
        to: Option<String>,
        max_spread: Option<Decimal>,
        single: bool,
    },
}

#[cw_serde]
pub struct SwapResponseData {
    pub return_amount: Uint128,
}

#[cw_serde]
pub enum Cw20HookMsg {
    ExecuteSwapOperations {
        operations: Vec<SwapOperation>,
        minimum_receive: Option<Uint128>,
        to: Option<String>,
        max_spread: Option<Decimal>,
    },
}

#[cw_serde]
#[derive(QueryResponses)]
pub enum QueryMsg {
    #[returns(ConfigResponse)]
    Config {},
    #[returns(SimulateSwapOperationsResponse)]
    SimulateSwapOperations {
        offer_amount: Uint128,
        operations: Vec<SwapOperation>,
    },
    #[returns(Uint128)]
    ReverseSimulateSwapOperations {
        ask_amount: Uint128,
        operations: Vec<SwapOperation>,
    },
}

#[cw_serde]
pub struct ConfigResponse {
    pub astroport_factory: String,
}

#[cw_serde]
pub struct SimulateSwapOperationsResponse {
    pub amount: Uint128,
}
