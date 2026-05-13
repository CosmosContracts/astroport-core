# Astroport-Juno fork — overview

This `planning/` folder is the canonical record of the work to take
`astroport-fi/astroport-core@v5.13.1` and ship it as the contract layer of a
sovereign Juno DEX. Each document is either a decision-of-record (ADR) or a
runbook.

## What this fork is

Astroport core, stripped down to the contract subset Juno needs, with the
smallest possible patch surface so we can inherit the upstream audit corpus
rather than re-audit from scratch.

Out of scope here: the UI, the strategic rationale, the cost model — those
live in the meta-repo's `memory/` folder (`memory/astroport-juno-deployment-plan.md`,
`memory/juno-defi-direction.md`, `memory/abc-graduation-architecture-astroport.md`).
This folder is contracts-only.

## Status at a glance

| Phase | State |
|---|---|
| P0 — Strip-and-green | ✅ Complete (tagged `v0.1.0-juno-rc0` @ `4f7e63a9`) |
| P1 — CI port | ✅ Complete (folded into rc0 commit `e32053da`) |
| P2 — Patches (`pool_unpause_at`, types shim) | ✅ Complete (tagged `v0.1.1-juno-rc1` @ `84446eda`) |
| P3 — Integration tests | ✅ Complete (5 commits @ `0e225a15..87c256ec`; 4 new tests in `integration-tests/`) |
| **P2.5 — Incentives + gauge adapter** | **Planned** (`planning/11-incentives-and-gauges.md`; targets `v0.1.2-juno-rc2`) |
| P4 — Audit handoff (3 diffs) | Blocked on P2.5 |
| P5 — Deploy infra | Pending |
| P6 — uni-7 bakeoff | Pending |
| P7 — juno-1 mainnet | Pending (DAO gate) |

## The two tags

- **`v0.1.0-juno-rc0`** — strip + Neutron-strip + CI ports. Zero functional
  change to kept contracts. Audit diff A target.
- **`v0.1.1-juno-rc1`** — `pool_unpause_at` patch + `astroport-juno-types`
  MIT shim crate. Audit diff B target.
- **`v0.1.2-juno-rc2`** *(planned, P2.5)* — re-introduce stripped
  `astroport-incentives` from upstream `v5.13.1` + extend MIT shim with
  incentives wire types. Audit diff C target. See `11-incentives-and-gauges.md`.

Three diffs to the AI audit; one mechanical (A), two functional (B, C). See
`07-audit-scope.md`.

## Operating posture (settled 2026-05-13)

- **Audit:** AI audit, not a paid-firm engagement. The two-diff packaging
  still applies — LLM review is cheaper but benefits from the same surface
  segregation (mechanical strip vs functional patch). Upstream Astroport
  audit PDFs are still useful priors but not on the critical path.
- **Permissioned pair creation:** **no.** v1 is permissionless from day one.
  Anyone can call `factory.CreatePair { Xyk, ... }`. The whitelist contract
  is still uploaded (for forward compatibility), but no `PairConfig` sets
  `permissioned: true`. Pair-quality signal layer is a UI concern, not
  contract.
- **Working location:** local only. Trunk lives in
  `/workspace/astroport-core` on branch `juno/main`. No remote push until
  fork-home identity is settled (`memory/juno-ai-github-identity.md`).
- **DEX-ecosystem coordination:** none required. JunoClaw / Junoswap v2
  coordination explicitly out of scope. Astroport-Juno is sovereign.

## Document index

| File | Purpose |
|---|---|
| `00-overview.md` | This file. |
| `01-strip-list.md` | Canonical table of deleted / deferred / kept contracts. |
| `02-juno-patches.md` | (P2) Line-by-line spec for the functional patches. |
| `03-whitelist-decision.md` | ADR D2 — why we Neutron-strip the whitelist instead of replacing with stock cw1. |
| `04-incentives-types-decision.md` | ADR D1 — why `packages/astroport/src/incentives.rs` stays as types-only. |
| `05-toolchain-and-ci.md` | Rust pin, optimizer image, cw-multi-test fork policy, capability flags. |
| `06-deploy-runbook.md` | (P5) uni-7 → juno-1 sequence. |
| `07-audit-scope.md` | (P4) Two-diff handoff + upstream audit-corpus inventory. |
| `08-test-matrix.md` | (P3) Gates per phase. |
| `09-roadmap-v1.1-v1.2.md` | Stable pair re-add, PCL re-add, post-v1 surfaces. |
| `10-open-questions.md` | Running list. |
| `11-incentives-and-gauges.md` | (P2.5) Re-introduce `astroport-incentives`; bind to DAO DAO gauge for community-voted emissions; permissionless external incentives. |
| `12-incentives-strip-decisions.md` | (P2.5) ADR D6 — per-decision rationale for the rc1→rc2 strip (vesting, cw20 axes, naming, MAX_REWARD_TOKENS, generator_controller, bech32 test mode). |

Files marked "(P*)" are stubs until that phase begins.

## Branch + tag conventions

- Trunk: `juno/main`, branched from upstream `v5.13.1` tag.
- Linear history. Semantic-conventional commit messages.
- Local-only until fork-home GitHub identity resolves
  (`memory/juno-ai-github-identity.md`).
