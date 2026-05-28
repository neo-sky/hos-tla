use anyhow::Result;
use near_workspaces::network::Sandbox;
use near_workspaces::types::{KeyType, NearToken};
use near_workspaces::{Account, Contract, Worker};
use serde_json::json;

const REGISTRY_WASM: &str = "../../target/near/tla_registry/tla_registry.wasm";
const MANAGER_WASM: &str = "../../target/near/tla_manager/tla_manager.wasm";
const RESALE_LOCKER_WASM: &str = "../../target/near/resale_locker/resale_locker.wasm";

const ONE_NEAR_YOCTO: u128 = 1_000_000_000_000_000_000_000_000;

struct Harness {
    worker: Worker<Sandbox>,
    admin: Account,
    registry: Contract,
}

async fn setup() -> Result<Harness> {
    let worker = near_workspaces::sandbox().await?;
    let registry_wasm = std::fs::read(REGISTRY_WASM)?;
    let registry = worker.dev_deploy(&registry_wasm).await?;
    let admin = worker.dev_create_account().await?;
    registry
        .call("new")
        .args_json(json!({ "admin": admin.id() }))
        .transact()
        .await?
        .into_result()?;
    Ok(Harness {
        worker,
        admin,
        registry,
    })
}

async fn deploy_manager_at_tla(
    worker: &Worker<Sandbox>,
    registry_id: &near_workspaces::AccountId,
    tla_name: &str,
) -> Result<Account> {
    let root = worker.root_account()?;
    let tla = root
        .create_subaccount(tla_name)
        .initial_balance(NearToken::from_near(50))
        .transact()
        .await?
        .into_result()?;
    let manager_wasm = std::fs::read(MANAGER_WASM)?;
    tla.deploy(&manager_wasm).await?.into_result()?;
    tla.call(tla.id(), "new")
        .args_json(json!({ "registry": registry_id }))
        .transact()
        .await?
        .into_result()?;
    Ok(tla)
}

#[tokio::test]
async fn test_lifecycle_business_tla() -> Result<()> {
    let h = setup().await?;

    let tla = deploy_manager_at_tla(&h.worker, h.registry.id(), "sovereign").await?;
    let licensee = h
        .worker
        .root_account()?
        .create_subaccount("licensee")
        .initial_balance(NearToken::from_near(2000))
        .transact()
        .await?
        .into_result()?;

    h.admin
        .call(h.registry.id(), "register_tla")
        .args_json(json!({
            "tla_id": tla.id(),
            "tla_type": "Business",
            "premium_category": "Premium",
            "licensee": licensee.id(),
        }))
        .transact()
        .await?
        .into_result()?;

    let activate = licensee
        .call(h.registry.id(), "activate_tla")
        .args_json(json!({ "tla_id": tla.id() }))
        .deposit(NearToken::from_near(1500))
        .transact()
        .await?;
    assert!(activate.is_success(), "activate_tla failed: {:?}", activate);

    let view: serde_json::Value = h
        .registry
        .view("get_tla")
        .args_json(json!({ "tla_id": tla.id() }))
        .await?
        .json()?;
    assert_eq!(view["lifecycle"], "Active");

    let owner = h
        .worker
        .root_account()?
        .create_subaccount("alice")
        .initial_balance(NearToken::from_near(100))
        .transact()
        .await?
        .into_result()?;

    let owner_key = near_workspaces::types::SecretKey::from_random(KeyType::ED25519);
    let owner_pk = owner_key.public_key();

    let rent = licensee
        .call(h.registry.id(), "rent_sub_account")
        .args_json(json!({
            "tla_id": tla.id(),
            "name": "alice",
            "owner_key": owner_pk,
            "main_wallet": owner.id(),
        }))
        .deposit(NearToken::from_near(10))
        .max_gas()
        .transact()
        .await?;
    assert!(rent.is_success(), "rent_sub_account failed: {:?}", rent);

    let sub_view: Option<serde_json::Value> = h
        .registry
        .view("get_sub_account")
        .args_json(json!({ "tla_id": tla.id(), "name": "alice" }))
        .await?
        .json()?;
    assert!(sub_view.is_some(), "sub-account entry should exist");

    Ok(())
}

#[tokio::test]
async fn test_mother_dos_rejected() -> Result<()> {
    let h = setup().await?;

    let tla = deploy_manager_at_tla(&h.worker, h.registry.id(), "shire").await?;
    let licensee = h
        .worker
        .root_account()?
        .create_subaccount("licensee2")
        .initial_balance(NearToken::from_near(2000))
        .transact()
        .await?
        .into_result()?;

    h.admin
        .call(h.registry.id(), "register_tla")
        .args_json(json!({
            "tla_id": tla.id(),
            "tla_type": "Business",
            "premium_category": "Standard",
            "licensee": licensee.id(),
        }))
        .transact()
        .await?
        .into_result()?;

    licensee
        .call(h.registry.id(), "activate_tla")
        .args_json(json!({ "tla_id": tla.id() }))
        .deposit(NearToken::from_near(1500))
        .transact()
        .await?
        .into_result()?;

    let owner_pk = near_workspaces::types::SecretKey::from_random(KeyType::ED25519).public_key();
    let main_wallet = h
        .worker
        .root_account()?
        .create_subaccount("bob")
        .initial_balance(NearToken::from_near(5))
        .transact()
        .await?
        .into_result()?;

    licensee
        .call(h.registry.id(), "rent_sub_account")
        .args_json(json!({
            "tla_id": tla.id(),
            "name": "bob",
            "owner_key": owner_pk,
            "main_wallet": main_wallet.id(),
        }))
        .deposit(NearToken::from_near(10))
        .max_gas()
        .transact()
        .await?
        .into_result()?;

    let attacker = h
        .worker
        .root_account()?
        .create_subaccount("attacker")
        .initial_balance(NearToken::from_near(10))
        .transact()
        .await?
        .into_result()?;

    let bob_sub_id: near_workspaces::AccountId = format!("bob.{}", tla.id()).parse()?;
    let evil = attacker
        .call(h.registry.id(), "set_mother")
        .args_json(json!({ "new_mother": bob_sub_id }))
        .deposit(NearToken::from_yoctonear(1))
        .transact()
        .await?;
    assert!(
        evil.is_failure(),
        "attacker must NOT be able to claim someone else's sub as their mother"
    );
    let err_str = format!("{:?}", evil);
    assert!(
        err_str.contains("only_owner") || err_str.contains("OnlyOwner"),
        "expected OnlyOwner failure, got: {}",
        err_str
    );

    Ok(())
}

#[tokio::test]
async fn test_pull_payment_refund_excess() -> Result<()> {
    let h = setup().await?;

    let tla = deploy_manager_at_tla(&h.worker, h.registry.id(), "valley").await?;
    let licensee = h
        .worker
        .root_account()?
        .create_subaccount("licensee3")
        .initial_balance(NearToken::from_near(2000))
        .transact()
        .await?
        .into_result()?;

    h.admin
        .call(h.registry.id(), "register_tla")
        .args_json(json!({
            "tla_id": tla.id(),
            "tla_type": "Business",
            "premium_category": "Community",
            "licensee": licensee.id(),
        }))
        .transact()
        .await?
        .into_result()?;

    licensee
        .call(h.registry.id(), "activate_tla")
        .args_json(json!({ "tla_id": tla.id() }))
        .deposit(NearToken::from_near(1800))
        .transact()
        .await?
        .into_result()?;

    let pending: serde_json::Value = h
        .registry
        .view("get_pending_refund")
        .args_json(json!({ "account_id": licensee.id() }))
        .await?
        .json()?;
    let pending_yocto: u128 = pending.as_str().unwrap().parse()?;
    assert!(
        pending_yocto > 0,
        "overpayment should have queued a refund, got 0"
    );

    let total_pending: serde_json::Value = h
        .registry
        .view("get_total_pending_refunds")
        .args_json(json!({}))
        .await?
        .json()?;
    let total: u128 = total_pending.as_str().unwrap().parse()?;
    assert_eq!(
        total, pending_yocto,
        "total_pending_refunds should equal the single user's refund"
    );

    let before = licensee.view_account().await?.balance.as_yoctonear();
    let claim = licensee
        .call(h.registry.id(), "claim_refund")
        .args_json(json!({}))
        .max_gas()
        .transact()
        .await?;
    assert!(claim.is_success(), "claim_refund failed: {:?}", claim);

    let after = licensee.view_account().await?.balance.as_yoctonear();
    assert!(
        after > before,
        "balance should have increased after claim (before {}, after {})",
        before,
        after
    );

    let post_total: serde_json::Value = h
        .registry
        .view("get_total_pending_refunds")
        .args_json(json!({}))
        .await?
        .json()?;
    let post: u128 = post_total.as_str().unwrap().parse()?;
    assert_eq!(post, 0, "total_pending_refunds should be 0 after claim");

    Ok(())
}

#[tokio::test]
async fn test_pause_blocks_user_methods() -> Result<()> {
    let h = setup().await?;

    h.admin
        .call(h.registry.id(), "pause")
        .args_json(json!({}))
        .transact()
        .await?
        .into_result()?;

    let owner_pk = near_workspaces::types::SecretKey::from_random(KeyType::ED25519).public_key();
    let bystander = h.worker.dev_create_account().await?;

    let attempt = bystander
        .call(h.registry.id(), "rent_sub_account")
        .args_json(json!({
            "tla_id": "nonexistent.test.near",
            "name": "x",
            "owner_key": owner_pk,
            "main_wallet": bystander.id(),
        }))
        .deposit(NearToken::from_yoctonear(ONE_NEAR_YOCTO * 5))
        .max_gas()
        .transact()
        .await?;
    assert!(attempt.is_failure(), "paused contract must reject rent");
    let err = format!("{:?}", attempt);
    assert!(
        err.contains("paused") || err.contains("Paused"),
        "expected Paused error, got: {}",
        err
    );

    Ok(())
}

#[tokio::test]
async fn test_resale_lock_unlock_replay_blocked() -> Result<()> {
    let h = setup().await?;

    let alice = h
        .worker
        .root_account()?
        .create_subaccount("alice-resale")
        .initial_balance(NearToken::from_near(10))
        .transact()
        .await?
        .into_result()?;

    let alice_pk = alice.secret_key().public_key();

    let resale_wasm = std::fs::read(RESALE_LOCKER_WASM)?;
    let lock = alice
        .batch(alice.id())
        .deploy(&resale_wasm)
        .call(
            near_workspaces::operations::Function::new("new")
                .args_json(json!({
                    "registry": h.registry.id(),
                    "recovery_key": alice_pk,
                }))
                .gas(near_workspaces::types::Gas::from_tgas(50)),
        )
        .delete_key(alice_pk.clone())
        .transact()
        .await?;
    assert!(lock.is_success(), "lock setup failed: {:?}", lock);

    let buyer_pk = near_workspaces::types::SecretKey::from_random(KeyType::ED25519).public_key();

    let unlock = h
        .admin
        .call(h.registry.id(), "resale_unlock")
        .args_json(json!({ "account": alice.id(), "buyer_key": buyer_pk }))
        .max_gas()
        .transact()
        .await?;
    assert!(unlock.is_success(), "resale_unlock failed: {:?}", unlock);

    let replay = h
        .admin
        .call(h.registry.id(), "resale_unlock")
        .args_json(json!({ "account": alice.id(), "buyer_key": buyer_pk }))
        .max_gas()
        .transact()
        .await?;
    assert!(
        replay.is_failure(),
        "second unlock must fail (settled flag)"
    );
    let err = format!("{:?}", replay);
    assert!(
        err.contains("already_settled") || err.contains("AlreadySettled"),
        "expected AlreadySettled error, got: {}",
        err
    );

    Ok(())
}

#[tokio::test]
async fn test_business_sub_cap_override() -> Result<()> {
    let h = setup().await?;

    let tla = deploy_manager_at_tla(&h.worker, h.registry.id(), "guild").await?;
    let licensee = h
        .worker
        .root_account()?
        .create_subaccount("licensee3")
        .initial_balance(NearToken::from_near(2000))
        .transact()
        .await?
        .into_result()?;

    h.admin
        .call(h.registry.id(), "register_tla")
        .args_json(json!({
            "tla_id": tla.id(),
            "tla_type": "Business",
            "premium_category": "Standard",
            "licensee": licensee.id(),
        }))
        .transact()
        .await?
        .into_result()?;

    licensee
        .call(h.registry.id(), "activate_tla")
        .args_json(json!({ "tla_id": tla.id() }))
        .deposit(NearToken::from_near(1500))
        .transact()
        .await?
        .into_result()?;

    h.admin
        .call(h.registry.id(), "set_business_sub_cap")
        .args_json(json!({ "tla_id": tla.id(), "cap": 1u32 }))
        .transact()
        .await?
        .into_result()?;

    let cap: u32 = h
        .registry
        .view("get_business_sub_cap")
        .args_json(json!({ "tla_id": tla.id() }))
        .await?
        .json()?;
    assert_eq!(cap, 1, "override cap should be 1");

    let first_key = near_workspaces::types::SecretKey::from_random(KeyType::ED25519).public_key();
    let first = licensee
        .call(h.registry.id(), "rent_sub_account")
        .args_json(json!({
            "tla_id": tla.id(),
            "name": "one",
            "owner_key": first_key,
            "main_wallet": licensee.id(),
        }))
        .deposit(NearToken::from_near(10))
        .max_gas()
        .transact()
        .await?;
    assert!(first.is_success(), "first rent should succeed: {:?}", first);

    let second_key = near_workspaces::types::SecretKey::from_random(KeyType::ED25519).public_key();
    let blocked = licensee
        .call(h.registry.id(), "rent_sub_account")
        .args_json(json!({
            "tla_id": tla.id(),
            "name": "two",
            "owner_key": second_key,
            "main_wallet": licensee.id(),
        }))
        .deposit(NearToken::from_near(10))
        .max_gas()
        .transact()
        .await?;
    assert!(blocked.is_failure(), "second rent must hit the cap");
    let err = format!("{:?}", blocked);
    assert!(
        err.contains("max_business_subs_reached") || err.contains("MaxBusinessSubsReached"),
        "expected MaxBusinessSubsReached, got: {}",
        err
    );

    h.admin
        .call(h.registry.id(), "set_business_sub_cap")
        .args_json(json!({ "tla_id": tla.id(), "cap": 2u32 }))
        .transact()
        .await?
        .into_result()?;

    let raised = licensee
        .call(h.registry.id(), "rent_sub_account")
        .args_json(json!({
            "tla_id": tla.id(),
            "name": "two",
            "owner_key": second_key,
            "main_wallet": licensee.id(),
        }))
        .deposit(NearToken::from_near(10))
        .max_gas()
        .transact()
        .await?;
    assert!(
        raised.is_success(),
        "rent should succeed after cap raised: {:?}",
        raised
    );

    Ok(())
}

#[tokio::test]
async fn test_hold_until_export() -> Result<()> {
    let h = setup().await?;

    let tla = deploy_manager_at_tla(&h.worker, h.registry.id(), "vault").await?;
    let licensee = h
        .worker
        .root_account()?
        .create_subaccount("licensee4")
        .initial_balance(NearToken::from_near(2000))
        .transact()
        .await?
        .into_result()?;

    h.admin
        .call(h.registry.id(), "register_tla")
        .args_json(json!({
            "tla_id": tla.id(),
            "tla_type": "Business",
            "premium_category": "Standard",
            "licensee": licensee.id(),
        }))
        .transact()
        .await?
        .into_result()?;

    licensee
        .call(h.registry.id(), "activate_tla")
        .args_json(json!({ "tla_id": tla.id() }))
        .deposit(NearToken::from_near(1500))
        .transact()
        .await?
        .into_result()?;

    let owner_pk = near_workspaces::types::SecretKey::from_random(KeyType::ED25519).public_key();
    licensee
        .call(h.registry.id(), "rent_sub_account")
        .args_json(json!({
            "tla_id": tla.id(),
            "name": "carol",
            "owner_key": owner_pk,
            "main_wallet": licensee.id(),
        }))
        .deposit(NearToken::from_near(10))
        .max_gas()
        .transact()
        .await?
        .into_result()?;

    let sub_id: near_workspaces::AccountId = format!("carol.{}", tla.id()).parse()?;

    let cfg: serde_json::Value = h.worker.view(&sub_id, "get_config").await?.json()?;
    assert_eq!(
        cfg["state"], "held",
        "rented sub-account must be held by the locker, not handed to the renter"
    );
    assert_eq!(
        cfg["owner_key"],
        serde_json::Value::String(owner_pk.to_string()),
        "locker must store the renter key for later export"
    );

    h.admin
        .call(h.registry.id(), "export_sub_account")
        .args_json(json!({ "tla_id": tla.id(), "name": "carol" }))
        .max_gas()
        .transact()
        .await?
        .into_result()?;

    let after: Option<serde_json::Value> = h
        .registry
        .view("get_sub_account")
        .args_json(json!({ "tla_id": tla.id(), "name": "carol" }))
        .await?
        .json()?;
    assert!(
        after.is_none(),
        "exported sub-account must leave registry management (no longer reclaimable)"
    );

    let cfg2: serde_json::Value = h.worker.view(&sub_id, "get_config").await?.json()?;
    assert_eq!(
        cfg2["state"], "exported",
        "locker must be in exported terminal state"
    );

    Ok(())
}

#[tokio::test]
async fn test_mother_pre_squat_does_not_block_future_sub() -> Result<()> {
    let h = setup().await?;

    let attacker = h
        .worker
        .root_account()?
        .create_subaccount("squatter")
        .initial_balance(NearToken::from_near(5))
        .transact()
        .await?
        .into_result()?;

    let future_sub = "future.notarealtla.test.near";
    attacker
        .call(h.registry.id(), "set_mother")
        .args_json(json!({ "new_mother": future_sub }))
        .deposit(NearToken::from_yoctonear(1))
        .transact()
        .await?
        .into_result()?;

    let count: u32 = h
        .registry
        .view("get_mother_use_count")
        .args_json(json!({ "account": future_sub }))
        .await?
        .json()?;
    assert_eq!(
        count, 0,
        "pre-squatting a non-managed account must NOT bump mother_use_count; otherwise a future sub of that name would be unreclaimable"
    );

    let mother: Option<String> = h
        .registry
        .view("get_mother")
        .args_json(json!({ "user": attacker.id() }))
        .await?
        .json()?;
    assert_eq!(
        mother.as_deref(),
        Some(future_sub),
        "the user's declared mother is still recorded; only the count is gated"
    );

    Ok(())
}

async fn rent_business_sub(
    h: &Harness,
    tla_name: &str,
    licensee_name: &str,
    sub_name: &str,
) -> Result<(Account, Account)> {
    let tla = deploy_manager_at_tla(&h.worker, h.registry.id(), tla_name).await?;
    let licensee = h
        .worker
        .root_account()?
        .create_subaccount(licensee_name)
        .initial_balance(NearToken::from_near(2000))
        .transact()
        .await?
        .into_result()?;

    h.admin
        .call(h.registry.id(), "register_tla")
        .args_json(json!({
            "tla_id": tla.id(),
            "tla_type": "Business",
            "premium_category": "Standard",
            "licensee": licensee.id(),
        }))
        .transact()
        .await?
        .into_result()?;

    licensee
        .call(h.registry.id(), "activate_tla")
        .args_json(json!({ "tla_id": tla.id() }))
        .deposit(NearToken::from_near(1500))
        .transact()
        .await?
        .into_result()?;

    let owner_pk = near_workspaces::types::SecretKey::from_random(KeyType::ED25519).public_key();
    licensee
        .call(h.registry.id(), "rent_sub_account")
        .args_json(json!({
            "tla_id": tla.id(),
            "name": sub_name,
            "owner_key": owner_pk,
            "main_wallet": licensee.id(),
        }))
        .deposit(NearToken::from_near(10))
        .max_gas()
        .transact()
        .await?
        .into_result()?;

    Ok((tla, licensee))
}

#[tokio::test]
async fn test_resale_list_buy_transfers_and_pays() -> Result<()> {
    let h = setup().await?;
    let (tla, seller) = rent_business_sub(&h, "market", "seller-a", "item").await?;

    seller
        .call(h.registry.id(), "list_sub_account")
        .args_json(json!({
            "tla_id": tla.id(),
            "name": "item",
            "price": NearToken::from_near(5).as_yoctonear().to_string(),
        }))
        .deposit(NearToken::from_yoctonear(1))
        .transact()
        .await?
        .into_result()?;

    let buyer = h
        .worker
        .root_account()?
        .create_subaccount("buyer-a")
        .initial_balance(NearToken::from_near(100))
        .transact()
        .await?
        .into_result()?;
    let buyer_pk = near_workspaces::types::SecretKey::from_random(KeyType::ED25519).public_key();

    let before: serde_json::Value = h
        .registry
        .view("get_pending_refund")
        .args_json(json!({ "account_id": seller.id() }))
        .await?
        .json()?;
    let before_yocto: u128 = before.as_str().unwrap().parse()?;

    let buy = buyer
        .call(h.registry.id(), "buy_sub_account")
        .args_json(json!({
            "tla_id": tla.id(),
            "name": "item",
            "new_owner_key": buyer_pk,
        }))
        .deposit(NearToken::from_near(5))
        .max_gas()
        .transact()
        .await?;
    assert!(buy.is_success(), "buy_sub_account failed: {:?}", buy);

    let sub_view: serde_json::Value = h
        .registry
        .view("get_sub_account")
        .args_json(json!({ "tla_id": tla.id(), "name": "item" }))
        .await?
        .json()?;
    assert_eq!(
        sub_view["owner"],
        serde_json::Value::String(buyer.id().to_string()),
        "ownership must move to the buyer after settlement"
    );

    let after: serde_json::Value = h
        .registry
        .view("get_pending_refund")
        .args_json(json!({ "account_id": seller.id() }))
        .await?
        .json()?;
    let after_yocto: u128 = after.as_str().unwrap().parse()?;
    assert_eq!(
        after_yocto,
        before_yocto + NearToken::from_near(5).as_yoctonear(),
        "seller proceeds (price minus zero commission) must be credited via pull payment"
    );

    let sub_id: near_workspaces::AccountId = format!("item.{}", tla.id()).parse()?;
    let cfg: serde_json::Value = h.worker.view(&sub_id, "get_config").await?.json()?;
    assert_eq!(
        cfg["owner_key"],
        serde_json::Value::String(buyer_pk.to_string()),
        "locker must hold the buyer key for later export"
    );
    assert_eq!(
        cfg["state"], "held",
        "account stays held after resale; no key is added until export"
    );

    let listing: Option<serde_json::Value> = h
        .registry
        .view("get_listing")
        .args_json(json!({ "tla_id": tla.id(), "name": "item" }))
        .await?
        .json()?;
    assert!(listing.is_none(), "listing must be cleared after sale");

    let replay = buyer
        .call(h.registry.id(), "buy_sub_account")
        .args_json(json!({
            "tla_id": tla.id(),
            "name": "item",
            "new_owner_key": buyer_pk,
        }))
        .deposit(NearToken::from_near(5))
        .max_gas()
        .transact()
        .await?;
    assert!(replay.is_failure(), "second buy must fail; listing is gone");
    let err = format!("{:?}", replay);
    assert!(
        err.contains("not_listed") || err.contains("NotListed"),
        "expected NotListed on replay, got: {}",
        err
    );

    Ok(())
}

#[tokio::test]
async fn test_resale_accepted_offer_bound_to_buyer() -> Result<()> {
    let h = setup().await?;
    let (tla, seller) = rent_business_sub(&h, "bazaar", "seller-b", "relic").await?;

    let buyer = h
        .worker
        .root_account()?
        .create_subaccount("buyer-b")
        .initial_balance(NearToken::from_near(100))
        .transact()
        .await?
        .into_result()?;
    let intruder = h
        .worker
        .root_account()?
        .create_subaccount("intruder-b")
        .initial_balance(NearToken::from_near(100))
        .transact()
        .await?
        .into_result()?;

    seller
        .call(h.registry.id(), "accept_offer")
        .args_json(json!({
            "tla_id": tla.id(),
            "name": "relic",
            "buyer": buyer.id(),
            "price": NearToken::from_near(3).as_yoctonear().to_string(),
        }))
        .deposit(NearToken::from_yoctonear(1))
        .transact()
        .await?
        .into_result()?;

    let intruder_pk = near_workspaces::types::SecretKey::from_random(KeyType::ED25519).public_key();
    let stolen = intruder
        .call(h.registry.id(), "buy_sub_account")
        .args_json(json!({
            "tla_id": tla.id(),
            "name": "relic",
            "new_owner_key": intruder_pk,
        }))
        .deposit(NearToken::from_near(3))
        .max_gas()
        .transact()
        .await?;
    assert!(
        stolen.is_failure(),
        "an accepted offer must only be fillable by its bound buyer"
    );
    let err = format!("{:?}", stolen);
    assert!(
        err.contains("not_listed") || err.contains("NotListed"),
        "non-bound buyer with no public listing should hit NotListed, got: {}",
        err
    );

    let buyer_pk = near_workspaces::types::SecretKey::from_random(KeyType::ED25519).public_key();
    let buy = buyer
        .call(h.registry.id(), "buy_sub_account")
        .args_json(json!({
            "tla_id": tla.id(),
            "name": "relic",
            "new_owner_key": buyer_pk,
        }))
        .deposit(NearToken::from_near(3))
        .max_gas()
        .transact()
        .await?;
    assert!(buy.is_success(), "bound buyer fill failed: {:?}", buy);

    let sub_view: serde_json::Value = h
        .registry
        .view("get_sub_account")
        .args_json(json!({ "tla_id": tla.id(), "name": "relic" }))
        .await?
        .json()?;
    assert_eq!(
        sub_view["owner"],
        serde_json::Value::String(buyer.id().to_string()),
        "bound buyer must own the account after settlement"
    );

    let offer: Option<serde_json::Value> = h
        .registry
        .view("get_accepted_offer")
        .args_json(json!({ "tla_id": tla.id(), "name": "relic" }))
        .await?
        .json()?;
    assert!(offer.is_none(), "accepted offer must be cleared after sale");

    Ok(())
}

#[tokio::test]
async fn test_resale_authorization_guards() -> Result<()> {
    let h = setup().await?;
    let (tla, seller) = rent_business_sub(&h, "exchange", "seller-c", "token").await?;

    let stranger = h
        .worker
        .root_account()?
        .create_subaccount("stranger-c")
        .initial_balance(NearToken::from_near(100))
        .transact()
        .await?
        .into_result()?;

    let not_owner = stranger
        .call(h.registry.id(), "list_sub_account")
        .args_json(json!({
            "tla_id": tla.id(),
            "name": "token",
            "price": NearToken::from_near(5).as_yoctonear().to_string(),
        }))
        .deposit(NearToken::from_yoctonear(1))
        .transact()
        .await?;
    assert!(
        not_owner.is_failure(),
        "a non-owner must not be able to list someone else's account"
    );
    let err = format!("{:?}", not_owner);
    assert!(
        err.contains("only_owner") || err.contains("OnlyOwner"),
        "expected OnlyOwner, got: {}",
        err
    );

    seller
        .call(h.registry.id(), "list_sub_account")
        .args_json(json!({
            "tla_id": tla.id(),
            "name": "token",
            "price": NearToken::from_near(5).as_yoctonear().to_string(),
        }))
        .deposit(NearToken::from_yoctonear(1))
        .transact()
        .await?
        .into_result()?;

    let underpaid_pk =
        near_workspaces::types::SecretKey::from_random(KeyType::ED25519).public_key();
    let underpaid = stranger
        .call(h.registry.id(), "buy_sub_account")
        .args_json(json!({
            "tla_id": tla.id(),
            "name": "token",
            "new_owner_key": underpaid_pk,
        }))
        .deposit(NearToken::from_near(4))
        .max_gas()
        .transact()
        .await?;
    assert!(
        underpaid.is_failure(),
        "a deposit below the listed price must be rejected"
    );
    let err = format!("{:?}", underpaid);
    assert!(
        err.contains("price_not_met") || err.contains("PriceNotMet"),
        "expected PriceNotMet, got: {}",
        err
    );

    Ok(())
}

#[tokio::test]
async fn test_resale_commission_split() -> Result<()> {
    let h = setup().await?;
    let (tla, seller) = rent_business_sub(&h, "souk", "seller-d", "gem").await?;

    let fee_config_json = concat!(
        "{\"config\":{",
        "\"tla_allocation_fee\":1000000000000000000000000000,",
        "\"rent_tier_5\":50000000000000000000000000,",
        "\"rent_tier_8\":20000000000000000000000000,",
        "\"rent_tier_10\":10000000000000000000000000,",
        "\"rent_tier_12plus\":5000000000000000000000000,",
        "\"sub_fee_per_account\":500000000000000000000000,",
        "\"account_creation_deposit\":2000000000000000000000000,",
        "\"business_max_subs\":1000,",
        "\"retraction_notice_ns\":604800000000000,",
        "\"resale_commission_bps\":250",
        "}}"
    );
    h.admin
        .call(h.registry.id(), "update_fee_config")
        .args(fee_config_json.as_bytes().to_vec())
        .transact()
        .await?
        .into_result()?;

    seller
        .call(h.registry.id(), "list_sub_account")
        .args_json(json!({
            "tla_id": tla.id(),
            "name": "gem",
            "price": NearToken::from_near(100).as_yoctonear().to_string(),
        }))
        .deposit(NearToken::from_yoctonear(1))
        .transact()
        .await?
        .into_result()?;

    let buyer = h
        .worker
        .root_account()?
        .create_subaccount("buyer-d")
        .initial_balance(NearToken::from_near(200))
        .transact()
        .await?
        .into_result()?;
    let buyer_pk = near_workspaces::types::SecretKey::from_random(KeyType::ED25519).public_key();

    let seller_before: u128 = {
        let v: serde_json::Value = h
            .registry
            .view("get_pending_refund")
            .args_json(json!({ "account_id": seller.id() }))
            .await?
            .json()?;
        v.as_str().unwrap().parse()?
    };
    let revenue_before: u128 = {
        let v: serde_json::Value = h
            .registry
            .view("get_stats")
            .args_json(json!({}))
            .await?
            .json()?;
        v["total_revenue_yocto"].as_str().unwrap().parse()?
    };

    let buy = buyer
        .call(h.registry.id(), "buy_sub_account")
        .args_json(json!({
            "tla_id": tla.id(),
            "name": "gem",
            "new_owner_key": buyer_pk,
        }))
        .deposit(NearToken::from_near(100))
        .max_gas()
        .transact()
        .await?;
    assert!(buy.is_success(), "buy_sub_account failed: {:?}", buy);

    let seller_after: u128 = {
        let v: serde_json::Value = h
            .registry
            .view("get_pending_refund")
            .args_json(json!({ "account_id": seller.id() }))
            .await?
            .json()?;
        v.as_str().unwrap().parse()?
    };
    let revenue_after: u128 = {
        let v: serde_json::Value = h
            .registry
            .view("get_stats")
            .args_json(json!({}))
            .await?
            .json()?;
        v["total_revenue_yocto"].as_str().unwrap().parse()?
    };

    let price = NearToken::from_near(100).as_yoctonear();
    let commission = price * 250 / 10_000;
    let proceeds = price - commission;
    assert_eq!(
        seller_after - seller_before,
        proceeds,
        "seller must receive price minus the 2.5% commission"
    );
    assert_eq!(
        revenue_after - revenue_before,
        commission,
        "HoS revenue must increase by exactly the commission"
    );

    Ok(())
}

#[tokio::test]
async fn test_resale_blocked_when_tla_suspended() -> Result<()> {
    let h = setup().await?;
    let (tla, seller) = rent_business_sub(&h, "embargo", "seller-e", "asset").await?;

    h.admin
        .call(h.registry.id(), "suspend_tla")
        .args_json(json!({ "tla_id": tla.id() }))
        .transact()
        .await?
        .into_result()?;

    let blocked = seller
        .call(h.registry.id(), "list_sub_account")
        .args_json(json!({
            "tla_id": tla.id(),
            "name": "asset",
            "price": NearToken::from_near(5).as_yoctonear().to_string(),
        }))
        .deposit(NearToken::from_yoctonear(1))
        .transact()
        .await?;
    assert!(
        blocked.is_failure(),
        "a sub-account under a suspended TLA must not be sellable"
    );
    let err = format!("{:?}", blocked);
    assert!(
        err.contains("sub_account_not_sellable") || err.contains("SubAccountNotSellable"),
        "expected SubAccountNotSellable, got: {}",
        err
    );

    Ok(())
}

async fn list_for_sale(
    h: &Harness,
    seller: &Account,
    tla: &Account,
    name: &str,
    near_price: u128,
) -> Result<()> {
    seller
        .call(h.registry.id(), "list_sub_account")
        .args_json(json!({
            "tla_id": tla.id(),
            "name": name,
            "price": NearToken::from_near(near_price).as_yoctonear().to_string(),
        }))
        .deposit(NearToken::from_yoctonear(1))
        .transact()
        .await?
        .into_result()?;
    Ok(())
}

#[tokio::test]
async fn test_resale_unlist_clears_sale() -> Result<()> {
    let h = setup().await?;
    let (tla, seller) = rent_business_sub(&h, "stall", "seller-f", "wares").await?;
    list_for_sale(&h, &seller, &tla, "wares", 5).await?;

    let stranger = h
        .worker
        .root_account()?
        .create_subaccount("stranger-f")
        .initial_balance(NearToken::from_near(10))
        .transact()
        .await?
        .into_result()?;
    let not_owner = stranger
        .call(h.registry.id(), "unlist_sub_account")
        .args_json(json!({ "tla_id": tla.id(), "name": "wares" }))
        .deposit(NearToken::from_yoctonear(1))
        .transact()
        .await?;
    assert!(
        not_owner.is_failure(),
        "only the owner may unlist their listing"
    );
    let err = format!("{:?}", not_owner);
    assert!(
        err.contains("only_owner") || err.contains("OnlyOwner"),
        "expected OnlyOwner, got: {}",
        err
    );

    seller
        .call(h.registry.id(), "unlist_sub_account")
        .args_json(json!({ "tla_id": tla.id(), "name": "wares" }))
        .deposit(NearToken::from_yoctonear(1))
        .transact()
        .await?
        .into_result()?;

    let listing: Option<serde_json::Value> = h
        .registry
        .view("get_listing")
        .args_json(json!({ "tla_id": tla.id(), "name": "wares" }))
        .await?
        .json()?;
    assert!(listing.is_none(), "listing must be cleared after unlist");

    let buyer = h
        .worker
        .root_account()?
        .create_subaccount("buyer-f")
        .initial_balance(NearToken::from_near(20))
        .transact()
        .await?
        .into_result()?;
    let buyer_pk = near_workspaces::types::SecretKey::from_random(KeyType::ED25519).public_key();
    let buy = buyer
        .call(h.registry.id(), "buy_sub_account")
        .args_json(json!({ "tla_id": tla.id(), "name": "wares", "new_owner_key": buyer_pk }))
        .deposit(NearToken::from_near(5))
        .max_gas()
        .transact()
        .await?;
    assert!(buy.is_failure(), "an unlisted account cannot be bought");
    let err = format!("{:?}", buy);
    assert!(
        err.contains("not_listed") || err.contains("NotListed"),
        "expected NotListed, got: {}",
        err
    );

    Ok(())
}

#[tokio::test]
async fn test_resale_revoke_offer_blocks_buyer() -> Result<()> {
    let h = setup().await?;
    let (tla, seller) = rent_business_sub(&h, "depot", "seller-g", "crate").await?;

    let buyer = h
        .worker
        .root_account()?
        .create_subaccount("buyer-g")
        .initial_balance(NearToken::from_near(20))
        .transact()
        .await?
        .into_result()?;

    seller
        .call(h.registry.id(), "accept_offer")
        .args_json(json!({
            "tla_id": tla.id(),
            "name": "crate",
            "buyer": buyer.id(),
            "price": NearToken::from_near(3).as_yoctonear().to_string(),
        }))
        .deposit(NearToken::from_yoctonear(1))
        .transact()
        .await?
        .into_result()?;

    seller
        .call(h.registry.id(), "revoke_offer")
        .args_json(json!({ "tla_id": tla.id(), "name": "crate" }))
        .deposit(NearToken::from_yoctonear(1))
        .transact()
        .await?
        .into_result()?;

    let offer: Option<serde_json::Value> = h
        .registry
        .view("get_accepted_offer")
        .args_json(json!({ "tla_id": tla.id(), "name": "crate" }))
        .await?
        .json()?;
    assert!(offer.is_none(), "offer must be cleared after revoke");

    let buyer_pk = near_workspaces::types::SecretKey::from_random(KeyType::ED25519).public_key();
    let buy = buyer
        .call(h.registry.id(), "buy_sub_account")
        .args_json(json!({ "tla_id": tla.id(), "name": "crate", "new_owner_key": buyer_pk }))
        .deposit(NearToken::from_near(3))
        .max_gas()
        .transact()
        .await?;
    assert!(
        buy.is_failure(),
        "a revoked offer can no longer be filled by the bound buyer"
    );
    let err = format!("{:?}", buy);
    assert!(
        err.contains("not_listed") || err.contains("NotListed"),
        "expected NotListed after revoke, got: {}",
        err
    );

    Ok(())
}

#[tokio::test]
async fn test_resale_list_requires_one_yocto() -> Result<()> {
    let h = setup().await?;
    let (tla, seller) = rent_business_sub(&h, "kiosk", "seller-h", "good").await?;

    let no_yocto = seller
        .call(h.registry.id(), "list_sub_account")
        .args_json(json!({
            "tla_id": tla.id(),
            "name": "good",
            "price": NearToken::from_near(5).as_yoctonear().to_string(),
        }))
        .transact()
        .await?;
    assert!(
        no_yocto.is_failure(),
        "list without the one-yocto full-access-key proof must be rejected"
    );
    let err = format!("{:?}", no_yocto);
    assert!(
        err.contains("requires_one_yocto") || err.contains("RequiresOneYocto"),
        "expected RequiresOneYocto, got: {}",
        err
    );

    Ok(())
}

#[tokio::test]
async fn test_resale_zero_price_rejected() -> Result<()> {
    let h = setup().await?;
    let (tla, seller) = rent_business_sub(&h, "mart", "seller-i", "lot").await?;

    let zero_list = seller
        .call(h.registry.id(), "list_sub_account")
        .args_json(json!({ "tla_id": tla.id(), "name": "lot", "price": "0" }))
        .deposit(NearToken::from_yoctonear(1))
        .transact()
        .await?;
    assert!(
        zero_list.is_failure(),
        "a zero-price listing must be rejected"
    );
    let err = format!("{:?}", zero_list);
    assert!(
        err.contains("invalid_price") || err.contains("InvalidPrice"),
        "expected InvalidPrice on list, got: {}",
        err
    );

    let zero_offer = seller
        .call(h.registry.id(), "accept_offer")
        .args_json(json!({
            "tla_id": tla.id(),
            "name": "lot",
            "buyer": seller.id(),
            "price": "0",
        }))
        .deposit(NearToken::from_yoctonear(1))
        .transact()
        .await?;
    assert!(
        zero_offer.is_failure(),
        "a zero-price accepted offer must be rejected"
    );
    let err = format!("{:?}", zero_offer);
    assert!(
        err.contains("invalid_price") || err.contains("InvalidPrice"),
        "expected InvalidPrice on accept_offer, got: {}",
        err
    );

    Ok(())
}

#[tokio::test]
async fn test_resale_buy_refunds_excess() -> Result<()> {
    let h = setup().await?;
    let (tla, seller) = rent_business_sub(&h, "outlet", "seller-j", "parcel").await?;
    list_for_sale(&h, &seller, &tla, "parcel", 5).await?;

    let buyer = h
        .worker
        .root_account()?
        .create_subaccount("buyer-j")
        .initial_balance(NearToken::from_near(50))
        .transact()
        .await?
        .into_result()?;
    let buyer_pk = near_workspaces::types::SecretKey::from_random(KeyType::ED25519).public_key();

    let buy = buyer
        .call(h.registry.id(), "buy_sub_account")
        .args_json(json!({ "tla_id": tla.id(), "name": "parcel", "new_owner_key": buyer_pk }))
        .deposit(NearToken::from_near(8))
        .max_gas()
        .transact()
        .await?;
    assert!(buy.is_success(), "buy failed: {:?}", buy);

    let buyer_pending: serde_json::Value = h
        .registry
        .view("get_pending_refund")
        .args_json(json!({ "account_id": buyer.id() }))
        .await?
        .json()?;
    let buyer_pending_yocto: u128 = buyer_pending.as_str().unwrap().parse()?;
    assert_eq!(
        buyer_pending_yocto,
        NearToken::from_near(3).as_yoctonear(),
        "overpayment above the price must be refunded to the buyer via pull payment"
    );

    Ok(())
}

#[tokio::test]
async fn test_resale_mother_not_sellable() -> Result<()> {
    let h = setup().await?;
    let (tla, seller) = rent_business_sub(&h, "anchor", "seller-k", "home").await?;

    let sub_id: near_workspaces::AccountId = format!("home.{}", tla.id()).parse()?;
    seller
        .call(h.registry.id(), "set_mother")
        .args_json(json!({ "new_mother": sub_id }))
        .deposit(NearToken::from_yoctonear(1))
        .transact()
        .await?
        .into_result()?;

    let blocked = seller
        .call(h.registry.id(), "list_sub_account")
        .args_json(json!({
            "tla_id": tla.id(),
            "name": "home",
            "price": NearToken::from_near(5).as_yoctonear().to_string(),
        }))
        .deposit(NearToken::from_yoctonear(1))
        .transact()
        .await?;
    assert!(
        blocked.is_failure(),
        "a mother/anchor account must never be sellable"
    );
    let err = format!("{:?}", blocked);
    assert!(
        err.contains("sub_account_is_mother") || err.contains("SubAccountIsMother"),
        "expected SubAccountIsMother, got: {}",
        err
    );

    Ok(())
}

#[tokio::test]
async fn test_resale_retraction_blocks_sale() -> Result<()> {
    let h = setup().await?;
    let (tla, seller) = rent_business_sub(&h, "lease", "seller-l", "unit").await?;

    seller
        .call(h.registry.id(), "schedule_retraction")
        .args_json(json!({ "tla_id": tla.id(), "name": "unit" }))
        .deposit(NearToken::from_yoctonear(1))
        .transact()
        .await?
        .into_result()?;

    let blocked = seller
        .call(h.registry.id(), "list_sub_account")
        .args_json(json!({
            "tla_id": tla.id(),
            "name": "unit",
            "price": NearToken::from_near(5).as_yoctonear().to_string(),
        }))
        .deposit(NearToken::from_yoctonear(1))
        .transact()
        .await?;
    assert!(
        blocked.is_failure(),
        "a sub-account with a pending retraction must not be sellable"
    );
    let err = format!("{:?}", blocked);
    assert!(
        err.contains("retraction_pending") || err.contains("RetractionPending"),
        "expected RetractionPending, got: {}",
        err
    );

    Ok(())
}

#[tokio::test]
async fn test_resale_pause_blocks_marketplace() -> Result<()> {
    let h = setup().await?;
    let (tla, seller) = rent_business_sub(&h, "fair", "seller-m", "booth").await?;
    list_for_sale(&h, &seller, &tla, "booth", 5).await?;

    h.admin
        .call(h.registry.id(), "pause")
        .args_json(json!({}))
        .transact()
        .await?
        .into_result()?;

    let relist = seller
        .call(h.registry.id(), "list_sub_account")
        .args_json(json!({
            "tla_id": tla.id(),
            "name": "booth",
            "price": NearToken::from_near(6).as_yoctonear().to_string(),
        }))
        .deposit(NearToken::from_yoctonear(1))
        .transact()
        .await?;
    assert!(relist.is_failure(), "list must be blocked while paused");
    assert!(
        format!("{:?}", relist).to_lowercase().contains("paused"),
        "expected Paused on list"
    );

    let buyer = h
        .worker
        .root_account()?
        .create_subaccount("buyer-m")
        .initial_balance(NearToken::from_near(20))
        .transact()
        .await?
        .into_result()?;
    let buyer_pk = near_workspaces::types::SecretKey::from_random(KeyType::ED25519).public_key();
    let buy = buyer
        .call(h.registry.id(), "buy_sub_account")
        .args_json(json!({ "tla_id": tla.id(), "name": "booth", "new_owner_key": buyer_pk }))
        .deposit(NearToken::from_near(5))
        .max_gas()
        .transact()
        .await?;
    assert!(buy.is_failure(), "buy must be blocked while paused");
    assert!(
        format!("{:?}", buy).to_lowercase().contains("paused"),
        "expected Paused on buy"
    );

    Ok(())
}

#[tokio::test]
async fn test_resale_relist_updates_price() -> Result<()> {
    let h = setup().await?;
    let (tla, seller) = rent_business_sub(&h, "plaza", "seller-n", "spot").await?;
    list_for_sale(&h, &seller, &tla, "spot", 5).await?;
    list_for_sale(&h, &seller, &tla, "spot", 9).await?;

    let listing: serde_json::Value = h
        .registry
        .view("get_listing")
        .args_json(json!({ "tla_id": tla.id(), "name": "spot" }))
        .await?
        .json()?;
    assert_eq!(
        listing["price_yocto"].as_str().unwrap().parse::<u128>()?,
        NearToken::from_near(9).as_yoctonear(),
        "re-listing must overwrite the price"
    );

    let buyer = h
        .worker
        .root_account()?
        .create_subaccount("buyer-n")
        .initial_balance(NearToken::from_near(20))
        .transact()
        .await?
        .into_result()?;
    let buyer_pk = near_workspaces::types::SecretKey::from_random(KeyType::ED25519).public_key();

    let underpaid = buyer
        .call(h.registry.id(), "buy_sub_account")
        .args_json(json!({ "tla_id": tla.id(), "name": "spot", "new_owner_key": buyer_pk.clone() }))
        .deposit(NearToken::from_near(5))
        .max_gas()
        .transact()
        .await?;
    assert!(
        underpaid.is_failure(),
        "the old price must no longer satisfy the listing"
    );
    let err = format!("{:?}", underpaid);
    assert!(
        err.contains("price_not_met") || err.contains("PriceNotMet"),
        "expected PriceNotMet at old price, got: {}",
        err
    );

    let ok = buyer
        .call(h.registry.id(), "buy_sub_account")
        .args_json(json!({ "tla_id": tla.id(), "name": "spot", "new_owner_key": buyer_pk }))
        .deposit(NearToken::from_near(9))
        .max_gas()
        .transact()
        .await?;
    assert!(
        ok.is_success(),
        "buy at the updated price must succeed: {:?}",
        ok
    );

    Ok(())
}

#[tokio::test]
async fn test_resale_blocked_while_assets_unverifiable() -> Result<()> {
    let h = setup().await?;
    let (tla, seller) = rent_business_sub(&h, "escrow", "seller-o", "vault").await?;

    let ghost_ft: near_workspaces::AccountId =
        format!("ghost-ft.{}", h.worker.root_account()?.id()).parse()?;
    h.admin
        .call(h.registry.id(), "add_ft_allowlist")
        .args_json(json!({ "token": ghost_ft }))
        .transact()
        .await?
        .into_result()?;

    list_for_sale(&h, &seller, &tla, "vault", 5).await?;

    let buyer = h
        .worker
        .root_account()?
        .create_subaccount("buyer-o")
        .initial_balance(NearToken::from_near(50))
        .transact()
        .await?
        .into_result()?;
    let buyer_pk = near_workspaces::types::SecretKey::from_random(KeyType::ED25519).public_key();

    let _ = buyer
        .call(h.registry.id(), "buy_sub_account")
        .args_json(json!({ "tla_id": tla.id(), "name": "vault", "new_owner_key": buyer_pk }))
        .deposit(NearToken::from_near(5))
        .max_gas()
        .transact()
        .await?;

    let sub_view: serde_json::Value = h
        .registry
        .view("get_sub_account")
        .args_json(json!({ "tla_id": tla.id(), "name": "vault" }))
        .await?
        .json()?;
    assert_eq!(
        sub_view["owner"],
        serde_json::Value::String(seller.id().to_string()),
        "a sale must not complete while allowlisted-asset balances cannot be confirmed empty"
    );

    let pending: serde_json::Value = h
        .registry
        .view("get_pending_refund")
        .args_json(json!({ "account_id": buyer.id() }))
        .await?
        .json()?;
    let pending_yocto: u128 = pending.as_str().unwrap().parse()?;
    assert_eq!(
        pending_yocto,
        NearToken::from_near(5).as_yoctonear(),
        "the blocked buyer must be fully refunded via pull payment"
    );

    Ok(())
}
