# 03 — Whitelist decision (ADR D2)

**Status:** decided 2026-05-13. Neutron-strip to vanilla cw1 semantics.

## Problem

`contracts/whitelist` in upstream Astroport v5.13.1 directly depends on
`neutron-sdk` (`Cargo.toml`), and `src/contract.rs:10-11` imports `NeutronMsg`
and `TransferSudoMsg`. The contract's `execute_freeze`,
`execute_update_admins`, and `sudo` entrypoints are generic over `T: CustomMsg`
parameterised to `NeutronMsg`. The contract also ships a `src/ibc.rs` with
Neutron IBC sudo handlers.

This won't compile on Juno's wasmd — Juno doesn't have the Neutron custom
message types in its wasmvm capabilities.

## Why factory wants a whitelist contract

Factory `InstantiateMsg.whitelist_code_id: u64` is **non-optional**. Factory
stores it in `Config` but **never instantiates** from it directly. The
whitelist code_id is used by an external incentive-staking flow we don't
ship in v1.

Permissioned pair-creation uses a separate `whitelist: Option<Vec<String>>`
field on `PairConfig` (set per pair-type at factory instantiate), not the
contract. So the whitelist contract itself is upload-but-don't-instantiate
in v1.

## Options considered

### (a) Neutron-strip — restore vanilla cw1 *(chosen)*

Drop `neutron-sdk` dep. Replace `Response<NeutronMsg>` with `Response`. Drop
`sudo` entrypoint. Drop `NeutronMsg` / `TransferSudoMsg` imports. Drop
`T: CustomMsg` generics on `execute_freeze` / `execute_update_admins`. Delete
`src/ibc.rs` and `mod ibc;` from `src/lib.rs`.

- ~15 LoC delete + a handful of type-signature edits.
- Keeps `astroport-whitelist` artifact name and version lineage (downstream
  tooling that grep-matches contract names doesn't break).
- Audit-able as a pure deletion + de-generic-isation.
- Preserves `cw2::set_contract_version` customization other Astroport
  tooling may expect.

### (b) Replace with stock `cw1-whitelist 1.1` crate

- Drops Astroport's `cw2::set_contract_version` customization.
- Breaks tooling that grep-matches by `astroport-whitelist`.
- Forces upload under a different artifact name.

Worse on every axis than (a).

### (c) Drop entirely; pass `whitelist_code_id: 0` to factory

- Factory `whitelist_code_id` is non-optional. Baking `0` into on-chain
  state is technically legal because factory never instantiates from the
  code_id, but it's a "we'll deal with it later" footgun.
- The day v2 wants whitelist functionality, requires a factory migration.

The day-of cost (~15 LoC) is much smaller than the future migration cost.
Reject.

## Decision

Option (a). Neutron-strip the contract in P0 commit #6. Whitelist contract
ships in the v0.1.0 artifact set, uploaded but never instantiated unless a
future `PairConfig` opts into `permissioned: true`.

## Execution checklist (commit #6)

- [ ] `contracts/whitelist/Cargo.toml` — drop `neutron-sdk = "0.9.0"`.
- [ ] `contracts/whitelist/src/contract.rs` — replace `Response<NeutronMsg>`
      with `Response`. Drop `NeutronMsg` and `TransferSudoMsg` imports. Drop
      `sudo` entry function. Drop `T: CustomMsg` generics on `execute_freeze`
      and `execute_update_admins`.
- [ ] Delete `contracts/whitelist/src/ibc.rs`.
- [ ] `contracts/whitelist/src/lib.rs` — drop `mod ibc;`.
- [ ] `cargo build -p astroport-whitelist` exits 0.
- [ ] `cargo test -p astroport-whitelist` exits 0.

## Open question — permissioned pair creation in v1?

**Resolved 2026-05-13: no.** v1 is permissionless. Anyone can call
`factory.CreatePair { Xyk, ... }`. No `PairConfig` sets `permissioned: true`.
Whitelist contract ships in the v0.1.0 artifact set as forward-compat
infrastructure only — no instantiation in v1.

## Follow-ups (post-v1)

If we ever want to ship the incentive-staking flow, the whitelist contract
needs the cw20-token-locking handlers re-added. That's a v2 design problem,
not v1.
