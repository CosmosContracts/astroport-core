//! MIT-licensed wire-type mirror for `astroport-juno` contracts (factory,
//! pair, router, asset). Downstream CosmWasm contracts that need to construct
//! cross-contract calls into the deployed Astroport-Juno set should depend on
//! this crate, **not** on the GPL-3.0 `astroport` crate — that keeps their
//! own licensing clean.
//!
//! The types here are intentionally re-written (not re-exported) from
//! `packages/astroport`. Wire formats must remain byte-identical to the
//! authoritative types in the GPL crate: a CI drift gate
//! (`scripts/check_juno_types_drift.sh`) fails if the generated JSON schemas
//! disagree.
//!
//! See `planning/02-juno-patches.md` for the rationale and maintenance
//! contract.

pub mod asset;
pub mod factory;
pub mod pair;
pub mod router;
