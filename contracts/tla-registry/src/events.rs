use near_sdk::env;
use near_sdk::serde::Serialize;

const STANDARD: &str = "hos-tla";
const VERSION: &str = "1.0.0";

pub fn emit<T: Serialize>(event: &str, data: &T) {
    let json = near_sdk::serde_json::to_string(data).expect("event serialization failed");
    env::log_str(&format!(
        "EVENT_JSON:{{\"standard\":\"{}\",\"version\":\"{}\",\"event\":\"{}\",\"data\":{}}}",
        STANDARD, VERSION, event, json
    ));
}

#[derive(Serialize)]
#[serde(crate = "near_sdk::serde")]
pub struct TlaRegisteredEvent<'a> {
    pub tla_id: &'a str,
    pub tla_type: &'a str,
    pub premium_category: &'a str,
    pub licensee: Option<&'a str>,
}

#[derive(Serialize)]
#[serde(crate = "near_sdk::serde")]
pub struct TlaActivatedEvent<'a> {
    pub tla_id: &'a str,
    pub expires_at: u64,
    pub paid_yocto: &'a str,
}

#[derive(Serialize)]
#[serde(crate = "near_sdk::serde")]
pub struct SubAccountRentedEvent<'a> {
    pub full_name: &'a str,
    pub tla_id: &'a str,
    pub owner: &'a str,
    pub rent_yocto: &'a str,
    pub expires_at: u64,
}

#[derive(Serialize)]
#[serde(crate = "near_sdk::serde")]
pub struct RenewalEvent<'a> {
    pub entity: &'a str,
    pub new_expires_at: u64,
    pub paid_yocto: &'a str,
}

#[derive(Serialize)]
#[serde(crate = "near_sdk::serde")]
pub struct WithdrawalEvent<'a> {
    pub amount_yocto: &'a str,
    pub recipient: &'a str,
}

#[derive(Serialize)]
#[serde(crate = "near_sdk::serde")]
pub struct PauseEvent<'a> {
    pub by: &'a str,
}

#[derive(Serialize)]
#[serde(crate = "near_sdk::serde")]
pub struct AdminChangeEvent<'a> {
    pub action: &'a str,
    pub account: &'a str,
    pub by: &'a str,
}

#[derive(Serialize)]
#[serde(crate = "near_sdk::serde")]
pub struct TlaSuspendedEvent<'a> {
    pub tla_id: &'a str,
    pub action: &'a str,
    pub by: &'a str,
}

#[derive(Serialize)]
#[serde(crate = "near_sdk::serde")]
pub struct RefundFailedEvent<'a> {
    pub account: &'a str,
    pub amount_yocto: &'a str,
    pub reason: &'a str,
}

#[derive(Serialize)]
#[serde(crate = "near_sdk::serde")]
pub struct SubAccountReclaimedEvent<'a> {
    pub full_name: &'a str,
    pub tla_id: &'a str,
    pub swept_to: &'a str,
}

#[derive(Serialize)]
#[serde(crate = "near_sdk::serde")]
pub struct AllowlistEvent<'a> {
    pub kind: &'a str,
    pub token: &'a str,
    pub by: &'a str,
}

#[derive(Serialize)]
#[serde(crate = "near_sdk::serde")]
pub struct MainWalletEvent<'a> {
    pub full_name: &'a str,
    pub new_wallet: &'a str,
}

#[derive(Serialize)]
#[serde(crate = "near_sdk::serde")]
pub struct FinalizeBlockedEvent<'a> {
    pub full_name: &'a str,
    pub token: &'a str,
    pub reason: &'a str,
}

#[derive(Serialize)]
#[serde(crate = "near_sdk::serde")]
pub struct MotherEvent<'a> {
    pub user: &'a str,
    pub mother: &'a str,
    pub source: &'a str,
}

#[derive(Serialize)]
#[serde(crate = "near_sdk::serde")]
pub struct MotherClearedEvent<'a> {
    pub user: &'a str,
    pub previous_mother: &'a str,
    pub by: &'a str,
}

#[derive(Serialize)]
#[serde(crate = "near_sdk::serde")]
pub struct RetractionScheduledEvent<'a> {
    pub full_name: &'a str,
    pub retraction_at: u64,
    pub by: &'a str,
}

#[derive(Serialize)]
#[serde(crate = "near_sdk::serde")]
pub struct RetractionCanceledEvent<'a> {
    pub full_name: &'a str,
    pub by: &'a str,
}

#[derive(Serialize)]
#[serde(crate = "near_sdk::serde")]
pub struct BusinessSubCapEvent<'a> {
    pub tla_id: &'a str,
    pub cap: &'a str,
    pub by: &'a str,
}

#[derive(Serialize)]
#[serde(crate = "near_sdk::serde")]
pub struct ResaleUnlockRequestedEvent<'a> {
    pub account: &'a str,
    pub by: &'a str,
}

#[derive(Serialize)]
#[serde(crate = "near_sdk::serde")]
pub struct ResaleAbortRequestedEvent<'a> {
    pub account: &'a str,
    pub by: &'a str,
}
