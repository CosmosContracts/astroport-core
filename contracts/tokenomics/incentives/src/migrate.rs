#![cfg(not(tarpaulin_include))]

#[cfg(not(feature = "library"))]
use cosmwasm_std::entry_point;
use cosmwasm_std::{DepsMut, Empty, Env, Response};

use crate::error::ContractError;
use crate::instantiate::{CONTRACT_NAME, CONTRACT_VERSION};

/// Migration handler for the Juno fork of `astroport-incentives`.
///
/// **The Juno fork does not accept migrations from upstream Astroport.** P2.5
/// renamed `Config.astro_token → reward_token` and removed
/// `Config.vesting_contract` (see `planning/12-incentives-strip-decisions.md`).
/// Any storage written by upstream `astroport-incentives` v1.0.0–v1.3.0 would
/// fail to deserialize under this fork's `Config` schema; bumping cw2 across
/// that boundary would silently brick every entrypoint on the next
/// `CONFIG.load`.
///
/// The fork-of-record migration path is **prior Juno → current Juno**. The
/// current release is the first Juno tag (`1.4.0-juno`), so no prior Juno
/// version exists yet and every migration is rejected. When a successor Juno
/// release ships, list its predecessor in `SUPPORTED_PRIOR_VERSIONS` below and
/// add any required state surgery before the final `set_contract_version` call.
#[cfg_attr(not(feature = "library"), entry_point)]
pub fn migrate(deps: DepsMut, _env: Env, _msg: Empty) -> Result<Response, ContractError> {
    /// Versions of `astroport-incentives` we know how to migrate forward from.
    /// Empty until a successor Juno release exists; upstream Astroport versions
    /// are intentionally absent because their `Config` shape is incompatible.
    const SUPPORTED_PRIOR_VERSIONS: &[&str] = &[];

    let contract_version = cw2::get_contract_version(deps.storage)?;

    let unsupported = || ContractError::UnsupportedMigrationVersion {
        from_contract: contract_version.contract.clone(),
        from_version: contract_version.version.clone(),
        to_contract: CONTRACT_NAME.to_string(),
        to_version: CONTRACT_VERSION.to_string(),
    };

    if contract_version.contract != CONTRACT_NAME {
        return Err(unsupported());
    }

    if !SUPPORTED_PRIOR_VERSIONS.contains(&contract_version.version.as_str()) {
        return Err(unsupported());
    }

    cw2::set_contract_version(deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;

    Ok(Response::new()
        .add_attribute("previous_contract_name", &contract_version.contract)
        .add_attribute("previous_contract_version", &contract_version.version)
        .add_attribute("new_contract_name", CONTRACT_NAME)
        .add_attribute("new_contract_version", CONTRACT_VERSION))
}
