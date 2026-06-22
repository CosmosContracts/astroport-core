//! Wire-type mirror of `astroport::incentives` subset needed by downstream
//! consumers. Identical JSON serialization to the GPL `astroport` crate;
//! the drift gate (`tests/wire_drift.rs`) enforces this.
//!
//! Astroport-Juno strip notes (P2.5) — these affect the shim too:
//!   - `Cw20Msg` (cw20-LP `Receive` hook) is NOT mirrored — the cw20-LP
//!     entry point was stripped on the Juno side.
//!   - `UpdateConfig` drops the upstream `astro_token` and `vesting_contract`
//!     fields; not mirrored anyway (admin-only).
//!   - `astro_token` / `astro_per_second` were renamed to
//!     `reward_token` / `reward_per_second` in `Config` on the Juno side;
//!     the shim mirrors the new names so UIs can introspect the live
//!     `Config` through the shim without dragging in the full GPL crate.
//!
//! Downstream call sites:
//!   - DAO DAO gauge adapter (in dao-contracts/contracts/gauges/) dispatches
//!     `SetupPools` each epoch close.
//!   - Project funding contracts dispatch `Incentivize` to fund external
//!     reward schedules (native or cw20).
//!   - Farm wrapper contracts (or LP-side automation) dispatch `Deposit`,
//!     `Withdraw`, `ClaimRewards`.
//!   - UIs query `Config`, `Deposit`, `PendingRewards`, `PoolInfo`,
//!     `ActivePools`.
//!
//! See planning/11-incentives-and-gauges.md.

use crate::asset::{Asset, AssetInfo};
use cosmwasm_schema::{cw_serde, QueryResponses};
use cosmwasm_std::{Addr, Coin, Decimal256, Uint128};

/// External incentive schedule input. Schedules align to weekly epochs
/// (Mondays UTC); `duration_periods` is the number of full weeks the
/// schedule spans. Valid range: 1..=25 (1 week to ~6 months).
#[cw_serde]
pub struct InputSchedule {
    pub reward: Asset,
    pub duration_periods: u64,
}

/// The incentives execute message surface that downstream consumers may
/// drive. Admin-side mutations (`UpdateConfig`, `RemoveRewardFromPool`,
/// `ClaimOrphanedRewards`, `UpdateBlockedTokenslist`, ownership transfer,
/// `DeactivatePool`, `DeactivateBlockedPools`) are NOT mirrored.
#[cw_serde]
pub enum ExecuteMsg {
    /// Setup generators with their respective allocation points.
    /// Only the owner or the generator controller (= the DAO DAO gauge
    /// adapter) can execute this.
    SetupPools {
        /// The list of (LP token, allocation point) pairs.
        pools: Vec<(String, Uint128)>,
    },
    /// Update rewards and return them to the caller.
    ClaimRewards {
        /// The LP token cw20 address or token-factory denom.
        lp_tokens: Vec<String>,
    },
    /// Stake LP tokens in the generator. LP tokens staked on behalf of
    /// `recipient` if set; otherwise on behalf of the message sender.
    ///
    /// Caller must include a single TF LP coin in `funds`. The cw20-LP
    /// entry point was stripped in P2.5.
    Deposit { recipient: Option<String> },
    /// Withdraw LP tokens from the generator.
    Withdraw {
        /// The LP token cw20 address or token-factory denom.
        lp_token: String,
        /// The amount to withdraw. Must not exceed total staked amount.
        amount: Uint128,
    },
    /// Set a new amount of the internal reward token to distribute per
    /// second. Only the owner can execute this.
    SetTokensPerSecond {
        /// The new amount of the internal reward token per second.
        amount: Uint128,
    },
    /// Incentivize a pool with external rewards. Native or cw20 reward
    /// tokens both supported.
    ///
    /// - **Native rewards:** caller includes funds matching
    ///   `schedule.reward.amount` in `info.funds`.
    /// - **cw20 rewards:** caller pre-grants `cw20::IncreaseAllowance`
    ///   to this contract, then calls `Incentivize`; the contract pulls
    ///   via `cw20::TransferFrom`.
    ///
    /// Caller must also send the per-pool incentivization fee (default 100
    /// ujuno on Astroport-Juno) when registering a new reward token for
    /// the pool — subsequent schedules for the same `(pool, reward_token)`
    /// pair don't incur the fee.
    Incentivize {
        /// The LP token cw20 address or token-factory denom of the pool
        /// to incentivize.
        lp_token: String,
        /// Incentives schedule.
        schedule: InputSchedule,
    },
    /// Same as `Incentivize` but for multiple pools in one call. The
    /// per-pool fee logic still applies per-pool.
    IncentivizeMany(Vec<(String, InputSchedule)>),
}

/// The incentives query surface that UIs and downstream contracts need.
/// Admin-only / paginated audit queries (`PoolStakers`, `BlockedTokensList`,
/// `ExternalRewardSchedules`, `IsFeeExpected`, `ListPools`) are NOT mirrored;
/// they're not on the critical path for any non-UI consumer.
#[cw_serde]
#[derive(QueryResponses)]
pub enum QueryMsg {
    /// Returns the main contract parameters. Mirrored so UIs can read the
    /// renamed `reward_token` / `reward_per_second` fields (was
    /// `astro_token` / `astro_per_second` upstream) without depending on
    /// the GPL `astroport` crate.
    #[returns(Config)]
    Config {},
    /// Returns the LP token amount deposited in a specific generator by a
    /// specific user. Returns 0 if no position exists.
    #[returns(Uint128)]
    Deposit { lp_token: String, user: String },
    /// Returns the amount of rewards that can be claimed by an account
    /// that deposited a specific LP token.
    #[returns(Vec<Asset>)]
    PendingRewards { lp_token: String, user: String },
    /// Returns reward info for a specified LP token.
    #[returns(Vec<RewardInfo>)]
    RewardInfo { lp_token: String },
    /// Returns info about the pool associated with the specified LP token.
    #[returns(PoolInfoResponse)]
    PoolInfo { lp_token: String },
    /// Returns the list of all pools receiving internal (DAO-funded)
    /// emissions, with their alloc_points.
    #[returns(Vec<(String, Uint128)>)]
    ActivePools {},
}

/// Discriminates internal (DAO-funded) reward from external rewards.
#[cw_serde]
#[derive(Eq)]
pub enum RewardType {
    /// Internal (DAO-funded) reward. Was `ASTRO` upstream.
    Int(AssetInfo),
    /// External reward with a schedule.
    Ext {
        info: AssetInfo,
        /// Unix timestamp when the next schedule should start.
        next_update_ts: u64,
    },
}

/// One reward stream attached to a pool (returned by `RewardInfo` and
/// embedded in `PoolInfoResponse`).
#[cw_serde]
pub struct RewardInfo {
    pub reward: RewardType,
    /// Reward tokens per second across the entire pool.
    pub rps: Decimal256,
    /// Last checkpointed reward index per LP token.
    pub index: Decimal256,
    /// Rewards that accrued before any LP was deposited.
    pub orphaned: Decimal256,
}

/// Returned by `PoolInfo`.
#[cw_serde]
pub struct PoolInfoResponse {
    /// Total LP tokens staked in this pool.
    pub total_lp: Uint128,
    /// One entry per active reward stream (one internal + 0..=5 external).
    pub rewards: Vec<RewardInfo>,
    /// Last time reward indexes were updated.
    pub last_update_ts: u64,
}

/// Per-pool fee charged when registering a NEW external reward token on a
/// pool. Mirrored from the GPL crate; field shape is the same.
#[cw_serde]
pub struct IncentivizationFeeInfo {
    /// Fee receiver (contract or wallet).
    pub fee_receiver: Addr,
    /// Native-only fee (no cw20).
    pub fee: Coin,
}

/// Returned by `QueryMsg::Config`. Mirrors the GPL `astroport::incentives::Config`
/// with the Astroport-Juno field renames (`astro_token` → `reward_token`,
/// `astro_per_second` → `reward_per_second`); see module docstring.
#[cw_serde]
pub struct Config {
    /// Address allowed to change contract parameters.
    pub owner: Addr,
    /// The factory address.
    pub factory: Addr,
    /// Contract address which can set active generators and their alloc
    /// points (in addition to `owner`). On Astroport-Juno this is the
    /// DAO DAO gauge adapter.
    pub generator_controller: Option<Addr>,
    /// [`AssetInfo`] of the internal (DAO-funded) reward token. Renamed
    /// from upstream's `astro_token`; for Astroport-Juno this is typically
    /// `AssetInfo::native("ujuno")`. Immutable post-instantiate.
    pub reward_token: AssetInfo,
    /// Total amount of the internal reward token to distribute per second.
    /// Renamed from upstream's `astro_per_second`.
    pub reward_per_second: Uint128,
    /// Total allocation points across all active generators.
    pub total_alloc_points: Uint128,
    /// Guardian address — can add/remove tokens from the blocked list.
    pub guardian: Option<Addr>,
    /// Native fee + receiver for registering new external rewards on a
    /// pool. `None` disables the fee.
    pub incentivization_fee_info: Option<IncentivizationFeeInfo>,
    /// Max gas allowed per external-reward token transfer. `None` means
    /// no limit.
    pub token_transfer_gas_limit: Option<u64>,
}
