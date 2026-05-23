use crate::error::ContractError;
use crate::events::{self, RefundFailedEvent, SubAccountRentedEvent};
use crate::types::*;
use crate::{TlaRegistry, TlaRegistryExt};
use near_sdk::json_types::U128;
use near_sdk::{is_promise_success, near, AccountId, FunctionError};

#[near]
impl TlaRegistry {
    #[private]
    pub fn on_sub_account_created(
        &mut self,
        tla_id: AccountId,
        name: String,
        payer: AccountId,
        rent_yocto: U128,
        attached_yocto: U128,
    ) {
        let key = sub_account_key(&tla_id, &name);

        if is_promise_success() {
            self.sub_account_count = self.sub_account_count.saturating_add(1);
            self.total_revenue = self.total_revenue.saturating_add(rent_yocto.0);

            let charged = rent_yocto
                .0
                .saturating_add(self.fee_config.account_creation_deposit);
            let excess = attached_yocto.0.saturating_sub(charged);
            if excess > 0 {
                self.add_pending_refund(&payer, excess);
            }

            let rent_str = rent_yocto.0.to_string();
            let sub = match self.sub_accounts.get(&key) {
                Some(s) => s,
                None => ContractError::SubAccountNotFound.panic(),
            };
            events::emit(
                "sub_account_rented",
                &SubAccountRentedEvent {
                    full_name: &key,
                    tla_id: tla_id.as_str(),
                    owner: payer.as_str(),
                    rent_yocto: &rent_str,
                    expires_at: sub.expires_at,
                },
            );
        } else {
            self.sub_accounts.remove(&key);

            let is_business = self
                .tlas
                .get(&tla_id)
                .map(|t| t.tla_type == TlaType::Business)
                .unwrap_or(false);
            if is_business {
                self.business_count_decrement(&tla_id);
            }

            self.add_pending_refund(&payer, attached_yocto.0);

            let amount_str = attached_yocto.0.to_string();
            events::emit(
                "refund_pending",
                &RefundFailedEvent {
                    account: payer.as_str(),
                    amount_yocto: &amount_str,
                    reason: "sub-account creation failed",
                },
            );
        }
    }
}
