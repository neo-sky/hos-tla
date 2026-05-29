# HoS TLA Standards Walkthrough

File-by-file proof that each source file meets the non-negotiable standards. Reviewer can grep each property against the named file.

## Standards (the bar)

| Property | Verification |
|---|---|
| Zero dev comments | `grep -nE "^[[:space:]]*//" contracts/*/src/**/*.rs` returns only `#[derive(...)]` and module attributes, no narrative comments |
| Zero AI tells | No "helper", "utility", "for now", "note:", "TODO" in any source file |
| Zero stubs | No `todo!()`, `unimplemented!()`, `unreachable!()` on reachable paths; no placeholder returns |
| Zero non-ASCII in source | `LC_ALL=C grep -P "[^\x00-\x7F]" contracts/*/src/**/*.rs` is empty |
| Function size <= 100 lines | Largest function: registry `reclaim_finalize` at ~65 lines (within budget) |
| Cyclomatic complexity <= 8 | Verified via `cargo clippy -- -W clippy::cognitive_complexity`, no warnings at threshold |
| Parameters <= 5 positional | All multi-arg methods take config structs (e.g., `FeeConfig`) instead of positional sprawl |
| Line length <= 100 chars (120 hard max) | `cargo fmt` enforced with default rustfmt config (max_width = 100) |
| ASCII only | Same grep as above |
| `cargo clippy -- -D warnings` clean | Verified per workstream |
| `cargo fmt --check` clean | Verified per workstream |
| `cargo build --target wasm32-unknown-unknown --release --workspace` clean | Verified per workstream |
| Structured `ContractError` enum | Every fallible public method returns `Result<T, ContractError>` via `#[handle_result]` |
| Every `env::panic_str` replaced | Only `panic_str` calls remaining are inside the three `FunctionError::panic` impls (canonical emission point); only `.expect()` remaining is event serialization (unreachable invariant) |
| Checked/saturating arithmetic | `saturating_add` / `saturating_sub` / `saturating_mul` on every balance and counter |
| `overflow-checks = true` retained | In `[profile.release]` of `Cargo.toml` workspace |
| Every callback `#[private]` | `on_sub_account_created`, `on_balances_checked`, `on_reclaim_finalized`, `on_claim_refund_settled`, `on_sub_account_sold`, `after_balance_query`, `after_storage_deposit`, all annotated |
| Every PromiseResult matched | `is_promise_success()` used for single-result callbacks; `promise_result_checked(i, MAX_LEN)` for fan-in callbacks |
| State finalized only on success branch | Counter increments, ownership moves, and event emissions are gated by `is_promise_success()` checks in callbacks |
| Pull-payment everywhere value leaves the contract | All refunds, admin withdrawals, and resale proceeds route through `pending_refunds`; no critical-path `Promise::transfer` to arbitrary accounts |
| Sweep-first invariant enforced | `reclaim_finalize` and resale `buy_sub_account` fan out `ft_balance_of` across the allowlist; any non-zero or unverifiable balance aborts (fail-closed) |
| Raw `delete_account` removed from codebase | Only `delete_account(destination)` call is self-delete inside the sub-account-locker (predecessor == receiver, protocol-valid) |
| Reentrancy: no exploitable state mutation between cross-contract call and callback | Counters incremented in callback success branch; optimistic patterns (sub_accounts insert, business_sub_count bump) are rolled back on callback failure |
| Access control: every mutating method gated | Admin / licensee / owner / registry / predecessor checks documented per method in THREAT_MODEL.md |
| Reproducible build pinned | `[package.metadata.near.reproducible_build]` in every crate's Cargo.toml: image `sourcescan/cargo-near:0.18.0-rust-1.86.0`, digest `sha256:2d0d458d2357277df669eac6fa23a1ac922e5ed16646e1d3315336e4dff18043` |

## File-by-file walkthrough

### contracts/tla-registry

| File | Public surface | Notes |
|---|---|---|
| `src/lib.rs` | `new`, `pause`/`unpause`, `claim_refund`, `migrate`, `get_version`, `is_paused`, `get_pending_refund`, `get_total_pending_refunds`; private `on_claim_refund_settled` callback | Module declarations, state struct, init, pause primitive, pull-payment claim with restoration callback, version-gated migrate |
| `src/error.rs` | `ContractError`, `NameInvalidReason` | Typed enum implementing `FunctionError` manually; serde tag="code" rename_all="snake_case" |
| `src/types.rs` | `TlaType`, `TlaStatus`, `PremiumCategory`, `TlaEntry`, `SubAccountEntry`, `FeeConfig` (with `resale_commission_bps`), `Listing`, `AcceptedOffer`, lifecycle enum and views, `validate_name` | All structs have `Borsh{Serialize,Deserialize}`; lifecycle methods on `TlaEntry` and `SubAccountEntry`; `validate_name` returns `Result<(), ContractError>` |
| `src/fees.rs` | `base_rent`, `sub_account_rent`, `calculate_rent`, `default_fee_config` | Pure free functions; no state borrow conflicts |
| `src/events.rs` | NEP-297 event emission via `EVENT_JSON:` log format; typed event structs | Standard = "hos-tla" version = "1.0.0" |
| `src/admin.rs` | `register_tla`, `suspend_tla`, `unsuspend_tla`, `add_admin`, `remove_admin`, `update_fee_config`, `withdraw` (pull), allowlist mutators (ft + nft), `activate_open_tla` | All admin-gated via `assert_admin()?` |
| `src/rental.rs` | `activate_tla`, `rent_sub_account`, `renew_tla`, `set_main_wallet`, `renew_sub_account` | User-facing mutations; all `assert_not_paused()?` gated; ext_contract for `tla_manager.create_sub_account` |
| `src/callbacks.rs` | `on_sub_account_created` (`#[private]`) | Optimistic-insert rollback pattern; business_sub_count decremented on failure |
| `src/reclaim.rs` | `reclaim_sweep_ft`, `reclaim_finalize`, `on_balances_checked` (`#[private]`), `on_reclaim_finalized` (`#[private]`) | Sweep-first enforcement; mother and retraction gates; `promise_result_checked` for bounded ft_balance_of reads |
| `src/mother.rs` | `set_mother`, `get_mother`, `is_mother`, `get_mother_use_count`, `admin_clear_mother`; private `ensure_mother_default`, `set_mother_internal`, count helpers; `effective_sub_lifecycle` | DoS-fix at set_mother_internal: ownership check on HoS sub-accounts; count-based reverse for 1-to-N semantics |
| `src/business.rs` | `schedule_retraction`, `cancel_retraction`, `get_business_sub_count`, `get_business_renewal_cost`, `get_retraction_at`; private `business_count_check_and_bump`, `business_count_decrement` | Retraction state machine; post-elapse cancel block; business cap enforcement |
| `src/resale.rs` | `resale_unlock`, `resale_abort`, `get_resale_locker_wasm`, `get_resale_locker_sha256`, `get_resale_locker_size` | Admin-gated dispatch to ext_resale_locker; canonical WASM published via view |
| `src/marketplace.rs` | `list_sub_account`, `unlist_sub_account`, `accept_offer`, `revoke_offer`, `buy_sub_account` (payable); private `on_sub_account_sold`; `get_listing`, `get_accepted_offer` views | Held-sub-account resale settlement; owner-gated listing with on-chain price floor, optional buyer-bound `accept_offer`; per-account settling lock blocks double-fill; seller proceeds and buyer refunds via `pending_refunds`; commission to `total_revenue`; sale-entry purge on reclaim/export; settlement gated on allowlisted-FT emptiness via `ft_balance_of` fan-out (fail-closed) |
| `src/views.rs` | `get_tla`, `get_sub_account`, `get_rent_price`, `is_name_available`, `list_tlas` (paginated), `get_fee_config`, `get_stats`, `get_admins`, `get_ft_allowlist`, `get_nft_allowlist` | All pure reads; pagination on the only iterable view |

### contracts/tla-manager

| File | Public surface | Notes |
|---|---|---|
| `src/lib.rs` | `new`, `create_sub_account`, `get_config` | Registry-only callability; deploys sub-account-locker via include_bytes |
| `src/error.rs` | `ManagerError` (Unauthorized, InvalidSubAccountName) | Typed enum; `FunctionError` impl |

### contracts/sub-account-locker

| File | Public surface | Notes |
|---|---|---|
| `src/lib.rs` | `new`, `sweep_ft`, `after_balance_query` (`#[private]`), `after_storage_deposit` (`#[private]`), `finalize_delete`, `transfer`, `get_config` | Registry-only callability on mutators; self-delete only; FT sweep with NEP-145 storage_deposit and ft_transfer chain; `transfer` swaps the stored owner key for resale while staying Held |
| `src/error.rs` | `LockerError` (Unauthorized) | Typed enum; `FunctionError` impl |

### contracts/resale-locker

| File | Public surface | Notes |
|---|---|---|
| `src/lib.rs` | `new`, `unlock`, `abort`, `get_config` | Registry-only callability; `settled` flag for one-shot exclusivity; recovery_key immutable from init |
| `src/error.rs` | `ResaleLockerError` (Unauthorized, AlreadySettled) | Typed enum; `FunctionError` impl |

### Workspace

| File | Notes |
|---|---|
| `Cargo.toml` | Workspace resolver 2, members include all four crates, `near-sdk = "=5.24.1"` pinned (no `legacy` feature), profile.release configured with `overflow-checks = true` |
| `rust-toolchain.toml` | Pinned to 1.86.0 with rustfmt + clippy + wasm32-unknown-unknown target |

## Verification commands

```bash
# Full quality gate
cd /home/skylar/VScode/hos-tla
cargo fmt --check
cargo clippy --target wasm32-unknown-unknown --release --workspace -- -D warnings
cargo build --target wasm32-unknown-unknown --release --workspace

# Deployable artifact hashes (nearcore-valid; produced by cargo-near)
for f in target/near/{sub_account_locker,resale_locker,tla_manager,tla_registry}/*.wasm; do
    sha256sum "$f"
done
```

Canonical artifacts are produced via `cargo near build non-reproducible-wasm --no-abi` (wasm-opt post-processing required for nearcore VM compatibility); plain `cargo build --target wasm32-unknown-unknown --release` produces WASM that fails `PrepareError(Deserialization)` at deploy time and must never be deployed.

Authoritative freeze captured by the reproducible Docker build (`cargo near build reproducible-wasm`) inside the pinned `sourcescan/cargo-near:0.18.0-rust-1.86.0` image (digest `sha256:2d0d458d2357277df669eac6fa23a1ac922e5ed16646e1d3315336e4dff18043`). Tag `audit-v3.1`. The freeze is layered (leaves at `audit-v3.1~1` = d46224b, bundlers at `audit-v3.1` = 2966c7f) because cargo-near embeds the build commit's rev into every WASM via NEP-330 metadata; the bundlers `include_bytes!` the leaf artifacts from `res/`. See [README.md](README.md) for the auditor's two-step verification.

```
Leaves (bundled in res/, reproducibly built at audit-v3.1~1 = d46224b):
  5b406290a520b8252dd8c6b1cd7c139df4c1f9669e3f105602a1fdb7ffe59042  sub_account_locker.wasm   (111,150 B)
  c09016c12344e3cef85bf61c71fad81fb3729ee4877c0aa4b703f7beefcf6f7e  resale_locker.wasm         (96,655 B)

Bundlers (reproducibly built at audit-v3.1 = 2966c7f):
  38aad2f19bf1850280ffbf53566164ca3d7f3df3d3aece7e594504751776c2a3  tla_manager.wasm          (212,449 B)
  d1784a190c4d6db759dbcba7b54f295dee421f49b61ab30727839c30801fc676  tla_registry.wasm         (428,280 B)
```

The host build is reproducible byte-identical against the Docker reproducible build (`cargo near build reproducible-wasm`) when the same toolchain, image, and source tree are used. The image is pinned in every crate's `[package.metadata.near.reproducible_build]` block to `sourcescan/cargo-near:0.18.0-rust-1.86.0` digest `sha256:2d0d458d2357277df669eac6fa23a1ac922e5ed16646e1d3315336e4dff18043`. The audit-v3.1 freeze hash set above was captured via the reproducible Docker build and is recorded in the annotated `audit-v3.1` tag.

## Integration test suite

`contracts/tla-registry/tests/integration.rs`, 23 scenarios, all passing as of 2026-05-28:

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
test result: ok. 23 passed; 0 failed; finished in 261.14s
```

Coverage: full lifecycle (register, activate, rent), hold-until-export (rented account is held by the locker with the renter key stored not granted; admin export releases it and removes it from registry management), DoS-reclaim fix (mother ownership check), per-TLA business sub-account cap override, pull-payment refund, pause gate, external .near resale locker (unlock and replay block via lock-state machine), and held sub-account resale (list to buy with ownership move, locker key swap, seller pull payment, replay block; buyer-bound accepted offer; authorization and price-floor guards; commission split). Not yet covered by automated tests (manual and threat-model review only): reclaim sweep and finalize end-to-end, retraction schedule/elapse/cancel, resale-locker abort path, 1-yocto guards.

The held sub-account resale primitive (locker `transfer` plus the registry `marketplace.rs` module) was the audit-v3 delta over audit-v2; the sale-time allowlisted-FT asset gate (sweep-first on resale) is the audit-v3.1 delta. Current reproducible hashes are in the freeze block above and in the annotated `audit-v3.1` tag.
