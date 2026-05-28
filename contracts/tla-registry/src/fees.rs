use crate::types::{total_name_length, FeeConfig, PremiumCategory, TlaEntry, TlaType, ONE_NEAR};
use near_sdk::AccountId;

pub fn base_rent(total_len: u8, config: &FeeConfig) -> u128 {
    match total_len {
        0..=5 => config.rent_tier_5,
        6..=8 => config.rent_tier_8,
        9..=10 => config.rent_tier_10,
        _ => config.rent_tier_12plus,
    }
}

pub fn sub_account_rent(total_len: u8, premium: &PremiumCategory, config: &FeeConfig) -> u128 {
    let base = base_rent(total_len, config);
    let (num, den) = premium.multiplier();
    base.saturating_mul(num) / den
}

pub fn calculate_rent(tla: &TlaEntry, tla_id: &AccountId, name: &str, config: &FeeConfig) -> u128 {
    match tla.tla_type {
        TlaType::Business => config.sub_fee_per_account,
        TlaType::Open => {
            let total_len = total_name_length(tla_id, name);
            sub_account_rent(total_len, &tla.premium_category, config)
        }
    }
}

pub fn default_fee_config() -> FeeConfig {
    FeeConfig {
        tla_allocation_fee: 1000 * ONE_NEAR,
        rent_tier_5: 50 * ONE_NEAR,
        rent_tier_8: 20 * ONE_NEAR,
        rent_tier_10: 10 * ONE_NEAR,
        rent_tier_12plus: 5 * ONE_NEAR,
        sub_fee_per_account: ONE_NEAR / 2,
        account_creation_deposit: 2 * ONE_NEAR,
        business_max_subs: 1000,
        retraction_notice_ns: 7 * 24 * 60 * 60 * 1_000_000_000,
        resale_commission_bps: 0,
    }
}
