mod error;

use crate::error::ResaleLockerError;
use near_sdk::borsh::{BorshDeserialize, BorshSerialize};
use near_sdk::serde::Serialize;
use near_sdk::{env, is_promise_success, near, AccountId, Gas, PanicOnDefault, Promise, PublicKey};

const LOCKER_VERSION: u8 = 2;

const GAS_FOR_SETTLE_CALLBACK: Gas = Gas::from_tgas(5);

#[derive(BorshSerialize, BorshDeserialize, Serialize, Clone, PartialEq)]
#[borsh(crate = "near_sdk::borsh")]
#[serde(crate = "near_sdk::serde", rename_all = "snake_case")]
pub enum LockState {
    Active,
    Settling,
    Settled,
}

#[near(contract_state)]
#[derive(PanicOnDefault)]
pub struct ResaleLocker {
    registry: AccountId,
    recovery_key: PublicKey,
    state: LockState,
}

#[near]
impl ResaleLocker {
    #[init]
    pub fn new(registry: AccountId, recovery_key: PublicKey) -> Self {
        Self {
            registry,
            recovery_key,
            state: LockState::Active,
        }
    }

    #[handle_result]
    pub fn unlock(&mut self, buyer_key: PublicKey) -> Result<Promise, ResaleLockerError> {
        self.assert_registry()?;
        self.assert_active()?;
        self.state = LockState::Settling;
        Ok(Promise::new(env::current_account_id())
            .add_full_access_key(buyer_key)
            .then(
                Self::ext(env::current_account_id())
                    .with_static_gas(GAS_FOR_SETTLE_CALLBACK)
                    .on_settle_resolved(),
            ))
    }

    #[handle_result]
    pub fn abort(&mut self) -> Result<Promise, ResaleLockerError> {
        self.assert_registry()?;
        self.assert_active()?;
        self.state = LockState::Settling;
        Ok(Promise::new(env::current_account_id())
            .add_full_access_key(self.recovery_key.clone())
            .then(
                Self::ext(env::current_account_id())
                    .with_static_gas(GAS_FOR_SETTLE_CALLBACK)
                    .on_settle_resolved(),
            ))
    }

    #[private]
    pub fn on_settle_resolved(&mut self) {
        if is_promise_success() {
            self.state = LockState::Settled;
        } else {
            self.state = LockState::Active;
        }
    }

    pub fn get_config(&self) -> ResaleLockerConfig {
        ResaleLockerConfig {
            account: env::current_account_id(),
            registry: self.registry.clone(),
            recovery_key: self.recovery_key.clone(),
            state: self.state.clone(),
            version: LOCKER_VERSION,
        }
    }
}

impl ResaleLocker {
    fn assert_registry(&self) -> Result<(), ResaleLockerError> {
        if env::predecessor_account_id() != self.registry {
            return Err(ResaleLockerError::Unauthorized);
        }
        Ok(())
    }

    fn assert_active(&self) -> Result<(), ResaleLockerError> {
        match self.state {
            LockState::Active => Ok(()),
            LockState::Settling => Err(ResaleLockerError::SettlementInProgress),
            LockState::Settled => Err(ResaleLockerError::AlreadySettled),
        }
    }
}

#[derive(Serialize)]
#[serde(crate = "near_sdk::serde")]
pub struct ResaleLockerConfig {
    pub account: AccountId,
    pub registry: AccountId,
    pub recovery_key: PublicKey,
    pub state: LockState,
    pub version: u8,
}
