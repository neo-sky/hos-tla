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
    assert_eq!(cfg2["state"], "exported", "locker must be in exported terminal state");

    Ok(())
}
