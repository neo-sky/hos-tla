use near_sdk::serde::Serialize;
use near_sdk::{env, FunctionError};

#[derive(Debug, Serialize)]
#[serde(crate = "near_sdk::serde", tag = "code", rename_all = "snake_case")]
pub enum LockerError {
    Unauthorized,
}

impl FunctionError for LockerError {
    fn panic(&self) -> ! {
        let json = near_sdk::serde_json::to_string(self)
            .unwrap_or_else(|_| String::from(r#"{"code":"serialization_failure"}"#));
        env::panic_str(&json)
    }
}
