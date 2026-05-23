use crate::error::ContractError;
use crate::events::{self, ResaleAbortRequestedEvent, ResaleUnlockRequestedEvent};
use crate::{TlaRegistry, TlaRegistryExt, RESALE_LOCKER_WASM};
use near_sdk::json_types::Base64VecU8;
use near_sdk::{env, ext_contract, near, AccountId, Gas, Promise, PublicKey};

const GAS_FOR_RESALE_DISPATCH: Gas = Gas::from_tgas(15);

#[allow(dead_code)]
#[ext_contract(ext_resale_locker)]
trait ResaleLocker {
    fn unlock(&mut self, buyer_key: PublicKey);
    fn abort(&mut self);
}

#[near]
impl TlaRegistry {
    #[handle_result]
    pub fn resale_unlock(
        &mut self,
        account: AccountId,
        buyer_key: PublicKey,
    ) -> Result<Promise, ContractError> {
        self.assert_admin()?;
        events::emit(
            "resale_unlock_requested",
            &ResaleUnlockRequestedEvent {
                account: account.as_str(),
                by: env::predecessor_account_id().as_str(),
            },
        );
        Ok(ext_resale_locker::ext(account)
            .with_static_gas(GAS_FOR_RESALE_DISPATCH)
            .unlock(buyer_key))
    }

    #[handle_result]
    pub fn resale_abort(&mut self, account: AccountId) -> Result<Promise, ContractError> {
        self.assert_admin()?;
        events::emit(
            "resale_abort_requested",
            &ResaleAbortRequestedEvent {
                account: account.as_str(),
                by: env::predecessor_account_id().as_str(),
            },
        );
        Ok(ext_resale_locker::ext(account)
            .with_static_gas(GAS_FOR_RESALE_DISPATCH)
            .abort())
    }

    pub fn get_resale_locker_wasm(&self) -> Base64VecU8 {
        Base64VecU8::from(RESALE_LOCKER_WASM.to_vec())
    }

    pub fn get_resale_locker_sha256(&self) -> String {
        let digest = env::sha256(RESALE_LOCKER_WASM);
        let mut hex = String::with_capacity(digest.len() * 2);
        for byte in digest {
            hex.push_str(&format!("{:02x}", byte));
        }
        hex
    }

    pub fn get_resale_locker_size(&self) -> u64 {
        RESALE_LOCKER_WASM.len() as u64
    }
}
