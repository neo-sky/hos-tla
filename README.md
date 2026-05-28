# HoS TLA Contracts: Audit Handoff

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

## Canonical freeze: tag `audit-v3` (SHA-256)

`audit-v3` adds the held sub-account resale primitive (the locker `transfer` method and the registry `marketplace.rs` module) on top of `audit-v2`, which closed the eight findings from the 2026-05-27 external review. The canonical hashes below are also recorded in the annotated `audit-v3` tag message.

History note: TRACKING.md was permanently removed from every commit via `git-filter-repo` because it contained sensitive contract figures. As a result, every commit SHA in the repo was rewritten and the prior `audit-v1` and pre-scrub `audit-v2` tags were deleted (their bundled WASM bytes had embedded NEP-330 rev metadata pointing to commits that no longer exist).

The freeze is layered (inherent to the `include_bytes!` factory pattern, since cargo-near embeds `NEP330_BUILD_INFO_SOURCE_CODE_SNAPSHOT=git+<repo>?rev=<HEAD>` into every WASM, so any new commit changes the leaf hashes):

```
Bundled leaf bytes (in res/, reproducibly built at audit-v3~1 = 7f0b774):
  cc44a9367237581cac44d56527fcad8de98d0c27c2b6afa6bdf95b9de76980c3  sub_account_locker.wasm  (111,150 B)
  45a639b293acff836413b245098f612227c55b279d355ad6b4c3d1bff45283ed  resale_locker.wasm        (96,655 B)

Bundlers (reproducibly built at audit-v3 = 85e3b89, embed the leaf bytes above):
  f42637fe3106a699635e5a78b4c4ed553ccb72a1fa30352fc5ea687e8f45a771  tla_manager.wasm         (212,449 B)
  44e5bdec9832fef5ad24f67040e725c86bbb4017501f9b4fedca31b33fedc599  tla_registry.wasm        (419,949 B)
```

### Auditor verification procedure

```bash
git clone https://github.com/neo-sky/hos-tla && cd hos-tla

# Step 1: verify the bundled leaf bytes against their build commit.
git checkout audit-v3~1
(cd contracts/sub-account-locker && cargo near build reproducible-wasm)
(cd contracts/resale-locker      && cargo near build reproducible-wasm)
sha256sum target/near/sub_account_locker/sub_account_locker.wasm  # must = cc44a936...
sha256sum target/near/resale_locker/resale_locker.wasm            # must = 45a639b2...

# Step 2: verify the bundlers at the audit tag.
git checkout audit-v3
(cd contracts/tla-manager  && cargo near build reproducible-wasm)
(cd contracts/tla-registry && cargo near build reproducible-wasm)
sha256sum target/near/tla_manager/tla_manager.wasm                # must = f42637fe...
sha256sum target/near/tla_registry/tla_registry.wasm              # must = 44e5bdec...
```

This layered procedure exists because rebuilding a leaf at `audit-v3` would embed `rev=audit-v3`, producing different bytes than the ones already committed in `res/` (which embed `rev=audit-v3~1`). The committed `res/` bytes ARE the canonical leaf artifacts; the bundlers verifiably embed them via `include_bytes!`.

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

1. [STANDARDS.md](STANDARDS.md): file-by-file proof that each source file meets the non-negotiable standards (zero comments, function-size limits, and so on) and the verification matrix (clippy strict, fmt, build, tests).
2. [THREAT_MODEL.md](THREAT_MODEL.md): trust assumptions, asset-flow paths, per-contract invariants (with code links), NEAR vulnerability-class mapping, and known limitations.
3. [BUILD.md](BUILD.md): build/deploy procedure, bundling discipline, the host-build vs reproducible-build distinction (critical: plain `cargo build --target wasm32-unknown-unknown --release` produces nearcore-invalid WASM; only `cargo near build` artifacts deploy).
4. [contracts/](contracts/): the four crates. Start at `tla-registry/src/lib.rs`; the modules read in this order: types, fees, error, events, admin, rental, callbacks, mother, business, reclaim, marketplace, views.
5. [contracts/tla-registry/tests/integration.rs](contracts/tla-registry/tests/integration.rs): twenty-three near-workspaces scenarios covering the audit-reviewable security paths.

## Custody model

Hold-until-export. Rented sub-accounts are held by the sub-account locker with
zero access keys after creation; the renter's key is stored in the locker and
added only on registry-gated `export`. Reclaim is genuinely enforceable
because the renter cannot deploy over the locker. See
[THREAT_MODEL.md](THREAT_MODEL.md) section 3.2-3.3 and
[contracts/tla-manager/src/lib.rs](contracts/tla-manager/src/lib.rs) for the
batched create-account flow.

A held sub-account can be resold without leaving custody. `buy_sub_account`
swaps the locker's stored key to the buyer and moves registry ownership while
the account stays held, so reclaim remains enforceable through a sale and the
buyer's key is added only on a later `export`. See
[THREAT_MODEL.md](THREAT_MODEL.md) section 2.7.

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
- **Held sub-account resale** is in the audited core (audit-v3). Listings,
  offers, and counteroffers are discovered off-chain; settlement is on-chain
  (`list_sub_account` or `accept_offer`, then `buy_sub_account`). The seller's
  on-chain price is the binding floor; below-floor sales use `accept_offer`
  bound to a specific buyer. `resale_commission_bps` ships at 0 and is admin-set
  via `update_fee_config`.

## Integration test results

```
test test_business_sub_cap_override ... ok
test test_hold_until_export ... ok
test test_lifecycle_business_tla ... ok
test test_mother_dos_rejected ... ok
test test_mother_pre_squat_does_not_block_future_sub ... ok
test test_pause_blocks_user_methods ... ok
test test_pull_payment_refund_excess ... ok
test test_resale_accepted_offer_bound_to_buyer ... ok
test test_resale_authorization_guards ... ok
test test_resale_blocked_when_tla_suspended ... ok
test test_resale_blocked_while_assets_unverifiable ... ok
test test_resale_buy_refunds_excess ... ok
test test_resale_commission_split ... ok
test test_resale_list_buy_transfers_and_pays ... ok
test test_resale_list_requires_one_yocto ... ok
test test_resale_lock_unlock_replay_blocked ... ok
test test_resale_mother_not_sellable ... ok
test test_resale_pause_blocks_marketplace ... ok
test test_resale_relist_updates_price ... ok
test test_resale_retraction_blocks_sale ... ok
test test_resale_revoke_offer_blocks_buyer ... ok
test test_resale_unlist_clears_sale ... ok
test test_resale_zero_price_rejected ... ok
test result: ok. 23 passed; 0 failed
```

Run with: `cargo test -p tla-registry --test integration -- --test-threads=1`
(first run downloads the `near-sandbox` binary via near-workspaces).
