# HoS TLA Build and Reproducible Verification

**One toolchain, no ambiguity:** all four contracts are built via `cargo near build`.
Plain `cargo build --target wasm32-unknown-unknown --release` produces WASM that
nearcore rejects with `CompilationError(PrepareError(Deserialization))`. Do not
deploy it. The standards bar requires `cargo near`'s `wasm-opt` post-processing.

## One-command verify

```bash
./verify.sh
```

Builds all four contracts via `cargo near build non-reproducible-wasm --locked --no-abi`,
hashes them, and runs the seven near-workspaces integration tests. Use this for
local pre-push checks.

## Local development build

```bash
cd contracts/sub-account-locker && cargo near build non-reproducible-wasm --no-abi
cd ../resale-locker            && cargo near build non-reproducible-wasm --no-abi
cd ../tla-manager              && cargo near build non-reproducible-wasm --no-abi
cd ../tla-registry             && cargo near build non-reproducible-wasm --no-abi
```

Artifacts at `target/near/{sub_account_locker,resale_locker,tla_manager,tla_registry}/<name>.wasm`.

### Bundling discipline

Two crates embed canonical WASM artifacts via `include_bytes!`:

| Bundler | Bundled | Path |
|---|---|---|
| `tla-manager` | `sub_account_locker.wasm` | `contracts/tla-manager/res/sub_account_locker.wasm` |
| `tla-registry` | `resale_locker.wasm` | `contracts/tla-registry/res/resale_locker.wasm` |

**Build order matters.** When the bundled WASM source changes, refresh the bundled copy and rebuild the bundler:

```bash
cp target/near/sub_account_locker/sub_account_locker.wasm \
   contracts/tla-manager/res/sub_account_locker.wasm
(cd contracts/tla-manager && cargo near build non-reproducible-wasm --no-abi)

cp target/near/resale_locker/resale_locker.wasm \
   contracts/tla-registry/res/resale_locker.wasm
(cd contracts/tla-registry && cargo near build non-reproducible-wasm --no-abi)
```

## Reproducible build (audit-grade)

Every crate's `Cargo.toml` declares NEP-330 reproducible-build metadata:

```toml
[package.metadata.near.reproducible_build]
image = "sourcescan/cargo-near:0.18.0-rust-1.86.0"
image_digest = "sha256:2d0d458d2357277df669eac6fa23a1ac922e5ed16646e1d3315336e4dff18043"
container_build_command = ["cargo", "near", "build", "non-reproducible-wasm", "--locked", "--no-abi"]
```

The image digest is pinned, so a reproducible build today and a reproducible build by a third-party reviewer produce byte-identical WASM.

### Run the reproducible build

```bash
(cd contracts/sub-account-locker && cargo near build reproducible-wasm)
(cd contracts/resale-locker      && cargo near build reproducible-wasm)
cp target/near/sub_account_locker/sub_account_locker.wasm contracts/tla-manager/res/sub_account_locker.wasm
cp target/near/resale_locker/resale_locker.wasm           contracts/tla-registry/res/resale_locker.wasm
# Commit the refreshed res/ artifacts; reproducible build snapshots git HEAD, so
# the bundlers must see the new bytes committed.
git add contracts/tla-manager/res/sub_account_locker.wasm contracts/tla-registry/res/resale_locker.wasm
git commit -m "Refresh bundled locker artifacts"
(cd contracts/tla-manager  && cargo near build reproducible-wasm)
(cd contracts/tla-registry && cargo near build reproducible-wasm)
```

### Layered freeze caveat (important for auditors)

`cargo-near` embeds `NEP330_BUILD_INFO_SOURCE_CODE_SNAPSHOT=git+<repo>?rev=<HEAD>`
inside the produced WASM. That means:

- The leaf locker WASMs committed in `contracts/tla-*/res/` were reproducibly built
  at the commit that *added them*. They are immutable byte-blobs from that earlier
  commit's perspective.
- The bundler (`tla-manager` / `tla-registry`) reproducibly builds at the *current*
  audit-tag HEAD and embeds those frozen leaf bytes verbatim via `include_bytes!`.
- Therefore: at the audit tag, `cargo near build reproducible-wasm` for a leaf
  produces bytes with the audit-tag rev embedded, and these will NOT equal the
  bytes in `res/`, because the `res/` bytes have an earlier rev embedded.

Auditor's two-step verification:

1. **Leaf integrity:** check out the commit at which the `res/` bytes were
   produced (see the leaf-build-commit table in [README.md](README.md)) and run
   `cargo near build reproducible-wasm` for that leaf. Compare to the `res/`
   bytes. Must match.
2. **Bundler integrity:** check out the audit tag and run
   `cargo near build reproducible-wasm` for each bundler. Compare to the
   published bundler hash. Must match.

This is inherent to the `include_bytes!` factory pattern; the alternative (re-baking
every commit) chases a moving rev and never converges.

### Verifying the on-chain deployment

After deployment, SourceScan (or any reviewer) reads `contract_source_metadata` via:

```bash
near view <contract.testnet> contract_source_metadata '{}'
```

The reviewer rebuilds from source via the pinned image and compares the SHA-256 of the freshly-built WASM to the on-chain code hash returned by:

```bash
near state <contract.testnet>
```

A match proves the deployed code corresponds to the audited source.

## Reproducible-build prerequisites

- Docker available (the build runs inside the pinned `sourcescan/cargo-near` image).
- `cargo-near` installed: `cargo install cargo-near`.
- Network access to pull the Docker image (first run only; subsequent runs use the local cache).

## Verification matrix

| Property | Check |
|---|---|
| Source builds with no warnings | `cargo build --workspace --target wasm32-unknown-unknown --release` (host check only, DO NOT deploy these artifacts) |
| Clippy strict clean | `cargo clippy --target wasm32-unknown-unknown --release --workspace -- -D warnings` |
| Format clean | `cargo fmt --check` |
| Deployable artifacts produced | `cargo near build non-reproducible-wasm --locked --no-abi` per crate |
| Embedded WASM matches committed res/ | `sha256sum contracts/tla-manager/res/sub_account_locker.wasm contracts/tla-registry/res/resale_locker.wasm` |
| Reproducible build produces byte-identical artifacts at a given commit | Two separate runs of `cargo near build reproducible-wasm` at the same HEAD produce the same SHA-256 |

## Integration test execution

```bash
# Tests read from target/near/*/*.wasm; build via cargo-near first
(cd contracts/sub-account-locker && cargo near build non-reproducible-wasm --no-abi)
(cd contracts/resale-locker      && cargo near build non-reproducible-wasm --no-abi)
(cd contracts/tla-manager        && cargo near build non-reproducible-wasm --no-abi)
(cd contracts/tla-registry       && cargo near build non-reproducible-wasm --no-abi)
cargo test -p tla-registry --test integration -- --test-threads=1
```

The tests use `near-workspaces` to spin up a local NEAR sandbox per test, deploy
the freshly-built WASM, and exercise the full lifecycle. `--test-threads=1` keeps
sandbox state isolated per test (sandboxes are not cheap to instantiate;
sequential is more reliable).

First run downloads the `near-sandbox` binary (~200 MB). Subsequent runs are local.
