# HoS TLA Contracts — Audit Handoff

NEAR smart contracts for the House of Stake TLA name-rental marketplace.
Four crates: registry (the orchestrator), manager (deployed at each TLA, owns
its sub-account namespace), sub-account locker (hold-until-export custody for
each rented sub-account), resale locker (custody-free `.near` resale primitive).

## Verifying the freeze

Reproducible-build environment is pinned in every contract's
`Cargo.toml` under `[package.metadata.near.reproducible_build]`:

- Image: `sourcescan/cargo-near:0.18.0-rust-1.86.0`
- Image digest: `sha256:2d0d458d2357277df669eac6fa23a1ac922e5ed16646e1d3315336e4dff18043`
- Container build command: `cargo near build non-reproducible-wasm --locked --no-abi`
  (reproducibility is provided by the deterministic Docker container; ABI
  generation is disabled because it is not required for audit verification)

To reproduce the canonical WASM byte-for-byte from a clean clone:

```bash
git clone https://github.com/neo-sky/hos-tla
cd hos-tla
git checkout <audit-tag>
for c in sub-account-locker resale-locker tla-manager tla-registry; do
  (cd contracts/$c && cargo near build reproducible-wasm)
done
for f in target/near/*/*.wasm; do sha256sum "$f"; done
```

Build order matters because `tla-manager` and `tla-registry` embed the locker
artifacts via `include_bytes!`. The expected hashes after the full chain are
listed below.

## Canonical freeze — tag `audit-v1` (SHA-256)

All four reproduced at the same commit (`e4ee683`). Determinism verified: two independent reproducible builds of `sub-account-locker` at this commit produce byte-identical WASM.

```
a7321deb849a4a50a40048b3074f4fdebec2b83febd89f141526eb990b8f7d33  sub_account_locker.wasm   (109,960 B)
1fec4ef292ae4fb3506381f393b98b5a00efae2cc08eefb5cf7dbed07bc0dc6a  resale_locker.wasm         (96,655 B)
5a796d2f278c3fa1d90d112e5393c11318c9510ff0af6f28f7356f5e3eb471fd  tla_manager.wasm          (211,264 B)
d3df1821872458ebad9e3a0108249836041f9ab07d37db28e14da7ecb3f483f3  tla_registry.wasm         (380,656 B)
```

NEP-330 source metadata embedded on-chain:
- `repository`: `https://github.com/neo-sky/hos-tla`
- `link`: the audit-tag commit URL
- `build_environment`: the pinned image with digest
- `source_code_snapshot`: `git+https://github.com/neo-sky/hos-tla?rev=<commit>`

After deployment, fetch on-chain metadata via:

```bash
near view <contract> contract_source_metadata '{}'
```

The reviewer rebuilds from the source snapshot inside the pinned image and
compares the resulting SHA-256 against the on-chain code hash.

## Audit reading order

1. [STANDARDS.md](STANDARDS.md) — file-by-file proof that each source file meets the non-negotiable standards (zero comments, function-size limits, etc.) and the verification matrix (clippy strict, fmt, build, tests).
2. [THREAT_MODEL.md](THREAT_MODEL.md) — trust assumptions, asset-flow paths, per-contract invariants (with code links), NEAR vulnerability-class mapping, and known limitations.
3. [BUILD.md](BUILD.md) — build/deploy procedure, bundling discipline, the host-build vs reproducible-build distinction (critical: plain `cargo build --target wasm32-unknown-unknown --release` produces nearcore-invalid WASM; only `cargo near build` artifacts deploy).
4. [contracts/](contracts/) — the four crates. Start at `tla-registry/src/lib.rs`; the modules read in this order: types, fees, error, events, admin, rental, callbacks, mother, business, reclaim, views.
5. [contracts/tla-registry/tests/integration.rs](contracts/tla-registry/tests/integration.rs) — seven near-workspaces scenarios covering the audit-reviewable security paths.

## Custody model

Hold-until-export. Rented sub-accounts are held by the sub-account locker with
zero access keys after creation; the renter's key is stored in the locker and
added only on registry-gated `export`. Reclaim is genuinely enforceable
because the renter cannot deploy over the locker. See
[THREAT_MODEL.md](THREAT_MODEL.md) section 3.2-3.3 and
[contracts/tla-manager/src/lib.rs](contracts/tla-manager/src/lib.rs) for the
batched create-account flow.

## Known assumptions documented for the audit

- **Export gating** is admin-mediated for V1. The registry's
  `export_sub_account` is admin-gated (HoS-mediated release). Self-service
  buyout-priced export is a marketplace-layer follow-up and bakes in no
  economic assumption at the contract level.
- **Renewal economics** (Finding 7 from the 2026-05-23 external review):
  per-sub-account renewal is collected via individual `renew_sub_account`
  calls; `get_business_renewal_cost` reports the aggregate. A
  single-call bulk renewal is a marketplace-layer feature, not contract-core.
- **FT sweep** is best-effort across heterogeneous NEP-141/145 token
  contracts. Safety is preserved by the registry's enforced finalize:
  `reclaim_finalize` blocks `delete_account` while any allowlisted FT balance
  is nonzero. A skipped sweep leaves the account undeletable, never
  deleted-with-stranded-funds.
- **NFT auto-sweep on expiry** is not implemented. NFTs left in an
  abandoned-and-lapsed sub-account stay in the NFT contract keyed to a
  now-deleted account. Documented; frontend warns at listing.
- **Admin authority** for V1 is the deploying address set; production should
  bind a multisig DAO to the admin role before mainnet launch.

## Integration test results

```
test test_business_sub_cap_override ... ok
test test_hold_until_export ... ok
test test_lifecycle_business_tla ... ok
test test_mother_dos_rejected ... ok
test test_pause_blocks_user_methods ... ok
test test_pull_payment_refund_excess ... ok
test test_resale_lock_unlock_replay_blocked ... ok
test result: ok. 7 passed; 0 failed
```

Run with: `cargo test -p tla-registry --test integration -- --test-threads=1`
(first run downloads the `near-sandbox` binary via near-workspaces).
