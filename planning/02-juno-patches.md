# 02 — Juno-specific functional patches (P2)

The total functional patch surface in v0.1.1-juno-rc1, taken against the
v0.1.0-juno-rc0 strip-only baseline. Audit diff B.

## Summary

| Patch | LoC (est.) | Audit weight |
|---|---|---|
| `pool_unpause_at` field + gate in `pair` | ~25 + 150 tests | **High** — new financial-risk surface |
| `astroport-juno-types` MIT shim crate | ~250 + 40 schema + 30 drift-check | Low — wire-type mirror with CI drift gate |
| Factory pair-config defaults | 0 LoC (config-time only) | None — documented in runbook |

## Patch 1: `pool_unpause_at` MEV gate on pair swaps

### Why

The cw-abc graduation flow (memo: `memory/abc-graduation-architecture-astroport.md`)
seeds a new pair atomically as a bonding curve crosses its reserve
threshold. Without a pause window, snipers can frontrun the
graduation block and extract value from the freshly-seeded pool
before retail discovery. The patch lets the seeder request a short
swap-blackout window during which liquidity can be provided but
swaps revert.

For non-graduating pools (the v1 default), `pool_unpause_at: None`
is the wire-default and the gate is bypassed. No behavioral change
vs upstream Astroport for normal pair creation.

### Gate site discovery

Upstream Astroport v5.13.1 routes both swap entry points through a
single `swap()` function:

| Entry point | File:line | Path |
|---|---|---|
| Native asset via `ExecuteMsg::Swap` | `contracts/pair/src/contract.rs:249-273` | calls `swap()` directly |
| cw20 token via `ExecuteMsg::Receive` → `Cw20HookMsg::Swap` | `contracts/pair/src/contract.rs:286-333` | dispatches into `swap()` after asset-info translation |

The gate therefore lives in **one** place: the top of `swap()`. This
is a smaller surface than the original plan envisioned (which assumed
two parallel gates).

### Wire-protocol entry

The pair's `InstantiateMsg.init_params: Option<Binary>` is already
the canonical configuration channel; upstream decodes it as
`XYKPoolParams` and extracts `track_asset_balances`. We extend the
same struct with `pool_unpause_at: Option<Timestamp>`. This means:

- **No `InstantiateMsg` JSON shape change** — only the inner
  `XYKPoolParams` gets an optional field.
- **Backward-compatible deserialization** — a v0.1.0 caller that
  omits the new field gets `None`, exactly what existing pools
  expect.
- **No factory edits** — factory already passes `init_params`
  through verbatim to pair instantiate.

### Patch sites (file-by-file)

1. **`packages/astroport/src/pair.rs`** (around line 227-234):

   ```rust
   #[cw_serde]
   pub struct XYKPoolParams {
       pub track_asset_balances: Option<bool>,
       /// Optional timestamp; swaps revert with `PoolPaused` until
       /// this time. `None` ⇒ no pause. Set by the graduation flow
       /// (see memory/abc-graduation-architecture-astroport.md).
       /// LP provide/withdraw remain callable during the pause.
       pub pool_unpause_at: Option<Timestamp>,
   }
   ```

2. **`contracts/pair/src/state.rs`** — extend `Config` after the
   `tracker_addr` field:

   ```rust
   /// Optional pause timestamp; if set, swaps revert with
   /// `PoolPaused` until block.time >= pool_unpause_at.
   pub pool_unpause_at: Option<Timestamp>,
   ```

   Add `cosmwasm_std::Timestamp` to the imports.

3. **`contracts/pair/src/error.rs`** — new variant:

   ```rust
   #[error("Pool is paused until {unpause_at}")]
   PoolPaused { unpause_at: Timestamp },
   ```

   Add `Timestamp` to the `cosmwasm_std` import line.

4. **`contracts/pair/src/contract.rs`**:

   - **Imports (top of file):** add `Timestamp` to the `cosmwasm_std::{...}`
     line.
   - **`instantiate()` (around line 70-99):** when decoding
     `XYKPoolParams`, lift the new `pool_unpause_at` field
     alongside `track_asset_balances` and persist it on `Config`.
   - **`swap()` (around line 616-628):** gate, immediately after
     `let mut config = CONFIG.load(deps.storage)?;`:

     ```rust
     if let Some(unpause_at) = config.pool_unpause_at {
         if env.block.time < unpause_at {
             return Err(ContractError::PoolPaused { unpause_at });
         }
     }
     ```

   No edits to `receive_cw20()` (it dispatches through `swap()`),
   `provide_liquidity()`, or `withdraw_liquidity()`. The LP entry
   points stay callable during the pause window so liquidity can be
   seeded before unpause.

### Test plan

Three new integration tests at the end of
`contracts/pair/tests/integration.rs`:

1. **`swap_during_pause_rejects`** — instantiate the pair with
   `init_params: XYKPoolParams { pool_unpause_at: Some(now + 60), .. }`.
   `provide_liquidity` must succeed (LP entry points remain open).
   `execute(ExecuteMsg::Swap { .. })` must fail with
   `ContractError::PoolPaused { unpause_at: <same timestamp> }`.

2. **`provide_and_withdraw_during_pause_succeeds`** — same paused
   instantiation. Two LPs `ProvideLiquidity`; one `WithdrawLiquidity`.
   Both succeed without touching `swap()`. Confirms LP-side
   surface is unaffected.

3. **`unpause_elapses_then_swap_works`** — same paused instantiation.
   `provide_liquidity`. `app.update_block(|b| b.time = b.time.plus_seconds(61))`.
   Swap must succeed and the price-cumulative math must update as
   on a non-paused pool.

Template: the existing native-asset Swap call at
`contracts/pair/tests/integration.rs:1121-1140` shows the funds + msg
shape; mirror it.

### Why this patch is small enough to land cleanly

- **One state field** + **one error variant** + **one gate site** =
  three diff points.
- No new dependencies; uses only `cosmwasm_std::Timestamp` which is
  already in scope across pair.
- Backward-compatible wire format means no factory edits and no
  migration concerns.
- All non-Swap entry points untouched.

The graduation flow that consumes this gate (cw-abc) is out of scope
for this fork; it lives in `dao-contracts`. This contract change
exposes the primitive; the consumer wires it in via
`to_json_binary(&XYKPoolParams { pool_unpause_at: Some(ts), .. })`.

## Patch 2: `astroport-juno-types` MIT shim crate

### Why

`packages/astroport` is GPL-3.0-only. Any downstream CosmWasm contract
that imports `astroport::factory::InstantiateMsg` (etc.) to construct
a `CreatePair` cross-contract call inherits GPL through linkage.

The most concrete downstream consumer is the cw-abc graduation
extension in `dao-contracts` — that crate is Apache-2.0 / BSD-3 and
should stay that way. Exposing the wire types under an MIT-licensed
shim crate lets downstream import only the message shapes they need
without touching the GPL pair/factory/router implementation crates.

### Design

- New workspace member: `packages/astroport_juno_types/`.
- License: MIT (sibling `LICENSE-MIT` file; `Cargo.toml`
  `license = "MIT"`).
- Dependencies: `cosmwasm-schema`, `cosmwasm-std`, `cw20`. **No
  path-dep on `astroport`** — that would re-pull GPL transitively.
- Types mirrored (re-written, not re-exported, so license cleanly
  separates):
  - `astroport::factory::{InstantiateMsg, ExecuteMsg::CreatePair,
    PairConfig, PairType, TrackerConfig}`
  - `astroport::pair::{InstantiateMsg, ExecuteMsg, QueryMsg,
    XYKPoolParams, FeeShareConfig}` (including the v0.1.1
    `pool_unpause_at` field on `XYKPoolParams`)
  - `astroport::router::{ExecuteMsg, QueryMsg, SwapOperation,
    SimulateSwapOperationsResponse, SwapResponseData}`
  - `astroport::asset::{Asset, AssetInfo, PairInfo}`

### Drift defense

Two crates owning the same wire format is a maintenance liability.
The mitigation is a CI gate that proves they don't diverge silently:

- `packages/astroport_juno_types/examples/schema.rs` emits JSON
  schemas for each mirrored message type.
- `scripts/check_juno_types_drift.sh` runs both schema-gen entry
  points (the shim's example and `packages/astroport`'s example)
  and diffs the equivalent JSON files. Any difference fails the
  script.
- New step in `.github/workflows/tests_and_checks.yml` runs the
  drift-check after the existing `cargo test --workspace`. Drift
  fails CI.

### What lives where

| Type | Authoritative impl | Wire mirror |
|---|---|---|
| `factory::ExecuteMsg::CreatePair` | `packages/astroport/src/factory.rs` (GPL) | `astroport_juno_types::factory` (MIT) |
| `pair::XYKPoolParams` | `packages/astroport/src/pair.rs` (GPL) | `astroport_juno_types::pair` (MIT) |
| Pair *behavior* (the swap math) | `contracts/pair/src/contract.rs` (GPL) | **Not mirrored.** Downstream contracts that need pair behavior depend on the contract via Stargate/wasm calls, not via Rust linkage. |

Downstream consumption pattern (in e.g. `dao-contracts`):

```rust
use astroport_juno_types::factory::{ExecuteMsg, PairType};
use astroport_juno_types::pair::XYKPoolParams;

let msg = ExecuteMsg::CreatePair {
    pair_type: PairType::Xyk {},
    asset_infos: vec![reserve, supply],
    init_params: Some(to_json_binary(&XYKPoolParams {
        track_asset_balances: None,
        pool_unpause_at: Some(unpause_ts),
    })?),
};
```

The downstream contract never imports `astroport::*` and stays
license-clean.

## Patch 3: Factory pair-config defaults (no-code, documentation)

For `v1` deployment, the deployer constructs the initial
`PairConfig` with:

```
total_fee_bps: 30,         // 0.3% — industry default
maker_fee_bps: 0,          // no DEX token in v1; LPs keep full commission
is_disabled: false,
is_generator_disabled: true, // no incentives in v1
permissioned: false,       // v1 is permissionless
whitelist: None,
```

This is operational configuration, not source code. Captured here
and in `planning/06-deploy-runbook.md`.

## Out of scope for v0.1.1

- Factory wire-protocol changes — none required; existing
  `init_params: Option<Binary>` carries `pool_unpause_at` through
  to pair instantiate untouched.
- LP-side pause — by design, providing and withdrawing remain
  open during the pause window so the seeder can fund the pool.
- `pool_unpause_at` mutation post-instantiate — there is no
  `UpdateConfig` path for it. Once a pair is created with a
  pause window, the window cannot be extended or shortened. This
  is intentional for v0.1.1; a future v0.2 could expose an
  admin-gated mutation if a use case arises.
- Other pair types (stable, PCL) — they're deferred from the v1
  workspace; the patch only touches XYK.

## How this maps to the audit

Diff A = `v5.13.1..v0.1.0-juno-rc0` (strip-only; subtractive review).
Diff B = `v0.1.0-juno-rc0..v0.1.1-juno-rc1` (this patch set; deep
review). Diff B's reviewable surface:

- ~25 LoC of contract change in `contracts/pair/src/`
- ~150 LoC of new tests
- ~250 LoC of MIT-shim types (mechanical mirror; review for
  exact-wire-match)
- ~70 LoC of CI drift-check + schema example

The MEV gate in `swap()` is the only line that affects fund flow
behavior. Everything else is data/wire-shape.
