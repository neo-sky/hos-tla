use crate::error::ContractError;
use crate::events::{
    self, OfferAcceptedEvent, OfferRevokedEvent, SaleFailedEvent, SubAccountListedEvent,
    SubAccountSoldEvent, SubAccountUnlistedEvent,
};
use crate::mother::effective_sub_lifecycle;
use crate::types::*;
use crate::{TlaRegistry, TlaRegistryExt};
use near_sdk::json_types::U128;
use near_sdk::{env, ext_contract, is_promise_success, near, AccountId, Gas, Promise, PublicKey};

const GAS_FOR_LOCKER_TRANSFER: Gas = Gas::from_tgas(15);
const GAS_FOR_SOLD_CALLBACK: Gas = Gas::from_tgas(20);

const BPS_DENOMINATOR: u128 = 10_000;

#[allow(dead_code)]
#[ext_contract(ext_locker)]
trait SubAccountLocker {
    fn transfer(&mut self, new_owner_key: PublicKey);
}

#[near]
impl TlaRegistry {
    #[handle_result]
    #[payable]
    pub fn list_sub_account(
        &mut self,
        tla_id: AccountId,
        name: String,
        price: U128,
    ) -> Result<(), ContractError> {
        crate::assert_one_yocto()?;
        self.assert_not_paused()?;
        if price.0 == 0 {
            return Err(ContractError::InvalidPrice);
        }
        let key = sub_account_key(&tla_id, &name);
        self.assert_sale_idle(&key)?;
        let owner = self.assert_sellable(&key, &tla_id)?;
        if env::predecessor_account_id() != owner {
            return Err(ContractError::OnlyOwner);
        }
        self.listings.insert(
            key.clone(),
            Listing {
                price: price.0,
                settling: false,
            },
        );
        let price_str = price.0.to_string();
        events::emit(
            "sub_account_listed",
            &SubAccountListedEvent {
                full_name: &key,
                price_yocto: &price_str,
                seller: owner.as_str(),
            },
        );
        Ok(())
    }

    #[handle_result]
    #[payable]
    pub fn unlist_sub_account(
        &mut self,
        tla_id: AccountId,
        name: String,
    ) -> Result<(), ContractError> {
        crate::assert_one_yocto()?;
        self.assert_not_paused()?;
        let key = sub_account_key(&tla_id, &name);
        self.assert_sale_idle(&key)?;
        if !self.listings.contains_key(&key) {
            return Err(ContractError::NotListed);
        }
        let owner = self.sub_account_owner(&key)?;
        if env::predecessor_account_id() != owner {
            return Err(ContractError::OnlyOwner);
        }
        self.listings.remove(&key);
        events::emit(
            "sub_account_unlisted",
            &SubAccountUnlistedEvent {
                full_name: &key,
                by: owner.as_str(),
            },
        );
        Ok(())
    }

    #[handle_result]
    #[payable]
    pub fn accept_offer(
        &mut self,
        tla_id: AccountId,
        name: String,
        buyer: AccountId,
        price: U128,
    ) -> Result<(), ContractError> {
        crate::assert_one_yocto()?;
        self.assert_not_paused()?;
        if price.0 == 0 {
            return Err(ContractError::InvalidPrice);
        }
        let key = sub_account_key(&tla_id, &name);
        self.assert_sale_idle(&key)?;
        let owner = self.assert_sellable(&key, &tla_id)?;
        if env::predecessor_account_id() != owner {
            return Err(ContractError::OnlyOwner);
        }
        self.accepted_offers.insert(
            key.clone(),
            AcceptedOffer {
                buyer: buyer.clone(),
                price: price.0,
                settling: false,
            },
        );
        let price_str = price.0.to_string();
        events::emit(
            "offer_accepted",
            &OfferAcceptedEvent {
                full_name: &key,
                buyer: buyer.as_str(),
                price_yocto: &price_str,
                seller: owner.as_str(),
            },
        );
        Ok(())
    }

    #[handle_result]
    #[payable]
    pub fn revoke_offer(&mut self, tla_id: AccountId, name: String) -> Result<(), ContractError> {
        crate::assert_one_yocto()?;
        self.assert_not_paused()?;
        let key = sub_account_key(&tla_id, &name);
        self.assert_sale_idle(&key)?;
        if !self.accepted_offers.contains_key(&key) {
            return Err(ContractError::NoAcceptedOffer);
        }
        let owner = self.sub_account_owner(&key)?;
        if env::predecessor_account_id() != owner {
            return Err(ContractError::OnlyOwner);
        }
        self.accepted_offers.remove(&key);
        events::emit(
            "offer_revoked",
            &OfferRevokedEvent {
                full_name: &key,
                by: owner.as_str(),
            },
        );
        Ok(())
    }

    #[handle_result]
    #[payable]
    pub fn buy_sub_account(
        &mut self,
        tla_id: AccountId,
        name: String,
        new_owner_key: PublicKey,
    ) -> Result<Promise, ContractError> {
        self.assert_not_paused()?;
        let key = sub_account_key(&tla_id, &name);
        self.assert_sale_idle(&key)?;
        self.assert_sellable(&key, &tla_id)?;
        let buyer = env::predecessor_account_id();
        let deposit = env::attached_deposit().as_yoctonear();
        let price = self.resolve_and_lock_sale(&key, &buyer, deposit)?;
        let sub_account: AccountId = key
            .parse()
            .map_err(|_| ContractError::InvalidSubAccountId)?;
        Ok(ext_locker::ext(sub_account)
            .with_static_gas(GAS_FOR_LOCKER_TRANSFER)
            .transfer(new_owner_key)
            .then(
                Self::ext(env::current_account_id())
                    .with_static_gas(GAS_FOR_SOLD_CALLBACK)
                    .on_sub_account_sold(tla_id, name, buyer, U128(price), U128(deposit)),
            ))
    }

    #[private]
    pub fn on_sub_account_sold(
        &mut self,
        tla_id: AccountId,
        name: String,
        buyer: AccountId,
        price: U128,
        deposit: U128,
    ) {
        let key = sub_account_key(&tla_id, &name);
        if !is_promise_success() {
            self.add_pending_refund(&buyer, deposit.0);
            self.clear_settling(&key);
            events::emit(
                "sub_account_sale_failed",
                &SaleFailedEvent {
                    full_name: &key,
                    buyer: buyer.as_str(),
                },
            );
            return;
        }
        let seller = match self.sub_accounts.get(&key).map(|s| s.owner.clone()) {
            Some(owner) => owner,
            None => {
                self.add_pending_refund(&buyer, deposit.0);
                self.clear_settling(&key);
                return;
            }
        };
        let bps = u128::from(self.fee_config.resale_commission_bps);
        let commission = price.0.saturating_mul(bps) / BPS_DENOMINATOR;
        let commission = commission.min(price.0);
        let seller_proceeds = price.0.saturating_sub(commission);
        self.total_revenue = self.total_revenue.saturating_add(commission);
        self.add_pending_refund(&seller, seller_proceeds);
        let excess = deposit.0.saturating_sub(price.0);
        if excess > 0 {
            self.add_pending_refund(&buyer, excess);
        }
        if let Some(sub) = self.sub_accounts.get_mut(&key) {
            sub.owner = buyer.clone();
            sub.main_wallet = buyer.clone();
        }
        self.listings.remove(&key);
        self.accepted_offers.remove(&key);
        let price_str = price.0.to_string();
        let commission_str = commission.to_string();
        let proceeds_str = seller_proceeds.to_string();
        events::emit(
            "sub_account_sold",
            &SubAccountSoldEvent {
                full_name: &key,
                tla_id: tla_id.as_str(),
                seller: seller.as_str(),
                buyer: buyer.as_str(),
                price_yocto: &price_str,
                commission_yocto: &commission_str,
                seller_proceeds_yocto: &proceeds_str,
            },
        );
    }

    pub fn get_listing(&self, tla_id: AccountId, name: String) -> Option<ListingView> {
        let key = sub_account_key(&tla_id, &name);
        self.listings.get(&key).map(|l| ListingView {
            full_name: key.clone(),
            price_yocto: U128(l.price),
            settling: l.settling,
        })
    }

    pub fn get_accepted_offer(&self, tla_id: AccountId, name: String) -> Option<AcceptedOfferView> {
        let key = sub_account_key(&tla_id, &name);
        self.accepted_offers.get(&key).map(|o| AcceptedOfferView {
            full_name: key.clone(),
            buyer: o.buyer.clone(),
            price_yocto: U128(o.price),
            settling: o.settling,
        })
    }
}

impl TlaRegistry {
    fn assert_sale_idle(&self, key: &str) -> Result<(), ContractError> {
        if let Some(listing) = self.listings.get(key) {
            if listing.settling {
                return Err(ContractError::SaleInProgress);
            }
        }
        if let Some(offer) = self.accepted_offers.get(key) {
            if offer.settling {
                return Err(ContractError::SaleInProgress);
            }
        }
        Ok(())
    }

    fn assert_sellable(&self, key: &str, tla_id: &AccountId) -> Result<AccountId, ContractError> {
        let sub_account: AccountId = key
            .parse()
            .map_err(|_| ContractError::InvalidSubAccountId)?;
        if self
            .mother_use_count
            .get(&sub_account)
            .copied()
            .unwrap_or(0)
            > 0
        {
            return Err(ContractError::SubAccountIsMother);
        }
        let sub = self
            .sub_accounts
            .get(key)
            .ok_or(ContractError::SubAccountNotFound)?;
        if sub.retraction_at.is_some() {
            return Err(ContractError::RetractionPending);
        }
        let tla = self.tlas.get(tla_id).ok_or(ContractError::TlaNotFound)?;
        if tla.status != TlaStatus::Active {
            return Err(ContractError::SubAccountNotSellable);
        }
        if !matches!(
            effective_sub_lifecycle(sub, tla, self.fee_config.retraction_notice_ns),
            LifecycleStatus::Active
        ) {
            return Err(ContractError::SubAccountNotSellable);
        }
        Ok(sub.owner.clone())
    }

    fn sub_account_owner(&self, key: &str) -> Result<AccountId, ContractError> {
        self.sub_accounts
            .get(key)
            .map(|s| s.owner.clone())
            .ok_or(ContractError::SubAccountNotFound)
    }

    fn resolve_and_lock_sale(
        &mut self,
        key: &str,
        buyer: &AccountId,
        deposit: u128,
    ) -> Result<u128, ContractError> {
        let offer_terms = self
            .accepted_offers
            .get(key)
            .map(|o| (o.buyer.clone(), o.price));
        if let Some((offer_buyer, offer_price)) = offer_terms {
            if &offer_buyer == buyer {
                if deposit < offer_price {
                    return Err(ContractError::PriceNotMet);
                }
                if let Some(offer) = self.accepted_offers.get_mut(key) {
                    offer.settling = true;
                }
                return Ok(offer_price);
            }
        }
        let listing_price = self.listings.get(key).map(|l| l.price);
        if let Some(listing_price) = listing_price {
            if deposit < listing_price {
                return Err(ContractError::PriceNotMet);
            }
            if let Some(listing) = self.listings.get_mut(key) {
                listing.settling = true;
            }
            return Ok(listing_price);
        }
        Err(ContractError::NotListed)
    }

    fn clear_settling(&mut self, key: &str) {
        if let Some(listing) = self.listings.get_mut(key) {
            listing.settling = false;
        }
        if let Some(offer) = self.accepted_offers.get_mut(key) {
            offer.settling = false;
        }
    }
}
