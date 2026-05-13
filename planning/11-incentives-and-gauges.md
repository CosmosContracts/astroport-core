# Astroport-Juno incentives + gauge integration (P2.5)

**Status:** Plan. Not yet executing. Slotted between rc1 and P4 audit handoff.
**Target tag:** `v0.1.2-juno-rc2`.
**Decision sources:** Jake on 2026-05-13 ("yes, incentives are absolutely
necessary"; "external incentives anyone should be able to incentivize a
pool with their own token").

This document supersedes the v1.1 roadmap entry for incentives — it's now
v1 scope, not v1.1.

## Why this exists

Without an incentives contract, the Juno DAO can vote to add or remove
pairs but has no lever to direct emissions to specific pools. That kills
the bootstrap story on a chain with no DEX token of its own. The fix is
to re-introduce Astroport's incentives contract (deleted in P0 as part
of `contracts/tokenomics/`), strip its xASTRO/maker/vesting dependencies,
and drive its `SetupPools` admin call from a DAO DAO gauge.

External incentives (anyone can fund any pool with any token) is
load-bearing — it's how teams shipping a project on Juno bootstrap
their own pool depth without going through a DAO vote.

## Architecture

Three layers, two already exist:

```
┌─────────────────────────────────────────────────────────────────┐
│  GOVERNANCE UX  (exists — feat/gauges in dao-contracts)         │
│    gauge-orchestrator                                           │
│      weekly epochs; voters allocate weight across "options"     │
│      (options = LP-token denoms / pair addresses)               │
│      reads StakeChangeHook from dao-voting-juno-staked          │
└────────────────────────┬────────────────────────────────────────┘
                         │ at epoch close: orchestrator queries
                         │ adapter.SampleGaugeMsgs(selected)
                         │ → DAO core executes returned msgs
                         ▼
┌─────────────────────────────────────────────────────────────────┐
│  ADAPTER  (new — dao-astroport-incentives-adapter)              │
│    mirrors gauge-budget-allocator's shape                       │
│    SampleGaugeMsgs → Vec<CosmosMsg> wrapping:                   │
│      WasmMsg::Execute {                                         │
│        contract: <astroport-incentives>,                        │
│        msg: SetupPools { pools: Vec<(String, Uint128)> } }      │
└────────────────────────┬────────────────────────────────────────┘
                         │ admin call (adapter is owner of incentives)
                         ▼
┌─────────────────────────────────────────────────────────────────┐
│  PAYOUT  (re-introduce — astroport-incentives, stripped)        │
│    holds DAO-funded ujuno emission budget                       │
│    emits per-second based on SetupPools alloc_points            │
│    LPs accrue PASSIVELY via tokenfactory_tracker (no deposit)   │
│    EXTERNAL: anyone can Incentivize { lp_token, schedule } with │
│              any reward token (native or cw20)                  │
└─────────────────────────────────────────────────────────────────┘
```

The DAO core (which holds the incentives budget) is the only thing
capable of refunding incentives. The gauge adapter only ever calls
`SetupPools` — it never moves funds. Funds flow from DAO core →
incentives in a separate DAO-governance flow (refund proposal).

## Why the gauge fit is clean

Reading `dao-contracts/contracts/gauges/budget-allocator/src/contract.rs:149-165`,
the adapter interface is exactly one query:

```rust
fn sample_gauge_msgs(
    deps: Deps,
    selected: Vec<(String, Decimal)>,  // (option, weight)
) -> StdResult<SampleGaugeMsgsResponse> { ... }
```

`gauge-budget-allocator` translates `selected` into N `BankMsg::Send`
calls. Our adapter translates `selected` into **one** `WasmMsg::Execute`
call carrying a `SetupPools { pools: Vec<(String, Uint128)> }` payload.
Same shape, different dispatch target. ~200 LoC of contract code total.

Reference: `contracts/gauges/budget-allocator/src/contract.rs:149-165`.

## Astroport incentives — re-introduction surface

Recover `contracts/tokenomics/incentives/` from `v5.13.1` via
`git show v5.13.1:contracts/tokenomics/incentives/<path>`. Upstream
contract is 7 source files + tests + assets. Net size after strip
(estimate): ~1,200 LoC (down from ~1,500). License: GPL-3.0
(unchanged; matches the rest of the keep-set).

### What we keep verbatim

- **`SetupPools { pools: Vec<(String, Uint128)> }`** — admin-only;
  this is the call the gauge adapter targets. Sets per-pool alloc_points.
- **TF-tracker-based passive accrual** — LPs hold the TF LP denom in
  their bank balance; incentives queries `tokenfactory_tracker` for
  historical balances; rewards accrue without deposit/withdraw.
- **`Incentivize { lp_token, schedule: InputSchedule }`** —
  permissionless external incentives. Anyone can fund any pool with
  any reward token (native or cw20). Schedule shape:
  `{ reward: Asset, duration_periods }` with `EPOCH_LENGTH = 7 days`,
  `MAX_PERIODS = 25` (~6 months max).
  - **Native reward token**: funder includes `funds: Vec<Coin>` with the
    `Incentivize` call; contract validates the funds match the schedule.
  - **cw20 reward token**: funder pre-grants `cw20::IncreaseAllowance`
    (spender = incentives contract); then calls `Incentivize`; the
    contract pulls via `cw20::TransferFrom`. This is the upstream
    pattern and is preserved verbatim — important for the audit
    corpus to carry over.
  This is the load-bearing feature Jake called out on 2026-05-13.
- **`IncentivizeMany(Vec<(String, InputSchedule)>)`** — batched form
  of `Incentivize`. Same semantics.
- **`RemoveRewardFromPool`** — admin-only; lets the DAO remove a
  reward token from a pool (cleanup after a schedule expires or a
  reward token misbehaves).
- **`ClaimRewards { lp_tokens: Vec<String> }`** — permissionless;
  LPs (or anyone on their behalf) claim accrued rewards across N pools.
- **`ClaimOrphanedRewards`** — admin-only; recover stuck rewards after
  the `TOKEN_TRANSFER_GAS_LIMIT` safety hits.
- **`MAX_REWARD_TOKENS = 5`** per pool — caps concurrent external
  incentive schedules per pool. Prevents per-claim gas blow-up.

### What we strip

- **`InstantiateMsg.astro_token: AssetInfo`** → rename to
  `reward_token: AssetInfo`, default `AssetInfo::NativeToken { denom: "ujuno" }`
  at deploy time. This is the "main" emission asset controlled by the
  gauge.
- **`InstantiateMsg.vesting_contract: String`** → **delete.** Upstream
  pulls each tick's emission from astroport-vesting (which we deleted).
  Replace with: incentives holds the ujuno budget directly in its own
  bank balance; the DAO refunds via `BankMsg::Send` when budget is low.
  Removes one entire contract dep + one external query path. **This is
  the biggest single simplification.**
- **cw20-LP staking flow only** — `ExecuteMsg::Receive(Cw20ReceiveMsg)` +
  the `Cw20Msg::{Deposit, DepositFor}` hook variants + explicit
  `ExecuteMsg::Deposit { recipient }` + `ExecuteMsg::Withdraw { lp_token, amount }`.
  Astroport-Juno LP tokens are TF-only (P0 stance), so the LP-side cw20
  path is dead. Removing it cuts the per-user deposit/withdraw state-machine
  surface and ~20% of `src/execute.rs`.
  - **cw20 dep STAYS** in `Cargo.toml` — it's load-bearing for the
    reward-token path (see below). Removing the cw20 LP flow does not
    let us drop the runtime dep.
  - **What we keep on the cw20 side** (Juno has real cw20 tokens in
    circulation — RAW, NETA, MARBLE, legacy projects — and projects
    must be able to incentivize pools with their own cw20):
    - `Incentivize { lp_token, schedule }` accepts `schedule.reward.info: AssetInfo::Token { contract_addr }`.
      Funder pre-grants allowance via `cw20::IncreaseAllowance`, then
      calls `Incentivize`; the contract pulls via `cw20::TransferFrom`.
      (Upstream's pattern — kept verbatim.)
    - `ClaimRewards` dispatches `cw20::ExecuteMsg::Transfer` outbound
      when the accrued reward asset is a cw20.
    - `RemoveRewardFromPool` and `ClaimOrphanedRewards` continue to
      dispatch cw20-aware transfers.
- **`UpdateConfig.astro_token` / `vesting_contract` fields** — drop;
  reward token + budget source are now immutable post-instantiate. (If
  a future v2 wants reward-token rotation, that's a migration.)
- **`UpdateBlockedTokenslist`** — admin-only blocklist of reward tokens
  that can't be used as external incentives. Upstream uses this to block
  shitcoins from polluting pool reward sets. **Keep, but rename** — same
  shape, different framing ("which denoms can be used as external
  incentive rewards"). Default blocklist is empty; DAO can add via
  governance.
- **`guardian: Option<String>`** — pauser address that can disable new
  external incentives. **Keep**; bind to DAO core. Useful incident lever.
- **`incentivization_fee_info: Option<IncentivizationFeeInfo>`** — the
  small fee paid to register a new external schedule (anti-spam).
  **Keep, but rename `IncentivizationFeeInfo.fee` to allow native ujuno
  default.** Upstream default fee was ASTRO; ours defaults to
  `Some({ fee_receiver: DAO core, fee: 1 ujuno })` — DAO-configurable.

### Strip surface estimate

- Delete entirely: `ExecuteMsg::Receive(Cw20ReceiveMsg)` +
  `Cw20Msg::{Deposit, DepositFor}` + `ExecuteMsg::Deposit` +
  `ExecuteMsg::Withdraw` + per-user-deposit `UserInfo` state machine
  for the cw20-LP path + `vesting_contract` plumbing + all
  `astro_token`-named references. **cw20 dep stays** (rewards).
  ~350 LoC delete.
- Rename + simplify: `UpdateConfig`, `IncentivizationFeeInfo`,
  `InstantiateMsg`. ~50 LoC touch.
- Generalize reward_token (was hardcoded ASTRO in many places).
  ~30 LoC touch.
- Strip dev-deps: `astroport-vesting`, `astroport-pair-stable`,
  `astro-token-converter`, `proptest` (proptest is heavy; keep only
  if a test uses it). ~60 LoC of `Cargo.toml` + dev-only test file
  surgery.
- Re-write `tests/incentives_integration_tests.rs` to use the
  keep-set (no pair_stable, no vesting), preserving cw20-reward-token
  test coverage. ~200 LoC rewrite.

Net contract source change: ~430 LoC delete + ~80 LoC touch + ~200
LoC test rewrite. Slightly smaller than the strip we did in P0.

## Astroport incentives — additions

Two small additions on top of the strip:

1. **`UpdateBudget { tokens_per_second: Uint128 }`** — admin-only;
   sets the main-emission rate. DAO calls this when refunding the
   contract or when ending the program. Mirrors upstream's
   `set_tokens_per_block` but on a per-second basis.
   ~30 LoC.

2. **`Query::Budget {}`** — returns current `tokens_per_second` +
   reward_token balance + estimated runway. Used by the UI and by
   gov tooling to surface "refund needed in N days" alerts.
   ~20 LoC.

These are pre-existing in upstream under different names; this is
naming/wire alignment, not net new logic.

## External incentives — load-bearing spec

This is the feature Jake specifically called out. Detailed spec:

| Property | Value | Rationale |
|---|---|---|
| Entry point | `ExecuteMsg::Incentivize { lp_token, schedule }` | Upstream shape; permissionless caller |
| Reward token type | Native (incl. IBC, TF) OR cw20 | `InputSchedule.reward.info: AssetInfo` covers both |
| Schedule shape | `{ reward: Asset, duration_periods: u64 }` | 1 ≤ duration_periods ≤ 25 (1 week to ~6 months) |
| Per-pool reward token cap | 5 concurrent schedules per pool | `MAX_REWARD_TOKENS` constant; prevents gas blow-up |
| Spam fee | DAO-configurable; default 1 ujuno | Charged on first schedule for a `(pool, token)` pair; subsequent schedules in same `(pool, token)` are free |
| Fee destination | DAO-configurable; default DAO core | `IncentivizationFeeInfo.fee_receiver` |
| Reward delivery (native) | Funder includes `info.funds` matching `schedule.reward.amount` | Standard CosmWasm native-funds-attached pattern |
| Reward delivery (cw20) | Funder pre-grants `cw20::IncreaseAllowance(spender=incentives, amount=schedule.reward.amount)`, then calls `Incentivize` | Contract pulls via `cw20::TransferFrom`. Two-tx UX; same as upstream Astroport |
| Reward claim (native) | `ClaimRewards { lp_tokens }` dispatches `BankMsg::Send` | TOKEN_TRANSFER_GAS_LIMIT applies (default ~600k) |
| Reward claim (cw20) | `ClaimRewards { lp_tokens }` dispatches `cw20::ExecuteMsg::Transfer` | TOKEN_TRANSFER_GAS_LIMIT applies |
| Schedule alignment | Aligned to weekly epoch boundary (Mondays UTC) | `EPOCH_LENGTH`, `EPOCHS_START` constants — kept verbatim |
| Cancellation | Not supported (schedule runs to completion) | Allows simple linear accrual math; funder commits at creation |
| Reward blocklist | DAO-maintained list of denied reward tokens | `UpdateBlockedTokenslist` — prevents griefing with worthless or malicious tokens |

The audit firm reads this surface as the highest-attention zone of
the contract:

- **Strip integrity** — did stripping the cw20-LP path (which lived
  alongside the cw20-reward path in the same `execute.rs`) accidentally
  break the cw20-reward path? Concretely: confirm `Incentivize` →
  `cw20::TransferFrom` and `ClaimRewards` → `cw20::Transfer` both still
  fire correctly. The `incentives_external_cw20.rs` integration test
  is the regression gate.
- **cw20 `TransferFrom` failure handling** — if the funder's
  `IncreaseAllowance` was insufficient or revoked between submission
  and execution, does the `Incentivize` call fail atomically (no
  schedule registered) or leave the schedule in a half-registered
  state? Upstream pattern is atomic-rollback via the cw20 sub-message
  reply; verify post-strip.
- **Accounting drift** — concurrent schedules in different reward
  tokens with overlapping windows.
- **Orphaned rewards** — what happens to rewards when an LP withdraws
  liquidity mid-schedule. Upstream uses `ClaimOrphanedRewards` as the
  recovery path; we keep it.
- **Spam-fee bypass** — ensuring the fee is enforced even on the
  multi-call `IncentivizeMany` path.
- **Token-transfer gas limit** — `TOKEN_TRANSFER_GAS_LIMIT` defaults
  to a range that covers cw20 (~150k), native (~90k), and TF-with-hook
  (~300k). With cw20 reward tokens, `ClaimRewards` for an LP holding
  N pools with K cw20 rewards each does N×K outbound `cw20::Transfer`
  calls — verify the per-claim gas budget is sane under realistic load.

## Gauge adapter — new contract surface

New crate at `/workspace/dao-contracts/contracts/gauges/astroport-incentives-adapter/`,
added to `dao-contracts`'s `feat/gauges` branch (or a follow-up
branch — decide at execute time). MIT-licensed (matches dao-contracts'
default). ~250 LoC contract + ~150 LoC tests.

### Structure (mirrors `gauge-budget-allocator`)

```
contracts/gauges/astroport-incentives-adapter/
├─ Cargo.toml
├─ src/
│  ├─ lib.rs
│  ├─ contract.rs        # instantiate / execute / query
│  ├─ msg.rs             # ExecuteMsg / QueryMsg / InstantiateMsg
│  ├─ state.rs           # Config { incentives_addr, total_alloc_points }
│  └─ error.rs
└─ multitest/
   └─ ...                # 8-10 tests
```

### `InstantiateMsg`

```rust
pub struct InstantiateMsg {
    pub owner: String,                    // DAO core (or a sub-DAO)
    pub incentives_addr: String,          // astroport-incentives address
    pub total_alloc_points: Uint128,      // sum of alloc_points across all pools
                                          // (the gauge weights get scaled to this)
    pub allowed_pools: Vec<String>,       // optional whitelist of pool LP-denoms
                                          // that can be options; empty = any
}
```

The `total_alloc_points` is the "denominator" for weight conversion.
`gauge-orchestrator` hands the adapter a `Vec<(option, Decimal)>` where
the decimals sum to ≤ 1.0. The adapter multiplies each weight by
`total_alloc_points` to get the `Uint128` alloc_points value the
incentives contract expects.

### `ExecuteMsg`

```rust
pub enum ExecuteMsg {
    AddPool { lp_token: String },              // admin
    RemovePool { lp_token: String },           // admin
    UpdateTotalAllocPoints { new: Uint128 },   // admin
    UpdateOwnership(cw_ownable::Action),
}
```

No `UpdateIncentivesAddr` — that's a migration if it ever needs to
change. Pinning it makes the audit trivial.

### `QueryMsg::SampleGaugeMsgs`

```rust
fn sample_gauge_msgs(
    deps: Deps,
    selected: Vec<(String, Decimal)>,
) -> StdResult<SampleGaugeMsgsResponse> {
    let cfg = CONFIG.load(deps.storage)?;
    let pools: Vec<(String, Uint128)> = selected
        .into_iter()
        .map(|(lp_token, weight)| {
            let alloc = cfg
                .total_alloc_points
                .checked_mul_floor(weight)
                .map_err(|e| StdError::generic_err(e.to_string()))?;
            Ok((lp_token, alloc))
        })
        .collect::<StdResult<_>>()?;

    let setup_pools_msg = WasmMsg::Execute {
        contract_addr: cfg.incentives_addr.into_string(),
        msg: to_json_binary(&astroport_juno_types::incentives::ExecuteMsg::SetupPools {
            pools,
        })?,
        funds: vec![],
    };

    Ok(SampleGaugeMsgsResponse {
        execute: vec![CosmosMsg::Wasm(setup_pools_msg)],
    })
}
```

Note: uses `astroport_juno_types` (the MIT shim we shipped in rc1),
**not** `astroport` directly. This keeps GPL-3.0 contagion off the
dao-contracts side. We need to extend the MIT shim with the
`incentives::ExecuteMsg::SetupPools` variant during P2.5.

## Deploy + admin model

```
                  ┌────────────────────────────┐
                  │  Juno DAO (cw20-staked or  │
                  │  juno-staked voting module)│
                  │                            │
                  │  Holds: 1M ujuno budget    │
                  └────────┬───────────────────┘
                           │ owns
                           ▼
        ┌──────────────────────────────────────────────────┐
        │  astroport-factory  (owner: DAO core)            │
        │  astroport-incentives                            │
        │    - owner: DAO core                             │
        │    - guardian: DAO core (incident pauser)        │
        │    - reward_token: ujuno                         │
        │    - tokens_per_second: <set by DAO>             │
        │    - generator_controller: <adapter addr>        │
        │      ↑ this is who can call SetupPools           │
        └──────────────────────────────────────────────────┘
                           ▲
                           │ calls SetupPools
        ┌──────────────────┴───────────────────────────────┐
        │  dao-astroport-incentives-adapter                │
        │    - owner: DAO core                             │
        │    - incentives_addr: <above>                    │
        │    - total_alloc_points: 10_000                  │
        └──────────────────┬───────────────────────────────┘
                           ▲
                           │ orchestrator calls SampleGaugeMsgs
                           │ at each weekly epoch close
        ┌──────────────────┴───────────────────────────────┐
        │  gauge-orchestrator  (one gauge: "astroport")    │
        │    - voting module: dao-voting-juno-staked       │
        │    - epoch_size: 7 days                          │
        │    - adapter: <above>                            │
        │    - DAO executes returned WasmMsg::Execute      │
        └──────────────────────────────────────────────────┘
```

The DAO core is the only address that can:
- Refund the incentives contract (`BankMsg::Send`)
- Change `tokens_per_second` (`UpdateBudget`)
- Add or remove pools from the adapter
- Pause new external incentives (via guardian)

The gauge adapter is the only address that can call `SetupPools`. Its
authority is *narrow*: it can only redistribute weights across the
allowed pool set. It cannot change the emission rate, withdraw funds,
or add pools. This is the security model that lets the gauge be
continuous (no per-epoch DAO proposal needed).

## What needs to land in dao-contracts

(In a follow-up branch off `feat/gauges`, or appended to it — decide
when executing.)

1. New crate `contracts/gauges/astroport-incentives-adapter/`.
2. Update the existing gauge-orchestrator README's "writing your
   own adapter" section to reference this as a second concrete example
   (after `gauge-budget-allocator`).
3. Multitest: full flow — orchestrator → adapter → mocked incentives
   contract. (The mocked incentives lives in the adapter's `multitest/`
   directory; we don't want to dev-dep into astroport-core from
   dao-contracts.)

## What needs to land in astroport-core

1. Re-introduce `contracts/tokenomics/incentives/` from `v5.13.1` via
   `git show` (single restore commit, then strip on top).
2. Strip the surface listed under "What we strip" above.
3. Re-wire `Cargo.toml` workspace `[members]` to include
   `contracts/tokenomics/incentives`.
4. Update `astroport-juno-types` (MIT shim) to add:
   - `incentives::ExecuteMsg::SetupPools`
   - `incentives::ExecuteMsg::Incentivize`
   - `incentives::ExecuteMsg::ClaimRewards`
   - `incentives::QueryMsg::PoolInfo`
   - `incentives::QueryMsg::Deposit` (for UI balance display)
   plus wire-drift round-trip tests for each.
5. Extend `integration-tests/` with new test files:
   - `incentives_setup_pools.rs` — admin sets alloc_points; LP holding
     the TF denom accrues rewards over time.
   - `incentives_external_native.rs` — third party funds a pool with a
     non-ujuno **native** (IBC/TF) token; LP accrues that token
     alongside ujuno.
   - `incentives_external_cw20.rs` — third party funds a pool with a
     **cw20** reward token via the allowance + `Incentivize` pattern;
     LP accrues; `ClaimRewards` correctly dispatches `cw20::Transfer`
     outbound. **This test is load-bearing for the audit** — proves
     the cw20-reward-token path survives the LP-side strip intact.
   - `incentives_via_factory_create_pair.rs` — full deploy → CreatePair →
     ProvideLiquidity → SetupPools → ClaimRewards round-trip.
6. Update `planning/06-deploy-runbook.md` (already pending in P5) to
   include the incentives + adapter instantiate sequence.
7. New ADR: `planning/12-incentives-strip-decisions.md` documenting
   the strip choices (why no vesting; why no cw20-LP path; why immutable
   reward_token).

## Slot in master plan

Insert after P3, before P4. Renumber: this is **P2.5**. Updated
sequence:

| Phase | State | Tag |
|---|---|---|
| P0 — Strip-and-green | ✅ | `v0.1.0-juno-rc0` |
| P1 — CI port | ✅ | (folded into rc0) |
| P2 — `pool_unpause_at` + types shim | ✅ | `v0.1.1-juno-rc1` |
| P3 — Integration tests | ✅ | (additive to rc1) |
| **P2.5 — Incentives + gauge adapter** | **Planned** | **`v0.1.2-juno-rc2`** |
| P4 — Audit handoff (3 diffs) | Blocked on P2.5 | — |
| P5 — Deploy infra | Parallel with P4 | — |
| P6 — uni-7 bakeoff | — | — |
| P7 — juno-1 mainnet | — | — |

The audit firm now reads three diffs at P4: mechanical strip
(rc0), pool_unpause_at (rc1), and incentives re-add + strip (rc2).
This is actually better-shaped than two diffs — each is a focused
review with a clear cost basis.

## Effort estimate

| Workstream | LoC | Days |
|---|---|---|
| Re-introduce upstream incentives via `git show` | ~1,500 (restore) | 0.5 |
| Strip xASTRO/vesting/cw20-LP/maker tangle | -500 +80 | 1.5 |
| Re-write integration tests against keep-set | ~400 | 1 |
| Extend `astroport-juno-types` with incentives surface | ~150 | 0.5 |
| Wire-drift tests for new shim types | ~80 | 0.5 |
| New `astroport-incentives-adapter` crate (dao-contracts side) | ~250 + 150 tests | 1.5 |
| `integration-tests/` extension (3 new test files) | ~400 | 1 |
| `planning/12-incentives-strip-decisions.md` ADR | — | 0.5 |
| Update `06-deploy-runbook.md` | — | 0.25 |
| **Total** | **~2,000 net LoC** | **~7 days** |

## Open questions

### Settled 2026-05-13 (by Jake)

1. **Adapter location.** Land in `dao-contracts` on `feat/gauges`
   branch (or its extension). ✅

2. **Spam fee default.** 100 ujuno is fine; tunable via
   `UpdateConfig` post-deploy. Easy to bump later if griefing emerges. ✅

3. **`MAX_REWARD_TOKENS` per pool.** Keep upstream's 5 — preserves
   audit-corpus alignment. Document worst-case `ClaimRewards` gas
   profile in the strip-decisions ADR. ✅

4. **Adapter authority.** Pure `cw_ownable` (single owner = DAO core).
   Refunds infrequent and proposal-gated; staying simple. ✅

### Still open (to resolve during execution)

5. **Audit scope cost basis.** The strip+re-add diff is genuinely
   larger than the `pool_unpause_at` patch. The two-diff audit
   becomes three; per-LoC cost should still favor us (the strip
   diff is mostly *deletions*, which is cheap for any auditor).
   Concrete numbers fall out at P4.

6. **Generator controller pattern.** Upstream incentives has a
   `generator_controller` distinct from `owner` — the controller
   alone can call `SetupPools`. We need to preserve this so the
   adapter is the controller but DAO core is the owner (for refunds
   and config updates). Confirm during the strip pass; if upstream
   doesn't already separate them, add it.

## Cross-references

- Master plan: `planning/00-overview.md`
- Strip decisions: `planning/01-strip-list.md` (incentives removal)
- ADR D1 (incentives types-only): `planning/04-incentives-types-decision.md` —
  this gets superseded by P2.5.
- Gauge state: `memory/gauges-are-cool-branch.md`
- Hack Juno (sibling consumer of the same gauges): `memory/hack-juno-plan-2026-05-12.md`
- DAO DAO architectural primitives: workspace `CLAUDE.md`
- TF tracker (already in keep-set): `contracts/periphery/tokenfactory_tracker/`
