mod error;

use crate::error::LockerError;
use near_sdk::json_types::U128;
use near_sdk::serde::Serialize;
use near_sdk::{
    env, ext_contract, near, AccountId, Gas, NearToken, PanicOnDefault, Promise, PromiseOrValue,
};

const LOCKER_VERSION: u8 = 1;

const STORAGE_DEPOSIT_AMOUNT: NearToken = NearToken::from_yoctonear(1_250_000_000_000_000_000_000);
const ONE_YOCTO: NearToken = NearToken::from_yoctonear(1);

const GAS_BALANCE_QUERY: Gas = Gas::from_tgas(5);
const GAS_STORAGE_DEPOSIT: Gas = Gas::from_tgas(8);
const GAS_FT_TRANSFER: Gas = Gas::from_tgas(10);
const GAS_AFTER_BALANCE: Gas = Gas::from_tgas(25);
const GAS_AFTER_DEPOSIT: Gas = Gas::from_tgas(12);

#[allow(dead_code)]
#[ext_contract(ext_ft)]
trait FungibleToken {
    fn ft_balance_of(&self, account_id: AccountId) -> U128;
    fn storage_deposit(&mut self, account_id: Option<AccountId>, registration_only: Option<bool>);
    fn ft_transfer(&mut self, receiver_id: AccountId, amount: U128, memo: Option<String>);
}

#[near(contract_state)]
#[derive(PanicOnDefault)]
pub struct SubAccountLocker {
    registry: AccountId,
}

#[near]
impl SubAccountLocker {
    #[init]
    pub fn new(registry: AccountId) -> Self {
        Self { registry }
    }

    #[handle_result]
    pub fn sweep_ft(
        &mut self,
        ft: AccountId,
        destination: AccountId,
    ) -> Result<Promise, LockerError> {
        self.assert_registry()?;
        Ok(ext_ft::ext(ft.clone())
            .with_static_gas(GAS_BALANCE_QUERY)
            .ft_balance_of(env::current_account_id())
            .then(
                Self::ext(env::current_account_id())
                    .with_static_gas(GAS_AFTER_BALANCE)
                    .after_balance_query(ft, destination),
            ))
    }

    #[private]
    pub fn after_balance_query(
        &mut self,
        ft: AccountId,
        destination: AccountId,
        #[callback_unwrap] balance: U128,
    ) -> PromiseOrValue<()> {
        if balance.0 == 0 {
            return PromiseOrValue::Value(());
        }
        let chain = ext_ft::ext(ft.clone())
            .with_static_gas(GAS_STORAGE_DEPOSIT)
            .with_attached_deposit(STORAGE_DEPOSIT_AMOUNT)
            .storage_deposit(Some(destination.clone()), Some(true))
            .then(
                Self::ext(env::current_account_id())
                    .with_static_gas(GAS_AFTER_DEPOSIT)
                    .after_storage_deposit(ft, destination, balance),
            );
        PromiseOrValue::Promise(chain)
    }

    #[private]
    pub fn after_storage_deposit(
        &mut self,
        ft: AccountId,
        destination: AccountId,
        amount: U128,
    ) -> Promise {
        ext_ft::ext(ft)
            .with_static_gas(GAS_FT_TRANSFER)
            .with_attached_deposit(ONE_YOCTO)
            .ft_transfer(destination, amount, Some(String::from("hos-tla reclaim")))
    }

    #[handle_result]
    pub fn finalize_delete(&mut self, destination: AccountId) -> Result<Promise, LockerError> {
        self.assert_registry()?;
        Ok(Promise::new(env::current_account_id()).delete_account(destination))
    }

    pub fn get_config(&self) -> LockerConfig {
        LockerConfig {
            registry: self.registry.clone(),
            locker: env::current_account_id(),
            version: LOCKER_VERSION,
        }
    }
}

impl SubAccountLocker {
    fn assert_registry(&self) -> Result<(), LockerError> {
        if env::predecessor_account_id() != self.registry {
            return Err(LockerError::Unauthorized);
        }
        Ok(())
    }
}

#[derive(Serialize)]
#[serde(crate = "near_sdk::serde")]
pub struct LockerConfig {
    pub registry: AccountId,
    pub locker: AccountId,
    pub version: u8,
}
