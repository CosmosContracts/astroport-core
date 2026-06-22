---
adr: D7
status: in-progress (P4)
---

# AI audit scope — three-diff handoff

This document defines the corpus, lenses, and acceptance criteria for the AI
audit of the astroport-juno fork. Companion to `00-overview.md` (which
declared the two-diff packaging) and `01-strip-list.md` / `02-juno-patches.md` /
`11-incentives-and-gauges.md` / `12-incentives-strip-decisions.md` (which
specify what each diff is supposed to do).

The audit is AI-only — no paid firm. Posture settled in `00-overview.md`
("Operating posture", 2026-05-13).

## The three diffs

| Diff | Tag boundaries | Patch artifact | Purpose | Class |
|---|---|---|---|---|
| **A** | `v5.13.1..v0.1.0-juno-rc0` | `audit/diff-a-keep-set-changes.patch` (+ `diff-a-deleted-files.txt`) | Strip + Neutron-strip + CI ports. | Mechanical — mostly subtractive. |
| **B** | `v0.1.0-juno-rc0..v0.1.1-juno-rc1` | `audit/diff-b-pool-unpause-types-shim.patch` | `pool_unpause_at` patch + MIT `astroport-juno-types` shim crate. | Functional — net additive. |
| **C** | `v0.1.1-juno-rc1..v0.1.2-juno-rc2` | `audit/diff-c-incentives-readd-strip.patch` | Re-add `astroport-incentives` from upstream + cw20-LP / vesting / ASTRO strip + types-shim extension + 4 new integration tests. | Functional — largest surface. |

The diffs against the **deleted contracts** (`pair_concentrated_duality`,
`pair_supervault_adapter`, `pair_astro_converter`, `pair_xastro`, the
`astro_converter*` periphery, tokenomics `{staking, maker, vesting,
xastro_token, incentives@rc0}`, and the deferred `pair_stable` /
`pair_concentrated` / `pair_xyk_sale_tax` / `pair_concentrated_sale_tax` /
`pair_transmuter`) are intentionally excluded from `diff-a-keep-set-changes.patch`
because their full-tree removal is the *purpose* of diff A. Deletions are
audited by inspecting `diff-a-deleted-files.txt` against
`01-strip-list.md` for accidental keeps or missed Neutron-strip targets.

## Audit lenses

For each diff, multiple independent reviewers — different angles, not redundant
checks. A finding survives only if at least one reviewer surfaces it; we don't
need majority agreement, but we do verify before remediation.

### Diff A — mechanical strip

1. **Strip-integrity reviewer.** Compare `diff-a-deleted-files.txt` to
   `01-strip-list.md`. Flag any kept-set contract whose files were
   accidentally deleted. Flag any deferred-set contract whose files were
   accidentally kept *in the workspace `members` list* (the strip-list
   keeps them on disk but excludes from build).
2. **Dangling-reference reviewer.** For each deleted contract, grep the
   kept set for residual imports, type aliases, message-variant references,
   or workspace-Cargo entries. Anything that compiles only because the
   reference is gated behind a feature flag or `cfg` block counts as a
   finding.

### Diff B — `pool_unpause_at` + MIT types shim

3. **Pool-pause correctness reviewer.** Specification is
   `02-juno-patches.md`. Check:
   - `swap()` and `ExecuteSwapOperations` reject when `block.time <
     pool_unpause_at`.
   - `ProvideLiquidity` / `WithdrawLiquidity` remain callable during pause.
   - `pool_unpause_at = None` is a true no-op (path unchanged from upstream).
   - The `PoolPaused` error variant is reachable, not unreachable code.
   - Migration handler treats absent `pool_unpause_at` correctly (legacy
     state — though v1 has no legacy state, so the question is just
     whether the schema is still backwards-load-able).
4. **Wire-drift reviewer (shim).** `astroport-juno-types` is an MIT-licensed
   message-type mirror of the GPL pair/factory/router/incentives. Check that
   every `ExecuteMsg` / `QueryMsg` / `InstantiateMsg` variant in the shim
   matches the corresponding contract types byte-for-byte on the JSON wire
   (field names, optionality, serde tags, enum discriminants). The shim's
   `wire_drift.rs` test gives one automated check; the reviewer should
   spot-verify by reading both sides and flag any case the test doesn't
   cover.

### Diff C — incentives re-add + Juno strip

5. **Incentives strip-correctness reviewer.** Decision matrix is
   `12-incentives-strip-decisions.md`. Check that each decision (cw20-LP
   entry point removed, vesting removed, ASTRO refs removed, generator-controller
   shim, MAX_REWARD_TOKENS, bech32 test mode) is implemented as ADR D6 says,
   and nothing else was incidentally changed. The "AUDIT GATE" integration
   test (`integration-tests/tests/incentives_cw20_reward.rs`) is the
   load-bearing check that cw20 reward tokens still work after the cw20-LP
   strip — verify it actually exercises that path and isn't a tautology.
6. **Incentives security reviewer.** New attack surface analysis:
   - Permissionless `Incentivize` external-reward path — can an attacker
     drain a pool's reward bucket, inflate APR displays, or grief LPs by
     paying micro-rewards in spam tokens?
   - Reward-claim accounting — any double-claim / under-claim drift across
     the new code paths?
   - Generator-controller shim — what does the stripped controller actually
     return / accept, and can a downstream contract call it with bad inputs
     to corrupt incentive state?
   - Cross-contract trust — does the incentives contract validate the
     factory address (or vice versa) when accepting pool-pair signals?

Total: 6 parallel reviewers (1 + 1 for A, 1 + 1 for B, 1 + 1 for C).

## Findings format

Each reviewer returns a structured JSON list of findings:

```json
{
  "diff": "A|B|C",
  "lens": "strip-integrity|dangling-ref|pool-pause|wire-drift|incentives-strip|incentives-security",
  "findings": [
    {
      "severity": "critical|high|medium|low|informational",
      "title": "short claim",
      "location": "path/to/file.rs:LINE or section name",
      "claim": "what is wrong",
      "rationale": "why this is wrong vs. spec or expected behavior",
      "suggested_remediation": "minimal fix"
    }
  ]
}
```

Findings land in `memory/astroport-juno-ai-audit-findings.md` (running
ledger), grouped by diff. Severity ≥ medium becomes a `TASKS.md` entry; ≤ low
becomes a comment in the relevant planning ADR.

## Acceptance criteria (P4 → P5 gate)

P4 is complete when **all** of the following hold:

1. All six reviewer outputs are recorded in
   `memory/astroport-juno-ai-audit-findings.md`.
2. Every finding ≥ medium has been either:
   - Remediated on a new branch off `juno/main`, with a new RC tag
     (`v0.1.3-juno-rc3` or similar), OR
   - Explicitly accepted-as-risk with rationale documented inline.
3. The acceptance + remediation summary is captured in a new ADR
   (`13-audit-resolution.md`).
4. `00-overview.md` "Status at a glance" updated to reflect P4 ✅.

Only then does P5 (deploy infra) unblock.

## Out of scope

- **Upstream Astroport audit corpus** is *not* re-reviewed. Anything
  bit-for-bit identical to v5.13.1 in the kept set inherits the upstream
  audits (Halborn, Oak Security; PDFs to be archived in
  `memory/astroport-audits/` separately). The diffs above are precisely the
  surface where audit inheritance does *not* extend.
- **Performance / gas profile** is not part of this audit. Optimization is
  a v1.1+ concern.
- **UI security review** (XSS, transaction-construction, slippage UX) is
  separate — runs against `apps/dex` in `dao-dao-ui` when that exists.
