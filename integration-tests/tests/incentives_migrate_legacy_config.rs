//! **AUDIT REGRESSION GATE** for the HIGH "migrate handler rubber-stamps
//! incompatible state" finding.
//!
//! Phase-1 fix (Agent A2) replaced the old `Ok(Response::new())` stub in
//! `contracts/tokenomics/incentives/src/migrate.rs` with a guarded handler
//! that:
//!
//! 1. Loads the existing cw2 `ContractVersion` via `cw2::get_contract_version`.
//! 2. Rejects any `contract != CONTRACT_NAME` ("astroport-incentives") with
//!    `ContractError::UnsupportedMigrationVersion`.
//! 3. Rejects any `version` not in `SUPPORTED_PRIOR_VERSIONS` (currently
//!    empty — the Juno fork's tag `1.4.0-juno` is the first release, so
//!    every migration is rejected until a successor Juno version exists).
//!
//! Why this matters: upstream Astroport's `Config` carried `astro_token` /
//! `vesting_contract` fields. P2.5 renamed the former to `reward_token` and
//! dropped the latter (`planning/12-incentives-strip-decisions.md`). A naive
//! migrate would silently bump cw2 across that boundary, leaving stored
//! `Config` JSON that no longer deserializes — every entry point would brick
//! on the next `CONFIG.load`. The fix rejects at the cw2 check, *before*
//! touching the live `Config` storage.
//!
//! This test seeds a contract instance with hand-written legacy cw2 state
//! (mimicking storage written by upstream `astroport-incentives` v1.3.0)
//! plus a hand-written legacy `Config` JSON carrying the stripped fields,
//! then invokes `WasmMsg::Migrate`. It asserts:
//!
//! - Migration is **rejected** (reject-upstream strategy A2 chose).
//! - The rejection cites the unsupported (contract, version) tuple.
//! - The pre-existing legacy `Config` bytes are **untouched** after the
//!   failed migrate (proving no partial-write occurred).
//! - The cw2 contract_info still points at the legacy version (no
//!   `set_contract_version` happened on the failure path).
//!
//! A separate sub-test exercises the contract-name mismatch path
//! ("crates.io:astroport-incentives" — what the upstream crates.io release
//! writes — vs. the fork's `astroport-incentives`).

use cosmwasm_std::{Addr, Empty};

use astroport::asset::AssetInfo;
use astroport::incentives::{InstantiateMsg as IncentivesInstantiateMsg, QueryMsg};
use astroport_test::cw_multi_test::{ContractWrapper, Executor};

use astroport_juno_integration_tests::{deploy_keep_set, mock_app, TestApp, UJUNO};

/// The Juno fork's contract name (matches `CARGO_PKG_NAME` of
/// `contracts/tokenomics/incentives`). Hard-coded here so a rename of
/// the crate trips this test rather than silently passes.
const FORK_CONTRACT_NAME: &str = "astroport-incentives";
/// The fork tag at the time of writing. Bumped in lockstep with
/// `contracts/tokenomics/incentives/Cargo.toml`. Used only for the
/// "post-failure cw2 untouched" assertion.
const FORK_CONTRACT_VERSION: &str = "1.4.0-juno";

/// Mimics the upstream crates.io-published name. Real upstream releases
/// write this exact string to cw2 contract_info — see
/// https://github.com/astroport-fi/hidden_astroport_generator/.
const UPSTREAM_CONTRACT_NAME: &str = "crates.io:astroport-incentives";
const UPSTREAM_VERSION_1_3_0: &str = "1.3.0";

/// Hand-rolled legacy `Config` JSON. This deliberately includes the
/// stripped `astro_token` and `vesting_contract` fields and omits the
/// fork's `reward_token` to prove that the migrate guard fires at the
/// cw2 check, *before* any attempt to deserialize the stored Config
/// against the new shape. If the guard were missing, this storage would
/// brick the contract on the next `CONFIG.load`.
const LEGACY_CONFIG_JSON: &str = r#"{
    "owner": "juno1ydlpvdr3vk9stwh4ml54zwlu5q0fcyh3r6x8u5e8y5xq7vd6vzns8zsrtq",
    "factory": "juno1ydlpvdr3vk9stwh4ml54zwlu5q0fcyh3r6x8u5e8y5xq7vd6vzns8zsrtq",
    "generator_controller": null,
    "astro_token": {"native_token": {"denom": "uastro"}},
    "vesting_contract": "juno1ydlpvdr3vk9stwh4ml54zwlu5q0fcyh3r6x8u5e8y5xq7vd6vzns8zsrtq",
    "astro_per_second": "0",
    "total_alloc_points": "0",
    "guardian": null,
    "incentivization_fee_info": null,
    "token_transfer_gas_limit": null
}"#;

/// cw2 contract_info key. cw2 stores its `ContractVersion` under the
/// raw key `b"contract_info"` (see cw2::CONTRACT). The cw-multi-test
/// `contract_storage_mut` handle is already namespaced to the contract,
/// so writing this key directly clobbers cw2's state.
const CW2_CONTRACT_INFO_KEY: &str = "contract_info";

/// CONFIG storage key for `astroport-incentives`. See
/// `contracts/tokenomics/incentives/src/state.rs` — `CONFIG: Item<Config> =
/// Item::new("config")`.
const CONFIG_KEY: &str = "config";

/// Store the fork's incentives code with the migrate handler wired up.
/// The harness-shared `deploy_incentives_addon` helper in
/// `integration-tests/src/lib.rs` does *not* wire migrate (the v1 deploy
/// runbook never migrates — first release), so we duplicate the wrapper
/// locally and add `.with_migrate_empty(...)`.
fn store_incentives_code_with_migrate(app: &mut TestApp) -> u64 {
    app.store_code(Box::new(
        ContractWrapper::new_with_empty(
            astroport_incentives::execute::execute,
            astroport_incentives::instantiate::instantiate,
            astroport_incentives::query::query,
        )
        .with_reply_empty(astroport_incentives::reply::reply)
        .with_migrate_empty(astroport_incentives::migrate::migrate),
    ))
}

/// Helper: instantiate a fresh incentives contract on top of a deployed
/// keep-set, using the migrate-aware code. Returns (contract_addr, code_id).
fn instantiate_with_migrate(app: &mut TestApp) -> (Addr, u64) {
    let handles = deploy_keep_set(app).unwrap();
    let code_id = store_incentives_code_with_migrate(app);
    let incentives = app
        .instantiate_contract(
            code_id,
            handles.deployer.clone(),
            &IncentivesInstantiateMsg {
                owner: handles.deployer.to_string(),
                factory: handles.factory.to_string(),
                reward_token: AssetInfo::NativeToken {
                    denom: UJUNO.to_string(),
                },
                incentivization_fee_info: None,
                guardian: None,
            },
            &[],
            "astroport-incentives",
            // admin = deployer so migrate is permitted
            Some(handles.deployer.to_string()),
        )
        .unwrap();
    (incentives, code_id)
}

/// Overwrite the cw2 contract_info to look like upstream-written state.
/// `cw2::set_contract_version` writes to the same `Item<ContractVersion>`
/// at key `b"contract_info"` that the migrate handler reads via
/// `cw2::get_contract_version`, so the storage layout is byte-identical to
/// what a real upstream contract would have left behind.
fn write_legacy_cw2(app: &mut TestApp, contract: &Addr, contract_name: &str, version: &str) {
    let mut storage = app.contract_storage_mut(contract);
    cw2::set_contract_version(storage.as_mut(), contract_name, version).unwrap();
}

/// Overwrite the live `CONFIG` storage with the legacy JSON above.
/// This is the smoking gun: if the migrate guard ever stops firing, the
/// next `CONFIG.load()` (any execute or query) deserializes this and
/// bricks the contract.
fn write_legacy_config_json(app: &mut TestApp, contract: &Addr) {
    let mut storage = app.contract_storage_mut(contract);
    storage.set(CONFIG_KEY.as_bytes(), LEGACY_CONFIG_JSON.as_bytes());
}

/// Read raw bytes from a contract storage key. Used to assert that the
/// failed migrate did not mutate state.
fn read_raw(app: &TestApp, contract: &Addr, key: &str) -> Option<Vec<u8>> {
    app.contract_storage(contract).get(key.as_bytes())
}

/// Read the current cw2 ContractVersion via the same path the migrate
/// handler uses (`cw2::get_contract_version`).
fn read_cw2(app: &TestApp, contract: &Addr) -> cw2::ContractVersion {
    cw2::get_contract_version(app.contract_storage(contract).as_ref()).unwrap()
}

// =====================================================================
// Test 1 — upstream version (correct contract name, unsupported version)
// =====================================================================
//
// Models the most dangerous case: upstream's crates.io wheel published
// `astroport-incentives` at a version like 1.3.0. If we naively accepted
// it, the legacy Config JSON below would silently survive into the new
// code path and brick the contract.

#[test]
fn migrate_from_upstream_unsupported_version_rejected() {
    let mut app = mock_app();
    let (incentives, code_id) = instantiate_with_migrate(&mut app);

    // Seed legacy state: cw2 says "astroport-incentives 1.3.0" (note: the
    // fork's CARGO_PKG_NAME match is just "astroport-incentives" — the
    // crates.io-published `crates.io:` prefix is exercised in test 2).
    write_legacy_cw2(&mut app, &incentives, FORK_CONTRACT_NAME, UPSTREAM_VERSION_1_3_0);
    write_legacy_config_json(&mut app, &incentives);

    // Snapshot the legacy bytes so we can prove no partial write.
    let cw2_before = read_raw(&app, &incentives, CW2_CONTRACT_INFO_KEY)
        .expect("cw2 contract_info seeded");
    let config_before = read_raw(&app, &incentives, CONFIG_KEY).expect("legacy config seeded");

    // Migrate to the same code_id (real-world this would be a new wasm
    // upload, but for the rejection path the code identity doesn't
    // matter — the guard fires before any contract logic runs).
    let deployer = app.api().addr_make(astroport_juno_integration_tests::DEPLOYER);
    let err = app
        .migrate_contract(deployer, incentives.clone(), &Empty {}, code_id)
        .expect_err(
            "Migrate from astroport-incentives 1.3.0 MUST be rejected — \
             SUPPORTED_PRIOR_VERSIONS is empty in 1.4.0-juno (first Juno tag)",
        );

    let err_str = format!("{err:#}");
    assert!(
        err_str.contains("Unsupported migration"),
        "rejection should surface UnsupportedMigrationVersion text; got: {err_str}"
    );
    assert!(
        err_str.contains(UPSTREAM_VERSION_1_3_0),
        "rejection should cite the offending source version `1.3.0`; got: {err_str}"
    );

    // Post-failure invariants: no partial write occurred.
    let cw2_after = read_raw(&app, &incentives, CW2_CONTRACT_INFO_KEY)
        .expect("cw2 contract_info still present after failed migrate");
    assert_eq!(
        cw2_before, cw2_after,
        "cw2 contract_info MUST be untouched on rejection — set_contract_version \
         is only called after the guard passes"
    );

    let config_after = read_raw(&app, &incentives, CONFIG_KEY)
        .expect("legacy config still present after failed migrate");
    assert_eq!(
        config_before, config_after,
        "legacy Config bytes MUST be untouched on rejection — no field translation happens"
    );

    // The cw2 still parses, and reports the *legacy* version. This is
    // the property that lets a future Juno-→-Juno migrate path detect
    // and translate this exact storage shape, once SUPPORTED_PRIOR_VERSIONS
    // is populated.
    let cw2 = read_cw2(&app, &incentives);
    assert_eq!(cw2.contract, FORK_CONTRACT_NAME);
    assert_eq!(cw2.version, UPSTREAM_VERSION_1_3_0);
}

// =====================================================================
// Test 2 — upstream contract name (crates.io: prefix) rejected
// =====================================================================
//
// Upstream crates.io releases write `crates.io:astroport-incentives` (the
// `crates.io:` prefix is the cw2 convention for crates.io-published
// contracts). The Juno fork's CARGO_PKG_NAME is just `astroport-incentives`
// (no prefix), so this is a contract-name mismatch and must be rejected by
// the `contract_version.contract != CONTRACT_NAME` branch of the guard.

#[test]
fn migrate_from_upstream_contract_name_rejected() {
    let mut app = mock_app();
    let (incentives, code_id) = instantiate_with_migrate(&mut app);

    // Seed legacy state: cw2 contract name matches what upstream's
    // crates.io build writes.
    write_legacy_cw2(
        &mut app,
        &incentives,
        UPSTREAM_CONTRACT_NAME,
        UPSTREAM_VERSION_1_3_0,
    );
    write_legacy_config_json(&mut app, &incentives);

    let deployer = app.api().addr_make(astroport_juno_integration_tests::DEPLOYER);
    let err = app
        .migrate_contract(deployer, incentives.clone(), &Empty {}, code_id)
        .expect_err(
            "Migrate with mismatched contract name MUST be rejected — the guard's first branch \
             enforces `contract_version.contract == CONTRACT_NAME`",
        );

    let err_str = format!("{err:#}");
    assert!(
        err_str.contains("Unsupported migration"),
        "rejection should surface UnsupportedMigrationVersion text; got: {err_str}"
    );
    assert!(
        err_str.contains(UPSTREAM_CONTRACT_NAME),
        "rejection should cite the offending source contract name; got: {err_str}"
    );

    // cw2 still reports the upstream identity — guard ran before
    // `set_contract_version`, so no rewrite happened.
    let cw2 = read_cw2(&app, &incentives);
    assert_eq!(cw2.contract, UPSTREAM_CONTRACT_NAME);
    assert_eq!(cw2.version, UPSTREAM_VERSION_1_3_0);
}

// =====================================================================
// Test 3 — self-migrate at current fork version also rejected (sanity)
// =====================================================================
//
// SUPPORTED_PRIOR_VERSIONS is empty in 1.4.0-juno. That means even a
// "migrate from the current version to the current version" must be
// rejected — there is no prior Juno tag to translate from. This guards
// against a future regression where someone accidentally lists the
// current version in SUPPORTED_PRIOR_VERSIONS (which would let a
// re-migrate corrupt freshly-instantiated state).

#[test]
fn migrate_from_current_fork_version_rejected() {
    let mut app = mock_app();
    let (incentives, code_id) = instantiate_with_migrate(&mut app);

    // No legacy seeding — the contract is at its freshly-instantiated
    // version (set by `cw2::set_contract_version` in `instantiate`).
    // Sanity-check the precondition.
    let cw2 = read_cw2(&app, &incentives);
    assert_eq!(cw2.contract, FORK_CONTRACT_NAME);
    assert_eq!(cw2.version, FORK_CONTRACT_VERSION);

    let deployer = app.api().addr_make(astroport_juno_integration_tests::DEPLOYER);
    let err = app
        .migrate_contract(deployer, incentives.clone(), &Empty {}, code_id)
        .expect_err(
            "Migrate from the current fork version MUST be rejected while \
             SUPPORTED_PRIOR_VERSIONS is empty",
        );

    let err_str = format!("{err:#}");
    assert!(
        err_str.contains("Unsupported migration"),
        "rejection should surface UnsupportedMigrationVersion text; got: {err_str}"
    );

    // The contract remains fully functional after a failed migrate —
    // Config{} query still resolves (no state was touched).
    let _: astroport::incentives::Config = app
        .wrap()
        .query_wasm_smart(&incentives, &QueryMsg::Config {})
        .expect("Config{} still queryable after the rejected migrate");
}
