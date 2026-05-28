# HoS TLA Threat Model

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

The contracts hold no trust over admin behavior beyond what the methods explicitly grant. There is no admin-only backdoor that can move user funds outside the documented paths (`withdraw -> pending_refunds`, `resale_unlock`, `resale_abort`). The held-sub-account marketplace adds no admin authority: `list_sub_account`, `unlist_sub_account`, `accept_offer`, and `revoke_offer` are owner-gated (`predecessor == sub.owner`, one-yocto), and `buy_sub_account` is a permissionless buyer fill paying the seller. A compromised admin cannot force-sell or retarget a user's held sub-account.

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
All overpayments on `activate_tla` / `renew_tla` / `renew_sub_account` route through `add_pending_refund` ([lib.rs:188-193](contracts/tla-registry/src/lib.rs#L188-L193)), never push-transferred. Solvency invariant: `contract_balance >= total_revenue + total_pending_refunds + in_flight`.

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
- No value transfer through the contracts during unlock, only an `AddKey` action emitted by the locker on itself.
- Money flow for the OTC sale happens off-locker (marketplace tier).

### 2.6 Resale abort
```
admin --resale_abort(account)--> ext_resale_locker(account).abort()
                                     -> AddKey(self.recovery_key, FullAccess)
                                     -> settled = true
```
- `recovery_key` is immutable from `new()`; abort cannot redirect to an attacker-controlled key even if admin is compromised, because admin has no input to abort target.

### 2.7 Held sub-account resale settlement
```
seller --list_sub_account(tla, name, price) [1 yocto, predecessor==owner]--> registry.listings[key] = {price, settling:false}
  (discovery + offers happen off-chain; the on-chain listing price is the binding floor)
seller --accept_offer(tla, name, buyer, price) [1 yocto, predecessor==owner]--> registry.accepted_offers[key] = {buyer, price, settling:false}
  (optional bound path: only that buyer may fill, at that price, taking precedence over the public listing)
buyer  --buy_sub_account(tla, name, new_owner_key) [attached deposit >= price]--> registry
            resolve_and_lock_sale: pick accepted_offer (if predecessor is the bound buyer) else listing; set settling=true
            --> ext_locker(sub_account).transfer(new_owner_key)   // swaps stored owner_key, account stays Held
                -> [callback on_sub_account_sold]
                    on success:
                        commission       = min(price * resale_commission_bps / 10000, price) --> registry.total_revenue
                        seller_proceeds  = price - commission                                --> pending_refunds[seller]
                        excess           = deposit - price                                   --> pending_refunds[buyer]
                        sub.owner = sub.main_wallet = buyer; remove listing + accepted_offer
                    on failure:
                        deposit --> pending_refunds[buyer] (full refund); clear settling; listing/owner unchanged
```
- No value is push-transferred. Seller proceeds, buyer excess, and failed-sale refunds all route through `pending_refunds` (pull). Commission accrues to `total_revenue` (admin-withdrawable via the same pull path).
- The buyer never receives a key on the account at sale time; `transfer` only swaps the locker's stored `owner_key`. The account stays `Held` with zero keys until a later admin-gated `export`, so reclaim enforceability is unchanged.
- Resale does NOT delete the account, but settlement enforces the same allowlisted-FT sweep-first scope as reclaim: `buy_sub_account` fans out `ft_balance_of` across the FT allowlist and refuses to settle (refunding the buyer in full) while any allowlisted FT balance is present, or while a balance is unverifiable (fail-closed). The buyer inherits only native NEAR and out-of-allowlist assets; NFTs remain the documented warn-only limitation. Sellers sweep assets out before selling; the frontend warns at listing.

---

## 3. Per-contract invariants

### 3.1 tla-registry
- **Pause gate**: `assert_not_paused()?` precedes every user-facing mutating method (activate, rent, renew, set_main_wallet, schedule_retraction, cancel_retraction, set_mother). Pause does NOT block admin operations or `claim_refund` ([lib.rs:181-186](contracts/tla-registry/src/lib.rs#L181-L186)).
- **Admin gate**: `assert_admin()?` precedes every admin method (register_tla, suspend_tla, unsuspend_tla, add_admin, remove_admin, update_fee_config, withdraw, allowlist mutators, activate_open_tla, admin_clear_mother, resale_unlock, resale_abort) ([lib.rs:174-179](contracts/tla-registry/src/lib.rs#L174-L179)).
- **Solvency**: `total_revenue + total_pending_refunds` is the contract's liability. Mutations always saturating-arithmetic; `claim_refund` decrements `total_pending_refunds` on remove, restores via `add_pending_refund` if the transfer callback fails.
- **Reclaim chokepoint**: a sub-account becomes Reclaimable when ANY of (a) sub's grace elapsed, (b) TLA's grace elapsed (cascade), (c) retraction notice elapsed. Computed by `effective_sub_lifecycle` ([mother.rs:132-147](contracts/tla-registry/src/mother.rs#L132-L147)).
- **Non-sellable mother**: `mother_use_count[sub_account_id] > 0` blocks both `reclaim_sweep_ft` and `reclaim_finalize` ([reclaim.rs:50-58](contracts/tla-registry/src/reclaim.rs#L50-L58)).
- **Business cap**: `business_sub_count[tla] < fee_config.business_max_subs` enforced at `rent_sub_account`; count incremented BEFORE cross-contract call; decremented in callback failure path or on `on_reclaim_finalized` success.
- **Resale unsellable gate**: `assert_sellable` requires the parent TLA status be `Active` (a Suspended TLA cannot have its sub-accounts traded) and `effective_sub_lifecycle == Active`, and it rejects mothers (`mother_use_count > 0`) and retraction-pending subs (`retraction_at.is_some()`). Enforced at list, accept_offer, and re-checked at buy time so a lifecycle drift between listing and fill cannot settle a no-longer-sellable account ([marketplace.rs](contracts/tla-registry/src/marketplace.rs)).
- **Resale settlement asset gate**: `buy_sub_account` does not transfer until an `ft_balance_of` fan-out across the FT allowlist confirms the account is empty of allowlisted tokens; any non-zero or unverifiable balance aborts the sale, refunds the buyer via `pending_refunds`, clears the settling lock, and emits `sub_account_sale_blocked` ([marketplace.rs](contracts/tla-registry/src/marketplace.rs) `on_buy_balances_checked`). Mirrors `reclaim_finalize` and closes the canonical Decision #9 laundering vector on the sale path. The callback is panic-free: a malformed, failed, unverifiable, or count-mismatched balance response is treated as a block (buyer fully refunded), so a held buyer deposit is never stranded and the listing lock is never wedged by a hostile or buggy allowlisted FT.
- **Resale settlement lock**: a per-account `settling` flag on the listing/accepted-offer blocks a second concurrent fill (`assert_sale_idle`); set in `resolve_and_lock_sale` before the cross-contract `transfer`, cleared on callback failure, and the entries removed on callback success. State (ownership move, seller payout, commission) is finalized ONLY in the `is_promise_success()` branch of `on_sub_account_sold`.
- **Resale price authority**: the seller's on-chain listing price is the binding floor; `buy_sub_account` enforces `deposit >= price`. There is no signature-based off-chain order, because a contract cannot prove that an off-chain signature came from a live full-access key on the seller's account (NEAR accounts are not keys). Below-floor sales require either a re-list or an on-chain `accept_offer` bound to a specific buyer.
- **Sale-entry purge**: `on_reclaim_finalized` and `on_export_settled` remove the sub's `listings` and `accepted_offers` entries, so an account leaving registry management cannot leave an orphaned sale record ([reclaim.rs](contracts/tla-registry/src/reclaim.rs)).
- **Commission bound**: `resale_commission_bps` is validated `<= 10000` in `update_fee_config`, and the split additionally clamps `commission = min(price * bps / 10000, price)`, so the credited commission can never exceed the price the buyer paid (no insolvency from a misconfigured rate).

### 3.2 tla-manager
- **Registry-only callability**: `assert_registry()?` precedes `create_sub_account` ([lib.rs:66-71](contracts/tla-manager/src/lib.rs#L66-L71)).
- **Locker WASM bundling**: every new sub-account gets the canonical locker code via `Promise::new(sub).deploy_contract(LOCKER_WASM).function_call("new",...)`. The bundled WASM is `include_bytes!`'d at compile time.
- **No raw `delete_account`**: the dangerous primitive that existed pre-WS1 was removed. Only registry-orchestrated locker calls can delete sub-accounts.
- **Hold-until-export (no renter key granted at creation)**: `create_sub_account` does NOT add a full-access key for the renter. It deploys the locker, passes `owner_key` into `new(registry, owner_key)`, and transfers. The account ends with ZERO access keys, controlled solely by the locker. This is what makes reclaim enforceable: the renter cannot deploy over the locker or otherwise bypass it. The stored `owner_key` is added only on registry-gated `export`.

### 3.3 sub-account-locker
- **Registry-only callability**: `assert_registry()?` on `sweep_ft`, `finalize_delete`, `export`, and `transfer`.
- **Held/Exporting/Exported state machine**: `sweep_ft`, `finalize_delete`, `export`, and `transfer` all require `Held`. `export` transitions `Held -> Exporting`, then a `#[private]` callback sets `Exported` on success or reverts to `Held` on failure. Once `Exported`, reclaim is blocked (the account has left HoS); once reclaim deletes the account, export is moot. Terminal states are mutually exclusive.
- **Resale transfer keeps custody**: `transfer(new_owner_key)` swaps the stored `owner_key` and leaves the state `Held`. It emits no `AddKey`/`DeleteKey` (the account has zero keys), so it is a synchronous state write with no value movement. The new owner's key is added only on a later registry-gated `export`. Reclaim remains enforceable throughout a resale.
- **Self-delete only**: `Promise::new(env::current_account_id()).delete_account(destination)`, predecessor == receiver, protocol-valid (no parent-deletes-child path).
- **FT sweep is best-effort but safe**: `after_balance_query` / `after_storage_deposit` use `#[callback_result]` and skip a token gracefully on query or NEP-145 storage-deposit failure. Safety is preserved by the registry's enforced finalize, which blocks `delete_account` while any allowlisted FT balance is nonzero, so a skipped sweep leaves the account undeletable, never deleted-with-stranded-funds.
- **Export releases custody**: `export` adds the stored `owner_key` as a full-access key; the renter then fully controls the account and it is removed from registry management (`export_sub_account` is admin-gated for V1, HoS-mediated; self-service/buyout-priced export is a marketplace-layer follow-up).

### 3.4 resale-locker
- **State immutability post-init**: `registry` and `recovery_key` are stored at `new()` and never changed (no setter exists). `state` is monotone `Active -> Settling -> Settled` via the callback-confirmed AddKey path.
- **One-shot exclusivity**: `assert_active` gates both `unlock` and `abort`. Once either resolves to `Settled`, the other is permanently blocked.
- **Registry-only callability**: `assert_registry` on both mutating methods.
- **Immutable abort target**: `abort()` takes no arguments; always uses `self.recovery_key`. Admin/registry has no input to the recovery key.
- **No payable methods**: locker holds no value; only emits `AddKey` actions on itself.

**Operational gap, seller key purge (known limitation, marketplace-side mitigation required):** the locker contract cannot introspect its host account's access-key set. It has no way to enforce that the seller deleted their pre-existing keys before listing. If the marketplace accepts a "locked" account without verifying the host-account state, the seller could retain a full-access key alongside the locker and drain assets after buyer settlement.

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

## 4. NEAR vulnerability class mapping

| Class | Where it would manifest | Mitigation |
|---|---|---|
| Async reentrancy | Between `rent_sub_account` and `on_sub_account_created`; between `buy_sub_account` and `on_sub_account_sold` | State mutations on the success path are limited to counters (`sub_account_count`, `business_sub_count`, `total_revenue`); the optimistic insert pattern is rolled back on failure. For resale, the `settling` flag locks the listing/offer across the cross-contract gap; ownership move, seller payout, and commission are written only in the success branch. No external calls between read and write of the same field. |
| Public callback exposure | If `on_sub_account_created`/`on_reclaim_finalized`/`on_balances_checked`/`on_claim_refund_settled`/`on_sub_account_sold` could be called by non-self | All are `#[private]`; near-sdk macro enforces `predecessor == current_account_id`. |
| Settlement authorization spoofing | If a buyer could acquire an account without the seller's on-chain authorization, or below the seller's floor | Authorization is on-chain only: seller's `list`/`accept_offer` (one-yocto, `predecessor == owner`) sets the price; `buy_sub_account` enforces `deposit >= price` and an accepted offer is fillable only by its bound buyer. No off-chain signature path exists (a contract cannot prove a signature maps to a live full-access key on the seller's account). |
| Storage griefing | If an attacker can fill registry storage | `pending_refunds` and `mothers` use admin's signer for cost; spam-prevention is implicit (caller pays gas+storage). |
| Prefix collisions in collections | If two `StorageKey` variants overlap | Each collection uses a distinct enum variant via `BorshStorageKey`; verified unique. |
| Promise result panics | Failed cross-contract call panics the callback, losing state | `is_promise_success()` is checked branch-style in every callback; failure branches restore state. `promise_result_checked(_, MAX_LEN)` used for hostile contract returns. |
| One-yocto / access-key abuse | If `ft_transfer` were called without the standard one-yocto guard | `with_attached_deposit(ONE_YOCTO)` set on `ft_transfer` in [locker after_storage_deposit](contracts/sub-account-locker/src/lib.rs#L88-L92). |
| Unbounded iteration / gas-bound fan-out | If a view iterates the full registry, or a cross-contract fan-out exceeds the 300 Tgas per-transaction cap | `list_tlas` is paginated (`from_index`, `limit`); `get_admins` / `get_ft_allowlist` / `get_nft_allowlist` are bounded by admin-controlled set size. The `ft_balance_of` fan-out in `reclaim_finalize` and `buy_sub_account` is bounded by `MAX_ALLOWLIST_SIZE = 40`, sized so the queries plus callback stay under 300 Tgas (40 x 5 Tgas + callback is about 270 Tgas); a larger cap would make reclaim and sale uncallable. |
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
| Resale-locker, HoS-defunct bricks accounts | If HoS-admin disappears, locked .near accounts can never be aborted or unlocked. Mitigation: operational responsibility to abort all open locks before shutdown. V2 candidate: time-based fallback abort. |
| Mother going Reclaimable mid-rental | User stops paying rent on their mother sub-account; `set_mother` would have rejected the initial pickup but doesn't auto-clear mid-rental. Escape hatch: `admin_clear_mother(user)`. |
| Storage stake validation on resale-locker deployment | Alice's `alice.near` must hold enough NEAR for the locker WASM storage stake (~1.4 NEAR). If insufficient, the lock tx fails atomically. Documented; frontend should preflight balance. |
| Migrate() handles shape-compatible upgrades only | Shape-breaking migrations would need `#[init(ignore_state)]` reading an `OldRegistry` parallel struct. The v6 `FeeConfig` gained `resale_commission_bps`, which changes the borsh layout, so v6 must be deployed fresh or upgraded with a state-reconstructing migration, not the plain version-bump `migrate()`. |
| Resale NFT residual | `buy_sub_account` blocks settlement while allowlisted FT balances are present (sweep-first), but NFTs are not balance-checked (same scope as reclaim). An account sold could still carry an NFT; sellers move assets out before selling and the frontend warns at listing. |

---

## 6. Test coverage map

Integration tests at [contracts/tla-registry/tests/integration.rs](contracts/tla-registry/tests/integration.rs) cover:

Automated (23 scenarios, all passing):

| Test | Threat covered |
|---|---|
| `test_lifecycle_business_tla` | Happy path: register Business TLA, activate, rent sub-account (manager deploys locker). |
| `test_hold_until_export` | Rented account is held with zero keys; locker stores the renter key; admin export releases it and removes it from registry management. |
| `test_mother_dos_rejected` | The CRITICAL DoS-reclaim fix: attacker cannot claim victim's sub as their mother (`OnlyOwner` gate). |
| `test_mother_pre_squat_does_not_block_future_sub` | Pre-squatting a non-managed account does not bump `mother_use_count`, so a future sub of that name stays reclaimable. |
| `test_resale_lock_unlock_replay_blocked` | Resale locker (external .near) unlock + replay prevention via the lock-state machine. |
| `test_pull_payment_refund_excess` | Pull-payment shape for excess on activation. |
| `test_pause_blocks_user_methods` | Pause gate blocks user-facing mutations. |
| `test_business_sub_cap_override` | Per-TLA cap override: set, enforce, raise-on-request. |
| `test_resale_list_buy_transfers_and_pays` | Held-sub resale: list -> buy moves ownership, swaps the locker key while staying Held, credits seller via pull payment, clears the listing, and blocks a replay buy. |
| `test_resale_accepted_offer_bound_to_buyer` | An accepted offer is fillable only by its bound buyer; a non-bound buyer with no public listing hits `NotListed`. |
| `test_resale_authorization_guards` | Non-owner cannot list; a deposit below the listed price is rejected (`PriceNotMet`). |
| `test_resale_commission_split` | With `resale_commission_bps = 250`, the seller receives price minus 2.5% and `total_revenue` rises by exactly the commission. |
| `test_resale_blocked_when_tla_suspended` | A sub-account under a Suspended parent TLA cannot be listed (`SubAccountNotSellable`). |
| `test_resale_buy_refunds_excess` | Overpayment above the price is refunded to the buyer via `pending_refunds`. |
| `test_resale_list_requires_one_yocto` | `list_sub_account` requires the one-yocto full-access-key proof. |
| `test_resale_mother_not_sellable` | A mother/anchor account (`mother_use_count > 0`) cannot be listed (`SubAccountIsMother`). |
| `test_resale_pause_blocks_marketplace` | Pause blocks both `list_sub_account` and `buy_sub_account`. |
| `test_resale_relist_updates_price` | Re-listing overwrites the price; the old price no longer satisfies the fill, the new one does. |
| `test_resale_retraction_blocks_sale` | A sub-account with a pending retraction cannot be listed (`RetractionPending`). |
| `test_resale_revoke_offer_blocks_buyer` | A revoked accepted offer can no longer be filled by the previously bound buyer. |
| `test_resale_unlist_clears_sale` | Owner unlist clears the listing (non-owner blocked); a post-unlist buy hits `NotListed`. |
| `test_resale_zero_price_rejected` | Zero-price `list_sub_account` and `accept_offer` are rejected (`InvalidPrice`). |
| `test_resale_blocked_while_assets_unverifiable` | With an allowlisted token whose balance cannot be confirmed zero, `buy_sub_account` blocks the sale (ownership unchanged) and refunds the buyer in full (sweep-first, fail-closed). |

Not yet automated, covered by manual reasoning and this threat model, scheduled for the next test pass:
reclaim sweep and finalize end-to-end (enforced-empty precondition), retraction elapse and post-elapse cancel-block, resale-locker abort path, storage-reserve guard on `claim_refund`.
