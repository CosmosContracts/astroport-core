# 01 — Strip list (canonical)

Authoritative table of what stays, what goes, and what's deferred. If this
table and the workspace `[members]` array in `Cargo.toml` disagree, this
table is wrong — fix it.

## Keep (v1)

| Path | Role | Notes |
|---|---|---|
| `contracts/factory` | Pool creation + registry | `whitelist_code_id` non-optional — see whitelist note below. `generator_address: None` in v1 (no incentives). |
| `contracts/pair` | XYK constant-product AMM | Receives `pool_unpause_at` patch in P2. LP token is TokenFactory-native (no cw20 LP code path in v5.13.1). |
| `contracts/router` | Multi-hop swap composition | XYK-only routing in v1. Dev-deps on `pair_concentrated` will be removed in P0. |
| `contracts/whitelist` | cw1-style permissioned-pair gate | Neutron-stripped in P0 (drops `neutron-sdk`, `NeutronMsg`, `sudo` entry, `src/ibc.rs`). See `03-whitelist-decision.md`. |
| `contracts/periphery/native_coin_registry` | Native-denom precision oracle | Required by factory; v1 seeds with `ujuno` + canonical IBC denoms. |
| `contracts/periphery/oracle` | Per-pair TWAP | Uploaded but not auto-instantiated. UI uses for price charts. 1-day hardcoded window. |
| `contracts/periphery/tokenfactory_tracker` | TF snapshot tracker | Uploaded, dormant in v1. Wakes up when a pair sets `track_asset_balances: true`. |
| `packages/astroport` | Wire-type + helper crate | Most modules kept; chain-specific modules removed (see "Pruned modules" below). |
| `packages/astroport_test` | cw-multi-test harness | Injective feature branches stripped. |

## Defer in-tree (excluded from workspace, kept for future re-add)

| Path | Role | Why deferred |
|---|---|---|
| `contracts/pair_stable` | Stableswap (Curve v1 invariant) | v1.1 target. Audit corpus exists upstream. |
| `contracts/pair_concentrated` | PCL (Curve v2-style) | v1.2 target. Capital-efficient but heavy audit. |

Both kept on disk so that re-adding is `git mv` + workspace-member edit +
audit-delta cycle, not a re-fork from upstream.

## Delete entirely

| Path | Why deleted |
|---|---|
| `contracts/pair_concentrated_duality` | Neutron `dex` precompile (Skip Slinky CL); not on Juno. |
| `contracts/pair_supervault_adapter` | Neutron Supervault; not on Juno. |
| `contracts/pair_astro_converter` | Terra burn-address cw20-ASTRO → TF migrator. No ASTRO on Juno. |
| `contracts/pair_xastro` | xASTRO ↔ ASTRO swap. No xASTRO. |
| `contracts/pair_transmuter` | 1:1 constant-sum. Niche. Defer indefinitely. |
| `contracts/pair_xyk_sale_tax` | XYK with sale tax. cw-abc graduation handles meme-launch flow orthogonally. |
| `contracts/pair_concentrated_sale_tax` | PCL with sale tax. Same reason. |
| `contracts/tokenomics/` (entire subtree) | No DEX token in v1 — no incentives, maker fee collector, xASTRO staking, vesting, or xastro_token. |
| `contracts/periphery/astro_converter` | Terra-specific cw20→TF converter. |
| `contracts/periphery/astro_converter_neutron` | Neutron outpost of above. |
| `e2e/` | TypeScript e2e harness bound to localterra-1 + localneutron-1 via feather.js. Juno-incompatible without a full rewrite. |
| `scripts/publish_crates.sh` | Publishes to crates.io under `astroport-*` ownership; not ours. |

## Pruned modules in `packages/astroport`

These `pub mod` declarations and their `.rs` files are removed from `src/lib.rs`
and `src/`:

- `astro_converter`
- `maker`
- `pair_xastro`
- `pair_xyk_sale_tax`
- `pair_concentrated_sale_tax`
- `pair_concentrated_duality`
- `staking`
- `vesting`
- `xastro_token`

**Kept:** `incentives` (types-only, see `04-incentives-types-decision.md`),
`asset`, `common`, `cosmwasm_ext`, `factory`, `mock_querier`, `native_coin_registry`,
`observation`, `oracle`, `pair`, `pair_concentrated` (types only — deferred contract),
`querier`, `router`, `testing`, `token`, `token_factory`, `tokenfactory_tracker`.

## Injective-feature strip

The `injective` feature flag is removed from:
- `packages/astroport/Cargo.toml` + `src/token_factory.rs` (Osmosis-style default path retained — exactly what Juno expects).
- `packages/astroport_test/Cargo.toml` + the stargate / coins modules.
- `contracts/pair/Cargo.toml` (plus the `metadata = { build_variants = ["injective"] }` line).

## Final wasm artifact set

```
astroport_factory.wasm
astroport_pair.wasm
astroport_router.wasm
astroport_native_coin_registry.wasm
astroport_oracle.wasm
astroport_tokenfactory_tracker.wasm
astroport_whitelist.wasm
```

7 wasms. Each must pass `cosmwasm-check --available-capabilities staking,cosmwasm_1_1,cosmwasm_2_0,iterator,stargate`
(no `neutron` capability).
