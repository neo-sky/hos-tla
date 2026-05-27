# HoS TLA — Threat Model

Status: V1 (June 2026 audit submission).
Scope: contracts/tla-registry, contracts/tla-manager, contracts/sub-account-locker, contracts/resale-locker.

This document is the auditor-facing consolidation of the trust assumptions, asset-flow paths, invariants, and known limitations across the four contracts. Each section names the threat, the mitigation in code, and the explicit code path that enforces it.

---

## 1. Trust assumptions (the gates above the contracts)

| Assumption | Held by | Failure mode | Recovery |
|---|---|---|---|
| Registry admin set is honest and operationally available | Initial deployer; rotated via `add_admin`/`remove_admin` | Compromised admin can: queue arbitrary withdrawals, clear any user's mother, trigger resale unlock/abort against any locked account | Multisig DAO replaces single-admin model post-V1; documented as required for audit sign-off |
| Manager contract is the parent of the TLA account it serves | Protocol-enforced: `Promise::new(sub_account).create_account()` only succeeds for direct sub-accounts of the predecessor | Manager deployed at wrong account cannot create the TLA's sub-accounts; rentals fail at the cross-contract call | Operational: licensee must deploy manager at the TLA account itself |
| The TLA-resale-Locker WASM embedded in the registry is the audited canonical version | `include_bytes!("../res/resale_locker.wasm")` in the registry source; SHA-256 visible via `get_resale_locker_sha256()` view | If the registry is redeployed with a malicious WASM bundled, all future locks use the malicious code | Buyer-side verification: independently SHA-256 the canonical artifact and compare against the registry's published value before purchasing |
| User-set `main_wallet` and `recovery_key` are valid keys/accounts the user controls | User responsibility at rent / lock setup | User loses funds at reclaim time | Frontend validation; documented |

The contracts hold no trust over admin behavior beyond what the methods explicitly grant. There is no admin-only backdoor that can move user funds outside the documented paths (`withdraw → pending_refunds`, `resale_unlock`, `resale_abort`).

---

## 2. Asset flow paths

Every path NEAR or FTs can travel through the system. Audit reviewer can verify each is gated correctly.

### 2.1 Rental revenue
```
user.wallet --attached_deposit--> tla-registry.rent_sub_account
                                        |
                                        +-- creation_deposit --> tla-manager (consumed by sub-account storage stake)
                                        +-- rent_yocto       --> registry.total_revenue
                                        +-- excess           --> registry.pending_refunds[user]
                                                              -> user.claim_refund pulls
```
- `total_revenue` is incremented only on `on_sub_account_created` success path ([callbacks.rs:23](contracts/tla-registry/src/callbacks.rs#L23)).
- Failure path: `attached_yocto` fully queued to pending_refunds and account creation rolled back ([callbacks.rs:48-71](contracts/tla-registry/src/callbacks.rs#L48-L71)).

### 2.2 Excess refunds
All overpayments on `activate_tla` / `renew_tla` / `renew_sub_account` route through `add_pending_refund` ([lib.rs:188-193](contracts/tla-registry/src/lib.rs#L188-L193)) — never push-transferred. Solvency invariant: `contract_balance >= total_revenue + total_pending_refunds + in_flight`.

### 2.3 Admin withdrawal
```
admin --withdraw(amount, recipient)--> registry.total_revenue -= amount
                                       registry.pending_refunds[recipient] += amount
                                       recipient.claim_refund pulls
```
- Same pull-payment shape as refunds. Even admin withdrawals are queued, never pushed ([admin.rs:162-183](contracts/tla-registry/src/admin.rs#L162-L183)).

### 2.4 Reclaim sweep (sub-account-locker on expiry/retraction)
```
registry.reclaim_sweep_ft(tla, name, ft)
    --[ft_balance_of check elsewhere]
    --> ext_locker(sub_account).sweep_ft(ft, sub.main_wallet)
            -> ft_balance_of(self) -> [callback after_balance_query]
                -> storage_deposit(destination) -> [callback after_storage_deposit]
                    -> ft_transfer(destination, amount)
registry.reclaim_finalize(tla, name)
    --> fan-out ft_balance_of across allowlist
        -> [callback on_balances_checked]
            if all zero -> ext_locker.finalize_delete(destination)
                              -> Promise::new(current).delete_account(destination)
                                  // native NEAR to destination by protocol
            on any nonzero or unverifiable balance -> emit reclaim_finalize_blocked, abort
```
- Native NEAR follows the account, beneficiary receives on `delete_account`.
- FTs are swept BEFORE delete (registry's enforced finalize precondition fails finalize if any balance non-zero).
- Destination is `sub.main_wallet`, user-set at rent time, updatable via `set_main_wallet`.

### 2.5 Resale unlock (resale-locker)
```
admin --resale_unlock(account, buyer_key)--> ext_resale_locker(account).unlock(buyer_key)
                                                  -> AddKey(buyer_key, FullAccess)
                                                  -> settled = true
account is now controlled by buyer_key. NEAR + FTs remain on the account.
```
- No value transfer through the contracts during unlock — only an `AddKey` action emitted by the locker on itself.
- Money flow for the OTC sale happens off-locker (marketplace tier).

### 2.6 Resale abort
```
admin --resale_abort(account)--> ext_resale_locker(account).abort()
                                     -> AddKey(self.recovery_key, FullAccess)
                                     -> settled = true
```
- `recovery_key` is immutable from `new()` — abort cannot redirect to an attacker-controlled key even if admin is compromised, because admin has no input to abort target.

---

## 3. Per-contract invariants

### 3.1 tla-registry
- **Pause gate**: `assert_not_paused()?` precedes every user-facing mutating method (activate, rent, renew, set_main_wallet, schedule_retraction, cancel_retraction, set_mother). Pause does NOT block admin operations or `claim_refund` ([lib.rs:181-186](contracts/tla-registry/src/lib.rs#L181-L186)).
- **Admin gate**: `assert_admin()?` precedes every admin method (register_tla, suspend_tla, unsuspend_tla, add_admin, remove_admin, update_fee_config, withdraw, allowlist mutators, activate_open_tla, admin_clear_mother, resale_unlock, resale_abort) ([lib.rs:174-179](contracts/tla-registry/src/lib.rs#L174-L179)).
- **Solvency**: `total_revenue + total_pending_refunds` is the contract's liability. Mutations always saturating-arithmetic; `claim_refund` decrements `total_pending_refunds` on remove, restores via `add_pending_refund` if the transfer callback fails.
- **Reclaim chokepoint**: a sub-account becomes Reclaimable when ANY of (a) sub's grace elapsed, (b) TLA's grace elapsed (cascade), (c) retraction notice elapsed. Computed by `effective_sub_lifecycle` ([mother.rs:132-147](contracts/tla-registry/src/mother.rs#L132-L147)).
- **Non-sellable mother**: `mother_use_count[sub_account_id] > 0` blocks both `reclaim_sweep_ft` and `reclaim_finalize` ([reclaim.rs:50-58](contracts/tla-registry/src/reclaim.rs#L50-L58)).
- **Business cap**: `business_sub_count[tla] < fee_config.business_max_subs` enforced at `rent_sub_account`; count incremented BEFORE cross-contract call; decremented in callback failure path or on `on_reclaim_finalized` success.

### 3.2 tla-manager
- **Registry-only callability**: `assert_registry()?` precedes `create_sub_account` ([lib.rs:66-71](contracts/tla-manager/src/lib.rs#L66-L71)).
- **Locker WASM bundling**: every new sub-account gets the canonical locker code via `Promise::new(sub).deploy_contract(LOCKER_WASM).function_call("new",...)`. The bundled WASM is `include_bytes!`'d at compile time.
- **No raw `delete_account`**: the dangerous primitive that existed pre-WS1 was removed. Only registry-orchestrated locker calls can delete sub-accounts.
- **Hold-until-export (no renter key granted at creation)**: `create_sub_account` does NOT add a full-access key for the renter. It deploys the locker, passes `owner_key` into `new(registry, owner_key)`, and transfers. The account ends with ZERO access keys, controlled solely by the locker. This is what makes reclaim enforceable — the renter cannot deploy over the locker or otherwise bypass it. The stored `owner_key` is added only on registry-gated `export`.

### 3.3 sub-account-locker
- **Registry-only callability**: `assert_registry()?` on `sweep_ft`, `finalize_delete`, and `export`.
- **Held/Exporting/Exported state machine**: `sweep_ft`, `finalize_delete`, and `export` all require `Held`. `export` transitions `Held → Exporting`, then a `#[private]` callback sets `Exported` on success or reverts to `Held` on failure. Once `Exported`, reclaim is blocked (the account has left HoS); once reclaim deletes the account, export is moot. Terminal states are mutually exclusive.
- **Self-delete only**: `Promise::new(env::current_account_id()).delete_account(destination)` — predecessor == receiver, protocol-valid (no parent-deletes-child path).
- **FT sweep is best-effort but safe**: `after_balance_query` / `after_storage_deposit` use `#[callback_result]` and skip a token gracefully on query or NEP-145 storage-deposit failure. Safety is preserved by the registry's enforced finalize, which blocks `delete_account` while any allowlisted FT balance is nonzero — a skipped sweep leaves the account undeletable, never deleted-with-stranded-funds.
- **Export releases custody**: `export` adds the stored `owner_key` as a full-access key; the renter then fully controls the account and it is removed from registry management (`export_sub_account` is admin-gated for V1 — HoS-mediated; self-service/buyout-priced export is a marketplace-layer follow-up).

### 3.4 resale-locker
- **State immutability post-init**: `registry` and `recovery_key` are stored at `new()` and never changed (no setter exists). `state` is monotone `Active → Settling → Settled` via the callback-confirmed AddKey path.
- **One-shot exclusivity**: `assert_active` gates both `unlock` and `abort`. Once either resolves to `Settled`, the other is permanently blocked.
- **Registry-only callability**: `assert_registry` on both mutating methods.
- **Immutable abort target**: `abort()` takes no arguments; always uses `self.recovery_key`. Admin/registry has no input to the recovery key.
- **No payable methods**: locker holds no value; only emits `AddKey` actions on itself.

**Operational gap — seller key purge (known limitation, marketplace-side mitigation required):** the locker contract cannot introspect its host account's access-key set. It has no way to enforce that the seller deleted their pre-existing keys before listing. If the marketplace accepts a "locked" account without verifying the host-account state, the seller could retain a full-access key alongside the locker and drain assets after buyer settlement.

**Required lock recipe** (the marketplace MUST verify this before accepting a listing; the contract cannot):

1. The seller deploys the canonical resale-locker WASM (matching SHA-256 published in [README.md](README.md)) to the account they want to sell.
2. The seller calls `new(registry, recovery_key)` to initialize the locker.
3. The seller deletes every access key on the account in the same transaction batch.

Marketplace pre-listing verification (off-chain, via NEAR RPC):

- `view_state` returns code_hash matching the canonical locker WASM hash.
- `view_access_key_list` returns an empty list (no full-access or function-call keys remain).
- `get_config` returns the expected `registry` and `recovery_key` and `state = "active"`.

If any of these three checks fails, the marketplace MUST refuse to list the account. A future V2 marketplace contract should bind this verification into the listing-creation flow so the operational gap closes on-chain.

---

## 4. NEAR vulnerability classes — mapping

| Class | Where it would manifest | Mitigation |
|---|---|---|
| Async reentrancy | Between `rent_sub_account` and `on_sub_account_created` | State mutations on the success path are limited to counters (`sub_account_count`, `business_sub_count`, `total_revenue`); the optimistic insert pattern is rolled back on failure. No external calls between read and write of the same field. |
| Public callback exposure | If `on_sub_account_created`/`on_reclaim_finalized`/`on_balances_checked`/`on_claim_refund_settled` could be called by non-self | All four are `#[private]`; near-sdk macro enforces `predecessor == current_account_id`. |
| Storage griefing | If an attacker can fill registry storage | `pending_refunds` and `mothers` use admin's signer for cost; spam-prevention is implicit (caller pays gas+storage). |
| Prefix collisions in collections | If two `StorageKey` variants overlap | Each collection uses a distinct enum variant via `BorshStorageKey`; verified unique. |
| Promise result panics | Failed cross-contract call panics the callback, losing state | `is_promise_success()` is checked branch-style in every callback; failure branches restore state. `promise_result_checked(_, MAX_LEN)` used for hostile contract returns. |
| One-yocto / access-key abuse | If `ft_transfer` were called without the standard one-yocto guard | `with_attached_deposit(ONE_YOCTO)` set on `ft_transfer` in [locker after_storage_deposit](contracts/sub-account-locker/src/lib.rs#L88-L92). |
| Unbounded iteration | If a view iterates the full registry | `list_tlas` is paginated (`from_index`, `limit`); `get_admins` / `get_ft_allowlist` / `get_nft_allowlist` are bounded by admin-controlled set size. |
| Account-deletion fund loss | If a sub-account is deleted with native NEAR or allowlisted FT balance | `delete_account(destination)` transfers native NEAR to destination by protocol; FTs swept first via locker.sweep_ft chain; `reclaim_finalize` enforces all FTs are zero before delete. |
| Float arithmetic | Used in fee calculations | All amounts are u128 yoctoNEAR; multipliers are integer ratios (e.g., PremiumCategory). No floats anywhere. |
| Upgrade gate | Unauthorized upgrade | `migrate()` is admin-gated and version-checked ([lib.rs:147-154](contracts/tla-registry/src/lib.rs#L147-L154)). Locker contracts are intentionally non-upgradable. |

---

## 5. Known limitations (acceptable for V1, documented)

| Limitation | Rationale |
|---|---|
| NFT auto-sweep on expiry is not implemented | NFT contracts have non-uniform storage requirements and pagination, significantly increasing the audit surface. NFTs left in an abandoned-and-lapsed sub-account stay in the NFT contract keyed to a now-deleted account. Frontend warns at listing; documented. |
| Locker non-upgradability | Sub-account locker and resale locker are non-upgradable by design. If a bug is found, every existing locked account is affected. Mitigation: small WASM (~130KB each), 3-4 public methods each, formal-verification candidates. |
| Admin single point of trust | V1 admin is whoever the deploying address designates. Audit sign-off requires moving to multisig DAO. |
| Resale-locker — HoS-defunct bricks accounts | If HoS-admin disappears, locked .near accounts can never be aborted or unlocked. Mitigation: operational responsibility to abort all open locks before shutdown. V2 candidate: time-based fallback abort. |
| Mother going Reclaimable mid-rental | User stops paying rent on their mother sub-account; `set_mother` would have rejected the initial pickup but doesn't auto-clear mid-rental. Escape hatch: `admin_clear_mother(user)`. |
| Storage stake validation on resale-locker deployment | Alice's `alice.near` must hold enough NEAR for the locker WASM storage stake (~1.4 NEAR). If insufficient, the lock tx fails atomically. Documented; frontend should preflight balance. |
| Migrate() handles shape-compatible upgrades only | Shape-breaking V4→V5 migrations would need `#[init(ignore_state)]` reading an `OldRegistry` parallel struct. Current body is correct for the next minor version. |

---

## 6. Test coverage map

Integration tests at [contracts/tla-registry/tests/integration.rs](contracts/tla-registry/tests/integration.rs) cover:

Automated (6 scenarios, all passing):

| Test | Threat covered |
|---|---|
| `test_lifecycle_business_tla` | Happy path: register Business TLA, activate, rent sub-account (manager deploys locker). |
| `test_mother_dos_rejected` | The CRITICAL DoS-reclaim fix: attacker cannot claim victim's sub as their mother (`OnlyOwner` gate). |
| `test_resale_lock_unlock_replay_blocked` | Resale locker unlock + replay prevention via the lock-state machine. |
| `test_pull_payment_refund_excess` | Pull-payment shape for excess on activation. |
| `test_pause_blocks_user_methods` | Pause gate blocks user-facing mutations. |
| `test_business_sub_cap_override` | Per-TLA cap override: set, enforce, raise-on-request. |

Not yet automated — covered by manual reasoning + this threat model, scheduled for the next test pass:
reclaim sweep+finalize end-to-end (enforced-empty precondition), retraction schedule/elapse/cancel-block, resale abort path, 1-yocto guard rejection, storage-reserve guard on `claim_refund`.
