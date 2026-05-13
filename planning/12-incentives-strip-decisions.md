# ADR D6 — Incentives strip decisions (P2.5)

**Status:** Settled.
**Tag boundary:** This ADR covers the diff `v0.1.1-juno-rc1..v0.1.2-juno-rc2`.
**Decision date:** 2026-05-13.

## Context

Astroport's incentives contract (`contracts/tokenomics/incentives`) was
deleted in P0 as part of the wholesale `contracts/tokenomics/` strip.
P2.5 re-introduces it, with surgery to remove three layers of Astroport
specificity that don't apply to Juno: the cw20-LP staking entry point,
the astroport-vesting contract dependency, and the ASTRO-token naming.

This ADR records the per-decision reasoning. The audit firm should
read this in conjunction with the rc1→rc2 diff.

## D6.1 — No vesting contract

**Decision:** Strip `astroport-vesting` integration entirely. The
incentives contract holds its internal reward token (ujuno on
Juno-mainnet) directly in its own bank balance; the DAO refunds via
`BankMsg::Send` when the runway runs low.

**Upstream behavior:** the incentives contract reads each tick's
emission via `wasm_execute(&config.vesting_contract,
&vesting::ExecuteMsg::Claim { amount: protocol_reward_amount, ... })`.
Vesting drips ASTRO into the incentives contract on an emission
schedule (linear vest, cliff, etc.).

**Why we strip:**

- **No deletion is cheaper than three deletions.** We already deleted
  `contracts/tokenomics/vesting` in P0. Keeping the dep would require
  re-introducing all of vesting + its dev-deps, which themselves
  reference deleted contracts (xastro_token, etc.).
- **Vesting buys ASTRO-specific properties Juno doesn't need.** The
  vesting contract's value-add is the curve (cliff + linear vest) — a
  reasonable shape for a DEX-token launch with a fixed schedule. Juno's
  emissions are budget-allocated DAO governance decisions, not a fixed
  pre-mine — a `tokens_per_second` field on the incentives contract
  itself, set via `SetTokensPerSecond` (existing) + funded via
  `BankMsg::Send` from the DAO, is the right shape.
- **The DAO refund operation is a single proposal.** "Send N ujuno from
  DAO core to incentives contract" is a one-message BankMsg dispatch.
  No new contract to audit, no migration story when the schedule
  shape needs to change.

**Audit implication:** the auditor verifies the strip in `utils.rs`
`claim_rewards()` — upstream's `wasm_execute(&vesting_contract,
vesting::ExecuteMsg::Claim {..})` was replaced with
`config.reward_token.with_balance(amount).into_submsg(user, ...)`.
The submsg dispatches `BankMsg::Send` for native rewards or
`cw20::ExecuteMsg::Transfer` for cw20 rewards via `AssetInfoExt`.

## D6.2 — Strip only the cw20-LP entry point, NOT cw20 entirely

**Decision:** Remove `ExecuteMsg::Receive(Cw20ReceiveMsg)` arm in
`execute.rs` and the `Cw20Msg::{Deposit, DepositFor}` hook variants in
`packages/astroport/src/incentives.rs`. **KEEP** the cw20 runtime dep,
the `Incentivize` flow's cw20::TransferFrom pull, and the
`ClaimRewards` flow's cw20::Transfer push.

**Why the split:** Juno has real cw20 tokens in circulation (RAW, NETA,
MARBLE, legacy projects); projects must be able to incentivize a pool
with their own cw20 token. The two cw20 axes are independent:

| cw20 axis | Astroport-Juno stance |
|---|---|
| cw20 as **LP token** | Stripped (Juno LPs are TF-only per P0) |
| cw20 as **reward token** | Kept verbatim |

**Audit regression gate:** the test
`integration-tests/tests/incentives_external_cw20.rs::cw20_as_external_reward_token_survives_lp_strip`
exercises the full reward-side cw20 flow end-to-end:
1. cw20::IncreaseAllowance(spender=incentives)
2. incentives::Incentivize { reward: AssetInfo::Token { ... } } — contract pulls via cw20::TransferFrom
3. ClaimRewards → cw20::Transfer to LP

If the strip accidentally broke any of these, the test fails. The audit
firm should treat this test as **load-bearing** and not skip it.

## D6.3 — `reward_token` immutable post-instantiate

**Decision:** `UpdateConfig.astro_token: Option<AssetInfo>` and
`UpdateConfig.vesting_contract: Option<String>` removed entirely.
The internal reward token (renamed `astro_token` → `reward_token` on
the Juno side) is fixed at instantiate time.

**Why:** rotating the reward token requires reconciling per-pool
accrual state (`PoolInfo.rewards[0].reward = RewardType::Int(...)`)
across all active pools — that's the deletion the upstream
`update_config` handler did when `astro_token` was set. Going forward
on Juno:

- Rotating the reward token is a rare event (in practice, never within
  a v1 lifetime).
- When it does happen, it should be a deliberate **migration** with
  per-pool reward-index recomputation, not a single config flip.
- Eliminating the runtime mutation removes a state-rewrite path from
  the audit surface.

**Audit implication:** the `UpdateConfig` handler's `astro_token` and
`vesting_contract` blocks are deleted in their entirety. The auditor
should confirm no orphaned references remain (`grep -i astro_token`
on the source tree should return zero matches; same for `vesting_contract`).

## D6.4 — Naming generalization

**Decision:** rename throughout to drop ASTRO-token specificity:

| Before (upstream) | After (Juno) |
|---|---|
| `Config.astro_token: AssetInfo` | `Config.reward_token: AssetInfo` |
| `Config.astro_per_second: Uint128` | `Config.reward_per_second: Uint128` |
| `InstantiateMsg.astro_token` | `InstantiateMsg.reward_token` |
| `PoolInfo::set_astro_rewards()` | `PoolInfo::set_internal_rewards()` |
| `PoolInfo::disable_astro_rewards()` | `PoolInfo::disable_internal_rewards()` |
| Doc comments referencing "ASTRO emissions" | "internal emissions" / "DAO-funded reward" |

**Why:** "ASTRO" is meaningless on Juno (no ASTRO token exists). Names
that refer to the *role* of the token in the contract (internal /
DAO-funded reward) rather than its specific identity are clearer for
future maintainers and for the audit.

**Wire compatibility:** these are NOT wire-compatible changes —
`InstantiateMsg.astro_token` and `InstantiateMsg.reward_token` are
different field names. Downstream consumers (including the
`astroport-juno-types` MIT shim) reference the new names.

## D6.5 — `MAX_REWARD_TOKENS = 5` per pool kept verbatim

**Decision:** Keep upstream's constant. Document the worst-case gas
profile rather than tightening to 3.

**Worst-case gas calculation:** `ClaimRewards { lp_tokens: [<one LP>] }`
with 5 active reward schedules dispatches up to 5 outbound transfers
in the reply chain:
- Each transfer is wrapped in `into_submsg` with
  `Some((ReplyOn::Error, POST_TRANSFER_REPLY_ID))`.
- The gas budget per transfer is set by `Config.token_transfer_gas_limit`,
  default range `400_000..=1_500_000` per
  `astroport::incentives::TOKEN_TRANSFER_GAS_LIMIT`.
- Native bank sends: ~90k gas.
- cw20::Transfer: ~150k gas.
- TF-with-hook: ~300k gas.

Worst case (5 TF-with-hook rewards, gas limit 1.5M each): 7.5M gas
budgeted. CosmWasm tx block limit on Juno is 50M+, so this fits with
substantial headroom even when an LP claims across multiple pools.

**Tightening to 3:** considered. Would reduce worst-case ClaimRewards
gas by ~3M but would reject the legitimate use case where a popular
pool wants to layer 4 short-term incentives from different projects.
The flexibility is worth the audit-corpus alignment with upstream.

**Audit implication:** reviewer should look at the `for reward_info in
self.rewards.iter_mut()` loop in `state.rs::PoolInfo::update_rewards`
and confirm the per-iteration cost is bounded; the
`MAX_REWARD_TOKENS = 5` cap in `state.rs::PoolInfo::incentivize` is
the gate that prevents unbounded growth.

## D6.6 — Pre-existing `generator_controller` separation kept

**Decision:** Preserve upstream's existing distinction between `owner`
and `generator_controller` on the incentives contract:
- `owner`: can call any privileged operation (UpdateConfig, RemoveRewardFromPool,
  SetTokensPerSecond, ClaimOrphanedRewards, etc.).
- `generator_controller`: can ONLY call `SetupPools`. Set via
  `UpdateConfig { generator_controller: Some(...) }`.

For the gauge adapter pattern, this is the security boundary:
- `owner` = DAO core (rare, proposal-gated)
- `generator_controller` = the DAO DAO gauge adapter (frequent, automated, per-epoch)

The adapter can ONLY redistribute alloc_points across the allowed pool
set. It cannot change the emission rate, transfer funds, or add pools.
This is what makes the gauge continuous-governance without a per-epoch
DAO proposal.

**Why we didn't add a new role:** upstream's owner/controller split
is exactly the shape we need. Reusing it preserves audit-corpus
alignment.

**Audit implication:** reviewer should confirm `setup_pools()` in
`execute.rs:259` is the only entry point gated on
`info.sender != config.owner && Some(info.sender) != config.generator_controller`.
Other admin operations check only `info.sender == config.owner`.

## D6.7 — Pre-existing `incentivization_fee_info` kept (default None at instantiate)

**Decision:** Keep upstream's spam-fee mechanism verbatim. Default to
`None` at instantiate; the DAO sets a non-zero fee via `UpdateConfig`
post-deploy (suggested: 100 ujuno, tunable).

**Why deferred:** spam-fee calibration is a governance decision that
benefits from real-world data. 100 ujuno is the placeholder; the DAO
can adjust as needed. The contract code path is unchanged from
upstream — only the default at instantiate time differs.

## D6.8 — Test infrastructure call: bech32 mode

**Decision (test-only):** the workspace `integration-tests` crate uses
`MockApiBech32("juno") + MockAddressGenerator` to mirror production
chain behavior. This is **not** a contract change — it only affects
how the in-process cw-multi-test app validates addresses.

**Why this matters for the audit:** TF LP denoms have the shape
`factory/juno1xxx/astroport/share`. In production, `addr_validate(<full denom>)`
fails (slashes aren't valid bech32) → `determine_asset_info` correctly
classifies the string as `AssetInfo::NativeToken`. In the default
loose MockApi, `addr_validate` passes (any short ASCII string is
"valid"), so the test environment mis-classifies the LP denom as a
cw20 contract address. The bech32-mode test app matches production.

**Implication for the auditor:** the integration-tests' bech32 mode
is the correct test posture; do not "relax" it to MockApi loose mode
without breaking the contract's TF-LP code path validation. The
contract itself is unchanged — production already has strict bech32
addr_validate.

## D6.9 — Keep-set tweak: `is_generator_disabled = false`

**Decision:** flip `astroport::factory::PairConfig.is_generator_disabled`
from `true` (P0/P3 default) to `false` for the XYK pair type in the
v1 production keep-set.

**Why:** the factory's `BlacklistedPairTypes` query (which
`incentives.setup_pools()` consults) returns pair types where
`is_disabled || is_generator_disabled`. With `is_generator_disabled =
true`, no pool of that type can receive incentives.

In P0/P3 this was set to `true` because incentives didn't exist — the
factory was deployed standalone. Now that incentives is v1 scope (P2.5),
the production keep-set must allow XYK pools to be incentivized.

The downstream effect on `factory.UpdateConfig`-via-DAO-governance:
no change. The DAO can still disable specific pools by setting
`is_disabled` on a per-PairConfig basis, or by setting
`is_generator_disabled` per-PairConfig.

**Audit implication:** reviewer should confirm the integration-tests
deployer flips this field and that the existing pair tests still pass
(no behavioral change to pair-side contracts).

## Cross-references

- Master plan: `planning/00-overview.md`
- Strip list: `planning/01-strip-list.md` (P0 deletions)
- ADR D1 (incentives types-only): `planning/04-incentives-types-decision.md`
  — superseded by this ADR (incentives is now a live contract again,
  not just types).
- P2.5 plan: `planning/11-incentives-and-gauges.md`
- Wire shim drift gate: `packages/astroport_juno_types/tests/wire_drift.rs`
- AUDIT REGRESSION GATE: `integration-tests/tests/incentives_external_cw20.rs`

## Open audit follow-ups

1. **External-incentive accounting drift** (per the audit-attention
   zone in `11-incentives-and-gauges.md`) — concurrent schedules in
   different reward tokens with overlapping windows. Upstream's logic
   in `PoolInfo::incentivize` (state.rs:264) is preserved verbatim;
   the auditor should confirm it.

2. **cw20::TransferFrom failure handling** — if a funder revokes
   allowance between submission and execution, the `Incentivize` call
   fails atomically. Auditor should verify via the existing reply
   handler (`reply.rs::POST_TRANSFER_REPLY_ID`).

3. **Worst-case gas under 5 cw20 rewards** — the audit firm may want
   to add a gas-profile measurement test to lock in the
   ClaimRewards-with-N-rewards upper bound.
