mod error;

use crate::error::ManagerError;
use near_sdk::serde::Serialize;
use near_sdk::{env, near, AccountId, Gas, NearToken, PanicOnDefault, Promise, PublicKey};

const VERSION: u8 = 3;

const LOCKER_WASM: &[u8] = include_bytes!("../res/sub_account_locker.wasm");

const GAS_FOR_LOCKER_INIT: Gas = Gas::from_tgas(10);

#[near(contract_state)]
#[derive(PanicOnDefault)]
pub struct TlaManager {
    registry: AccountId,
}

#[near]
impl TlaManager {
    #[init]
    pub fn new(registry: AccountId) -> Self {
        Self { registry }
    }

    #[handle_result]
    #[payable]
    pub fn create_sub_account(
        &mut self,
        name: String,
        owner_key: PublicKey,
    ) -> Result<Promise, ManagerError> {
        self.assert_registry()?;
        let tla = env::current_account_id();
        let sub_account: AccountId = format!("{}.{}", name, tla)
            .parse()
            .map_err(|_| ManagerError::InvalidSubAccountName)?;

        let init_args = near_sdk::serde_json::json!({ "registry": self.registry })
            .to_string()
            .into_bytes();

        Ok(Promise::new(sub_account)
            .create_account()
            .deploy_contract(LOCKER_WASM.to_vec())
            .function_call(
                "new".to_string(),
                init_args,
                NearToken::from_yoctonear(0),
                GAS_FOR_LOCKER_INIT,
            )
            .add_full_access_key(owner_key)
            .transfer(env::attached_deposit()))
    }

    pub fn get_config(&self) -> ManagerConfig {
        ManagerConfig {
            registry: self.registry.clone(),
            tla: env::current_account_id(),
            version: VERSION,
        }
    }
}

impl TlaManager {
    fn assert_registry(&self) -> Result<(), ManagerError> {
        if env::predecessor_account_id() != self.registry {
            return Err(ManagerError::Unauthorized);
        }
        Ok(())
    }
}

#[derive(Serialize)]
#[serde(crate = "near_sdk::serde")]
pub struct ManagerConfig {
    pub registry: AccountId,
    pub tla: AccountId,
    pub version: u8,
}
