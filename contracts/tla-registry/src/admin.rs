use crate::error::ContractError;
use crate::events::{self, TlaRegisteredEvent, WithdrawalEvent};
use crate::types::*;
use crate::{TlaRegistry, TlaRegistryExt};
use near_sdk::json_types::U128;
use near_sdk::{env, near, AccountId};

const MAX_ALLOWLIST_SIZE: u32 = 64;

#[near]
impl TlaRegistry {
    #[handle_result]
    pub fn register_tla(
        &mut self,
        tla_id: AccountId,
        tla_type: TlaType,
        premium_category: PremiumCategory,
        licensee: Option<AccountId>,
    ) -> Result<(), ContractError> {
        self.assert_admin()?;
        if self.tlas.contains_key(&tla_id) {
            return Err(ContractError::TlaAlreadyRegistered);
        }
        if tla_type == TlaType::Business && licensee.is_none() {
            return Err(ContractError::BusinessTlaRequiresLicensee);
        }

        let entry = TlaEntry {
            tla_type: tla_type.clone(),
            status: TlaStatus::Registered,
            licensee: licensee.clone(),
            premium_category: premium_category.clone(),
            activated_at: 0,
            expires_at: 0,
        };
        self.tlas.insert(tla_id.clone(), entry);

        let type_str = match tla_type {
            TlaType::Business => "business",
            TlaType::Open => "open",
        };
        let premium_str = match premium_category {
            PremiumCategory::Legendary => "legendary",
            PremiumCategory::Premium => "premium",
            PremiumCategory::Standard => "standard",
            PremiumCategory::Community => "community",
        };
        let licensee_str = licensee.as_ref().map(|a| a.as_str());
        events::emit(
            "tla_registered",
            &TlaRegisteredEvent {
                tla_id: tla_id.as_str(),
                tla_type: type_str,
                premium_category: premium_str,
                licensee: licensee_str,
            },
        );
        Ok(())
    }

    #[handle_result]
    pub fn suspend_tla(&mut self, tla_id: AccountId) -> Result<(), ContractError> {
        self.assert_admin()?;
        let entry = self
            .tlas
            .get_mut(&tla_id)
            .ok_or(ContractError::TlaNotFound)?;
        entry.status = TlaStatus::Suspended;
        events::emit(
            "tla_suspended",
            &events::TlaSuspendedEvent {
                tla_id: tla_id.as_str(),
                action: "suspended",
                by: env::predecessor_account_id().as_str(),
            },
        );
        Ok(())
    }

    #[handle_result]
    pub fn unsuspend_tla(&mut self, tla_id: AccountId) -> Result<(), ContractError> {
        self.assert_admin()?;
        let entry = self
            .tlas
            .get_mut(&tla_id)
            .ok_or(ContractError::TlaNotFound)?;
        if entry.status != TlaStatus::Suspended {
            return Err(ContractError::TlaNotSuspended);
        }
        if entry.tla_type == TlaType::Business && entry.licensee.is_none() {
            return Err(ContractError::BusinessTlaMissingLicensee);
        }
        entry.status = TlaStatus::Active;
        events::emit(
            "tla_unsuspended",
            &events::TlaSuspendedEvent {
                tla_id: tla_id.as_str(),
                action: "unsuspended",
                by: env::predecessor_account_id().as_str(),
            },
        );
        Ok(())
    }

    #[handle_result]
    pub fn add_admin(&mut self, account_id: AccountId) -> Result<(), ContractError> {
        self.assert_admin()?;
        if !self.admins.insert(account_id.clone()) {
            return Ok(());
        }
        events::emit(
            "admin_added",
            &events::AdminChangeEvent {
                action: "added",
                account: account_id.as_str(),
                by: env::predecessor_account_id().as_str(),
            },
        );
        Ok(())
    }

    #[handle_result]
    pub fn remove_admin(&mut self, account_id: AccountId) -> Result<(), ContractError> {
        self.assert_admin()?;
        if self.admins.len() <= 1 {
            return Err(ContractError::CannotRemoveLastAdmin);
        }
        self.admins.remove(&account_id);
        events::emit(
            "admin_removed",
            &events::AdminChangeEvent {
                action: "removed",
                account: account_id.as_str(),
                by: env::predecessor_account_id().as_str(),
            },
        );
        Ok(())
    }

    #[handle_result]
    pub fn update_fee_config(&mut self, config: FeeConfig) -> Result<(), ContractError> {
        self.assert_admin()?;
        if config.rent_tier_5 == 0
            && config.rent_tier_8 == 0
            && config.rent_tier_10 == 0
            && config.rent_tier_12plus == 0
        {
            return Err(ContractError::AllRentTiersZero);
        }
        if config.account_creation_deposit == 0 {
            return Err(ContractError::CreationDepositZero);
        }
        self.fee_config = config;
        events::emit(
            "fee_config_updated",
            &events::PauseEvent {
                by: env::predecessor_account_id().as_str(),
            },
        );
        Ok(())
    }

    #[handle_result]
    pub fn withdraw(&mut self, amount: U128, recipient: AccountId) -> Result<(), ContractError> {
        self.assert_admin()?;
        let amount_yocto = amount.0;
        if amount_yocto == 0 {
            return Err(ContractError::WithdrawalAmountZero);
        }
        if amount_yocto > self.total_revenue {
            return Err(ContractError::InsufficientRevenue);
        }
        if self.total_pending_refunds.saturating_add(amount_yocto) > self.available_balance() {
            return Err(ContractError::InsufficientContractBalance);
        }
        self.total_revenue = self.total_revenue.saturating_sub(amount_yocto);
        self.add_pending_refund(&recipient, amount_yocto);

        let amount_str = amount_yocto.to_string();
        events::emit(
            "withdrawal_queued",
            &WithdrawalEvent {
                amount_yocto: &amount_str,
                recipient: recipient.as_str(),
            },
        );
        Ok(())
    }

    #[handle_result]
    pub fn add_ft_allowlist(&mut self, token: AccountId) -> Result<(), ContractError> {
        self.assert_admin()?;
        if self.ft_allowlist.contains(&token) {
            return Ok(());
        }
        if self.ft_allowlist.len() >= MAX_ALLOWLIST_SIZE {
            return Err(ContractError::AllowlistFull);
        }
        self.ft_allowlist.insert(token.clone());
        events::emit(
            "ft_allowlist_added",
            &events::AllowlistEvent {
                kind: "ft",
                token: token.as_str(),
                by: env::predecessor_account_id().as_str(),
            },
        );
        Ok(())
    }

    #[handle_result]
    pub fn remove_ft_allowlist(&mut self, token: AccountId) -> Result<(), ContractError> {
        self.assert_admin()?;
        if !self.ft_allowlist.remove(&token) {
            return Ok(());
        }
        events::emit(
            "ft_allowlist_removed",
            &events::AllowlistEvent {
                kind: "ft",
                token: token.as_str(),
                by: env::predecessor_account_id().as_str(),
            },
        );
        Ok(())
    }

    #[handle_result]
    pub fn add_nft_allowlist(&mut self, token: AccountId) -> Result<(), ContractError> {
        self.assert_admin()?;
        if self.nft_allowlist.contains(&token) {
            return Ok(());
        }
        if self.nft_allowlist.len() >= MAX_ALLOWLIST_SIZE {
            return Err(ContractError::AllowlistFull);
        }
        self.nft_allowlist.insert(token.clone());
        events::emit(
            "nft_allowlist_added",
            &events::AllowlistEvent {
                kind: "nft",
                token: token.as_str(),
                by: env::predecessor_account_id().as_str(),
            },
        );
        Ok(())
    }

    #[handle_result]
    pub fn remove_nft_allowlist(&mut self, token: AccountId) -> Result<(), ContractError> {
        self.assert_admin()?;
        if !self.nft_allowlist.remove(&token) {
            return Ok(());
        }
        events::emit(
            "nft_allowlist_removed",
            &events::AllowlistEvent {
                kind: "nft",
                token: token.as_str(),
                by: env::predecessor_account_id().as_str(),
            },
        );
        Ok(())
    }

    #[handle_result]
    pub fn activate_open_tla(&mut self, tla_id: AccountId) -> Result<(), ContractError> {
        self.assert_admin()?;
        let now = env::block_timestamp();
        let new_expires_at = {
            let entry = self
                .tlas
                .get_mut(&tla_id)
                .ok_or(ContractError::TlaNotFound)?;
            if entry.status != TlaStatus::Registered {
                return Err(ContractError::TlaNotInRegisteredState);
            }
            if entry.tla_type != TlaType::Open {
                return Err(ContractError::WrongActivationEndpoint);
            }
            entry.status = TlaStatus::Active;
            entry.activated_at = now;
            entry.expires_at = now.saturating_add(ONE_YEAR_NS);
            entry.expires_at
        };

        events::emit(
            "tla_activated",
            &events::TlaActivatedEvent {
                tla_id: tla_id.as_str(),
                expires_at: new_expires_at,
                paid_yocto: "0",
            },
        );
        Ok(())
    }
}
