# HoS TLA — Build & Reproducible Verification

## Local development build

```bash
cd /home/skylar/VScode/hos-tla
cargo build --target wasm32-unknown-unknown --release --workspace
```

Artifacts at `target/wasm32-unknown-unknown/release/{resale_locker,sub_account_locker,tla_manager,tla_registry}.wasm`.

### Bundling discipline

Two crates embed canonical WASM artifacts via `include_bytes!`:

| Bundler | Bundled | Path |
|---|---|---|
| `tla-manager` | `sub_account_locker.wasm` | `contracts/tla-manager/res/sub_account_locker.wasm` |
| `tla-registry` | `resale_locker.wasm` | `contracts/tla-registry/res/resale_locker.wasm` |

**Build order matters.** When the bundled WASM source changes, refresh the bundled copy and rebuild the bundler:

```bash
# Refresh sub-account-locker bundle in manager
cp target/wasm32-unknown-unknown/release/sub_account_locker.wasm \
   contracts/tla-manager/res/sub_account_locker.wasm
cargo build --target wasm32-unknown-unknown --release -p tla-manager

# Refresh resale-locker bundle in registry
cp target/wasm32-unknown-unknown/release/resale_locker.wasm \
   contracts/tla-registry/res/resale_locker.wasm
cargo build --target wasm32-unknown-unknown --release -p tla-registry
```

## Reproducible build (audit-grade)

Every crate's `Cargo.toml` declares NEP-330 reproducible-build metadata:

```toml
[package.metadata.near.reproducible_build]
image = "sourcescan/cargo-near:0.18.0-rust-1.86.0"
image_digest = "sha256:2d0d458d2357277df669eac6fa23a1ac922e5ed16646e1d3315336e4dff18043"
container_build_command = ["cargo", "near", "build", "reproducible-wasm"]
```

The image digest is pinned, so a reproducible build today and a reproducible build by a third-party reviewer produce byte-identical WASM.

### Run the reproducible build

```bash
cd contracts/sub-account-locker && cargo near build reproducible-wasm
cd ../resale-locker            && cargo near build reproducible-wasm
# Then refresh the embedded artifacts:
cp ../sub-account-locker/target/near/sub_account_locker/sub_account_locker.wasm \
   ../tla-manager/res/sub_account_locker.wasm
cp ../resale-locker/target/near/resale_locker/resale_locker.wasm \
   ../tla-registry/res/resale_locker.wasm
cd ../tla-manager  && cargo near build reproducible-wasm
cd ../tla-registry && cargo near build reproducible-wasm
```

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
| Source builds with no warnings | `cargo build --workspace --target wasm32-unknown-unknown --release` |
| Clippy strict clean | `cargo clippy --target wasm32-unknown-unknown --release --workspace -- -D warnings` |
| Format clean | `cargo fmt --check` |
| Embedded WASM matches latest build | `sha256sum target/.../release/{sub_account_locker,resale_locker}.wasm` matches `sha256sum contracts/.../res/*.wasm` |
| Reproducible build produces byte-identical artifacts | Two separate runs of `cargo near build reproducible-wasm` on different machines produce the same SHA-256 |

## Integration test execution

```bash
cd /home/skylar/VScode/hos-tla
cargo build --target wasm32-unknown-unknown --release --workspace
cargo test -p tla-registry --test integration -- --test-threads=1
```

The tests use `near-workspaces` to spin up a local NEAR sandbox per test, deploy the freshly-built WASM, and exercise the full lifecycle. `--test-threads=1` keeps sandbox state isolated per test (sandboxes are not cheap to instantiate; sequential is more reliable).

First run downloads the `near-sandbox` binary (~200 MB). Subsequent runs are local.
