# 05 — Toolchain and CI

Reference doc for the build / test / CI surface of the fork. Update when any
of these change.

## Rust toolchain

Pinned to **Rust 1.81.0** in GitHub Actions workflows
(`.github/workflows/tests_and_checks.yml`, `code_coverage.yml`).

No `rust-toolchain.toml` file at the repo root. Adding one is optional —
upstream Astroport hasn't, and matching their convention keeps the diff
clean for audit.

When the workspace eventually moves to a newer pin (e.g., to follow
`dao-contracts`' nightly-2024-01-08 or the eventual cosmwasm-std 3.x bump),
update both workflows simultaneously and note the date here.

## Build optimizer

`scripts/build_release.sh` uses **`cosmwasm/rust-optimizer:0.17.0`** Docker
image, mounted with target + cargo registry caches. Outputs `.wasm`
artifacts to `artifacts/` at the repo root.

Artifact size budget (`scripts/check_artifacts_size.sh`):

- **Upstream Astroport:** 800 KB per wasm (Terra + Injective ceiling).
- **Our fork:** 3072 KB (3 MB) per wasm — Juno wasmd's default ceiling.

The change lands in P0 commit #10.

## `cosmwasm-check` capabilities

Run on each built artifact in `check_artifacts.yml`. **Capabilities for Juno:**

```
staking,cosmwasm_1_1,cosmwasm_2_0,iterator,stargate
```

Differences from upstream:

- Drop `neutron` capability (not on Juno).
- Add `cosmwasm_2_0` for forward compatibility with wasmvm v3.0.4
  (Juno post-v30; v3.0.4 accepts both interface_version_8 (cosmwasm-std
  1.x / 2.x) and interface_version_9 (cosmwasm-std 3.x)). Astroport ships
  cosmwasm-std 1.5 → interface_version_8 → works on Juno today.

## `cw-multi-test` fork pin

Workspace pins `cw-multi-test` to:

```
github.com/astroport-fi/cw-multi-test
branch: feat/bank_with_send_hooks_1_0
```

We do not control this branch. Two options:

### Current state — ride astroport-fi's fork

Cheapest. Risk: silent breakage when upstream cw-multi-test ships a
breaking change and astroport-fi doesn't rebase promptly. Mitigation:
quarterly check on whether the branch has been updated; if it has, rebase
our pin.

### Decision (deferred to v1.1)

Fork-the-fork to the Juno fork-home org (per
`memory/juno-ai-github-identity.md`). One-time `git push` + `Cargo.toml`
URL swap. Cost: ~1 hour. Pays off the first time astroport-fi falls
behind upstream.

For v0.1.0-juno-rc0 / -rc1 we keep the astroport-fi pin to minimize the
audit-side diff vs upstream.

## CI workflows

Four workflows in `.github/workflows/`:

| Workflow | Trigger | Purpose | Juno changes |
|---|---|---|---|
| `tests_and_checks.yml` | PR + push main | fmt, clippy, test, schema-diff | Drop `--features tests-tube` if no longer applicable. |
| `check_artifacts.yml` | PR + push main | optimizer + `cosmwasm-check` | Capability change as above. Size bump. |
| `code_coverage.yml` | push main | tarpaulin → codecov.io | Drop `CODECOV_TOKEN` step (astroport-fi's). |
| `release_artifacts.yml` | tag `v*.*.*` | tar artifacts → GH release | Reduce to `gh release create` of wasms. |

GitHub Actions versions: `actions/checkout@v3 → @v4`. Replace deprecated
`actions-rs/*` with `dtolnay/rust-toolchain@1.81.0`. Lands in P0 commit #10.

## Schemas convention

Upstream Astroport tracks `schemas/{contract}/raw/` in git. We keep this
convention — audit-traceability of "the JSON wire format at commit X
matches the audit-time wire format" outweighs the ~80 KB git-tree saving
of treating `raw/` as a build artifact.

Differs from `dao-contracts`, where `raw/` is `.gitignore`d post PR #926.

`scripts/build_schemas.sh` + a `git diff --exit-code` CI step enforces
no-drift.

## Code style

No `rustfmt.toml` / `clippy.toml` / `.taplo.toml` at the workspace root.
CI uses default `rustfmt` and `cargo clippy -- -D warnings -A unexpected_cfgs`.
Inherit upstream's implicit style.

If a future contributor wants stricter linting (forbid unsafe, enforce
docs, etc.), add the config here and update CI accordingly.

## Local task runner

A `justfile` will land at the repo root in P5. Targets:

```
build          # cargo build --workspace --locked --release
test           # cargo test --workspace --locked
clippy         # cargo clippy --workspace --all-targets -- -D warnings -A unexpected_cfgs
fmt            # cargo fmt --all
fmt-check      # cargo fmt --all -- --check
schema         # cargo run --example schema for each contract; refresh schemas/
wasms          # scripts/build_release.sh (Docker optimizer)
cosmwasm-check # cosmwasm-check artifacts/*.wasm --available-capabilities <as above>
deploy-uni7    # scripts/deploy/uni7.sh
deploy-juno-1  # scripts/deploy/juno-1.sh
find-tf-addr   # derive Juno TokenFactory module bech32
```

Not added in P0 — that work is in P5 where the deploy scripts also land.
