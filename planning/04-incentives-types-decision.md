# 04 — Incentives types-only retention (ADR D1)

**Status:** decided 2026-05-13. Keep `packages/astroport/src/incentives.rs`
as a types-only module. Do not refactor the factory's import of it.

## Problem

`contracts/factory/src/contract.rs` imports
`astroport::incentives::ExecuteMsg::DeactivatePool`. The factory constructs
this message when a pair is deactivated and the factory has a
`generator_address: Some(_)` configured.

In v1 we ship `generator_address: None` (no incentives, no DEX token, see
`memory/juno-defi-direction.md`). The code path that constructs the
`DeactivatePool` message is unreachable.

But: `packages/astroport/src/incentives.rs` defines the type. If we delete
that module, the factory no longer compiles.

## Options

### (a) Keep `incentives.rs` as types-only *(chosen)*

503 lines of pure `cw_serde` type definitions and constants. No `Deps`,
no `DepsMut`, no storage, no contract logic, no transitive deps beyond
`cosmwasm-std` and `cw20`.

- Invisible from audit perspective (auditor reads "types declaration, no
  state machine to verify").
- Forward-compatible: when v2 ships incentives, no factory edit needed.
- ~6 KB of metadata in the compiled `astroport` package; negligible.

### (b) Refactor factory to drop the import

Two sub-options:

1. Duplicate the `DeactivatePool` type locally in the factory crate. Breaks
   wire compatibility with any downstream tooling that match on the JSON
   shape. Forks the schema.
2. Construct the message as raw JSON `Binary`. Uglier; harder to audit;
   relies on string-formatting being correct rather than the type system.

Both larger diffs than (a) and worse on every axis.

## Decision

Option (a). Keep `packages/astroport/src/incentives.rs` exactly as upstream.
Keep `pub mod incentives;` in `src/lib.rs`. The factory's import line at
`contracts/factory/src/contract.rs:19` stays untouched.

## What changes in the strip

- The `contracts/tokenomics/incentives` *contract* is deleted (it implements
  the incentives state machine; we don't ship it).
- The `astroport::incentives` *types module* in `packages/astroport` stays.

The factory imports `astroport::incentives::ExecuteMsg::DeactivatePool` —
the *type*, not the contract. Type stays; contract goes.

## How to apply this in audit narrative

In `planning/07-audit-scope.md`, the audit-house brief should state:

> The `astroport::incentives` module in `packages/astroport` is retained as
> a wire-type declaration only. The corresponding `tokenomics/incentives`
> contract is not shipped. The factory's single import-site of this type is
> in a `generator_address: Some(_)` branch that is never reached in v1
> deployments (factory is instantiated with `generator_address: None`).
> Treat the import as dead code; treat the type module as a JSON schema
> artifact, not a contract surface.

## Future shape

When v2 brings incentives: ship a new `astroport-incentives` contract built
from upstream Astroport's reference impl, instantiate it, set
`factory.generator_address: Some(addr)`. Factory's existing
`DeactivatePool` import-site activates with no factory change required.
