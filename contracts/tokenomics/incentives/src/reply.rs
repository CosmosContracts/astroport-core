#[cfg(not(feature = "library"))]
use cosmwasm_std::entry_point;
use cosmwasm_std::{DepsMut, Env, Reply, Response, StdError, Storage, SubMsgResult, Uint128};
use cw_storage_plus::{Item, Map};

use astroport::asset::AssetInfo;

use crate::error::ContractError;
use crate::state::ORPHANED_REWARDS;
use crate::utils::{asset_info_key, from_key_to_asset_info};

pub const POST_TRANSFER_REPLY_ID: u64 = 1;

/// First reply id assigned to per-transfer pending entries. Values below this are
/// reserved for legacy / global ids (currently only `POST_TRANSFER_REPLY_ID`).
/// Each outbound reward transfer pulls a fresh id from `NEXT_REPLY_ID`, then
/// registers its `(asset_info, amount)` payload in `PENDING_REWARD_TRANSFERS`
/// so the reply handler can credit `ORPHANED_REWARDS` on failure instead of
/// silently swallowing the loss.
pub const FIRST_DYNAMIC_REPLY_ID: u64 = 1_000;

/// Monotonic counter for transfer-submsg reply ids. Bumped before every transfer
/// emitted by `utils::claim_rewards`, `utils::remove_reward_from_pool`, and
/// `utils::claim_orphaned_rewards`.
pub const NEXT_REPLY_ID: Item<u64> = Item::new("next_reply_id");

/// Transient map of in-flight reward-transfer submessages, keyed by reply id.
/// Stores the `(asset_info_key, amount)` pair so the reply handler can route
/// failed transfers into `ORPHANED_REWARDS` without losing the dispatch context.
/// Entries are inserted right before the submsg is emitted and removed by the
/// reply handler regardless of success/failure (we use `ReplyOn::Always` so we
/// always get a chance to clean up).
pub const PENDING_REWARD_TRANSFERS: Map<u64, (Vec<u8>, Uint128)> =
    Map::new("pending_reward_transfers");

/// Reserve a fresh reply id and register the pending transfer payload.
/// Callers in `utils.rs` use the returned id when constructing the `SubMsg` so
/// the reply handler can correlate the result back to the asset + amount.
pub fn register_pending_transfer(
    storage: &mut dyn Storage,
    reward: &AssetInfo,
    amount: Uint128,
) -> Result<u64, ContractError> {
    let next = NEXT_REPLY_ID
        .may_load(storage)?
        .unwrap_or(FIRST_DYNAMIC_REPLY_ID);
    // Saturating add is sufficient — exhausting a u64 reply-id counter would
    // require ~2^64 transfers; well outside any plausible operational lifetime.
    let bumped = next
        .checked_add(1)
        .ok_or_else(|| StdError::generic_err("reply id counter overflow"))?;
    NEXT_REPLY_ID.save(storage, &bumped)?;
    PENDING_REWARD_TRANSFERS.save(storage, next, &(asset_info_key(reward), amount))?;
    Ok(next)
}

/// The entry point to the contract for processing replies from submessages.
#[cfg_attr(not(feature = "library"), entry_point)]
pub fn reply(deps: DepsMut, _env: Env, msg: Reply) -> Result<Response, ContractError> {
    match msg {
        // Legacy global error id. No payload is tracked for this branch — it is
        // retained for backwards compatibility with messages that may still be
        // in-flight across a migration boundary.
        Reply {
            id: POST_TRANSFER_REPLY_ID,
            result: SubMsgResult::Err(err_msg),
        } => Ok(Response::new().add_attribute("transfer_error", err_msg)),

        // Per-transfer reply with a registered payload. On success we just
        // garbage-collect the pending entry. On failure we route the failed
        // amount into `ORPHANED_REWARDS` keyed by reward asset so the DAO can
        // recover the funds via `ClaimOrphanedRewards`, and we emit a
        // structured `transfer_error` attribute so off-chain monitors can alert.
        Reply { id, result } if id >= FIRST_DYNAMIC_REPLY_ID => {
            let Some((reward_key, amount)) =
                PENDING_REWARD_TRANSFERS.may_load(deps.storage, id)?
            else {
                // Defensive: a payload we did not register. Best-effort log and
                // continue rather than aborting the whole transaction.
                return Ok(Response::new()
                    .add_attribute("transfer_error", "missing_pending_payload")
                    .add_attribute("reply_id", id.to_string()));
            };

            PENDING_REWARD_TRANSFERS.remove(deps.storage, id);

            match result {
                SubMsgResult::Ok(_) => Ok(Response::new()),
                SubMsgResult::Err(err_msg) => {
                    ORPHANED_REWARDS.update::<_, StdError>(
                        deps.storage,
                        &reward_key,
                        |existing| Ok(existing.unwrap_or_default() + amount),
                    )?;

                    // Reconstruct a human-readable asset label via the symmetric
                    // decoder used everywhere else in the contract. Falling back
                    // to a placeholder keeps the orphan credit itself unaffected
                    // by any (extremely unlikely) malformed key on disk.
                    let asset_label = from_key_to_asset_info(reward_key)
                        .map(|info| info.to_string())
                        .unwrap_or_else(|_| "<malformed-asset-key>".to_string());

                    Ok(Response::new()
                        .add_attribute("action", "orphan_failed_transfer")
                        .add_attribute("reply_id", id.to_string())
                        .add_attribute("reward", asset_label)
                        .add_attribute("amount", amount.to_string())
                        .add_attribute("transfer_error", err_msg))
                }
            }
        }

        _ => Err(ContractError::FailedToParseReply {}),
    }
}
