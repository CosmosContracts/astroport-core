use cosmwasm_std::{
    CheckedFromRatioError, ConversionOverflowError, OverflowError, StdError, Uint128,
};
use cw_utils::PaymentError;
use thiserror::Error;

use astroport::factory::PairType;
use astroport::incentives::MAX_REWARD_TOKENS;

#[derive(Error, Debug, PartialEq)]
pub enum ContractError {
    #[error("{0}")]
    Std(#[from] StdError),

    #[error("{0}")]
    PaymentError(#[from] PaymentError),

    #[error("{0}")]
    CheckedFromRatioError(#[from] CheckedFromRatioError),

    #[error("{0}")]
    OverflowError(#[from] OverflowError),

    #[error("{0}")]
    ConversionOverflowError(#[from] ConversionOverflowError),

    #[error("Unauthorized")]
    Unauthorized {},

    #[error("Duplicated pool found")]
    DuplicatedPoolFound {},

    #[error("Amount to withdraw {withdraw_amount} exceeds balance {available}")]
    AmountExceedsBalance {
        available: Uint128,
        withdraw_amount: Uint128,
    },

    #[error("User {user} doesn't have position in {lp_token}")]
    PositionDoesntExist { user: String, lp_token: String },

    #[error("Pool {pool} doesn't have {reward} reward")]
    RewardNotFound { pool: String, reward: String },

    #[error("Too many reward tokens in pool {lp_token}. Maximum allowed is {MAX_REWARD_TOKENS}")]
    TooManyRewardTokens { lp_token: String },

    #[error("Incentivization fee {fee} expected as you are trying to add new reward token {new_reward_token} for pool {lp_token}")]
    IncentivizationFeeExpected {
        fee: String,
        lp_token: String,
        new_reward_token: String,
    },

    #[error("Token {token} is blocked")]
    BlockedToken { token: String },

    #[error("Pair type {pair_type} is blocked")]
    BlockedPairType { pair_type: PairType },

    #[error("Failed to parse or process reply message")]
    FailedToParseReply {},

    #[error("No orphaned rewards to claim")]
    NoOrphanedRewards {},

    #[error("Failed to set 0 alloc point for pool {lp_token}")]
    ZeroAllocPoint { lp_token: String },

    #[error("Failed to migrate contract")]
    MigrationError {},

    #[error("Unsupported migration from {from_contract} {from_version} to {to_contract} {to_version}: upstream Astroport state shape is incompatible with the Juno fork (Config.astro_token → reward_token, vesting_contract removed). Re-instantiate instead of migrating.")]
    UnsupportedMigrationVersion {
        from_contract: String,
        from_version: String,
        to_contract: String,
        to_version: String,
    },

    #[error("Sent insufficient reward {reward} for pool {lp_token}")]
    InsuffiicientRewardToken { reward: String, lp_token: String },

    /// Internal invariant: a `PoolInfo` reported `is_active_pool() == true` but the
    /// pool was missing from the `ACTIVE_POOLS` registry. Surfacing the breach as a
    /// typed error (instead of unwrap-panicking) lets monitors alert without halting
    /// the transaction stack in an unrecoverable state.
    #[error("Active pool invariant broken for lp_token {lp_token}: pool reports active but is missing from ACTIVE_POOLS registry")]
    ActivePoolInvariantBroken { lp_token: String },
}
