//! Emit JSON Schemas for the shim's wire types under
//! `packages/astroport_juno_types/schema/`. Useful for IDE tooling and
//! human review.
//!
//! The drift gate that proves bit-identity vs the authoritative
//! `packages/astroport` types is in `tests/wire_drift.rs` — a Rust
//! round-trip test, not a schema diff.

use cosmwasm_schema::{export_schema, remove_schemas, schema_for};
use std::env::current_dir;
use std::fs::create_dir_all;
use std::path::PathBuf;

fn emit_in(subdir: &str, exports: impl FnOnce(&PathBuf)) {
    let mut out_dir = current_dir().unwrap();
    out_dir.push("packages/astroport_juno_types/schema");
    out_dir.push(subdir);
    create_dir_all(&out_dir).unwrap();
    remove_schemas(&out_dir).unwrap();
    exports(&out_dir);
}

fn main() {
    use astroport_juno_types::asset::{Asset, AssetInfo, PairInfo};
    use astroport_juno_types::factory as f;
    use astroport_juno_types::incentives as i;
    use astroport_juno_types::pair as p;
    use astroport_juno_types::router as r;

    emit_in("asset", |out_dir| {
        export_schema(&schema_for!(Asset), out_dir);
        export_schema(&schema_for!(AssetInfo), out_dir);
        export_schema(&schema_for!(PairInfo), out_dir);
    });

    emit_in("factory", |out_dir| {
        export_schema(&schema_for!(f::PairType), out_dir);
        export_schema(&schema_for!(f::PairConfig), out_dir);
        export_schema(&schema_for!(f::TrackerConfig), out_dir);
        export_schema(&schema_for!(f::InstantiateMsg), out_dir);
        export_schema(&schema_for!(f::ExecuteMsg), out_dir);
        export_schema(&schema_for!(f::QueryMsg), out_dir);
        export_schema(&schema_for!(f::ConfigResponse), out_dir);
        export_schema(&schema_for!(f::PairsResponse), out_dir);
        export_schema(&schema_for!(f::FeeInfoResponse), out_dir);
    });

    emit_in("pair", |out_dir| {
        export_schema(&schema_for!(p::XYKPoolParams), out_dir);
        export_schema(&schema_for!(p::FeeShareConfig), out_dir);
        export_schema(&schema_for!(p::InstantiateMsg), out_dir);
        export_schema(&schema_for!(p::ExecuteMsg), out_dir);
        export_schema(&schema_for!(p::Cw20HookMsg), out_dir);
        export_schema(&schema_for!(p::QueryMsg), out_dir);
        export_schema(&schema_for!(p::PoolResponse), out_dir);
        export_schema(&schema_for!(p::SimulationResponse), out_dir);
        export_schema(&schema_for!(p::ReverseSimulationResponse), out_dir);
    });

    emit_in("router", |out_dir| {
        export_schema(&schema_for!(r::InstantiateMsg), out_dir);
        export_schema(&schema_for!(r::ExecuteMsg), out_dir);
        export_schema(&schema_for!(r::Cw20HookMsg), out_dir);
        export_schema(&schema_for!(r::QueryMsg), out_dir);
        export_schema(&schema_for!(r::SwapOperation), out_dir);
        export_schema(&schema_for!(r::SwapResponseData), out_dir);
        export_schema(&schema_for!(r::SimulateSwapOperationsResponse), out_dir);
        export_schema(&schema_for!(r::ConfigResponse), out_dir);
    });

    emit_in("incentives", |out_dir| {
        export_schema(&schema_for!(i::InputSchedule), out_dir);
        export_schema(&schema_for!(i::ExecuteMsg), out_dir);
        export_schema(&schema_for!(i::QueryMsg), out_dir);
        export_schema(&schema_for!(i::RewardType), out_dir);
        export_schema(&schema_for!(i::RewardInfo), out_dir);
        export_schema(&schema_for!(i::PoolInfoResponse), out_dir);
    });
}
