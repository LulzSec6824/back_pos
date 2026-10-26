//! Small shared helpers used across handlers and services.

use chrono::{NaiveDateTime, Utc};
use rust_decimal::Decimal;
use uuid::Uuid;

pub fn new_id() -> String {
    Uuid::new_v4().to_string()
}

pub fn now() -> NaiveDateTime {
    Utc::now().naive_utc()
}

pub fn hash_secret(plain: &str, cost: u32) -> Result<String, bcrypt::BcryptError> {
    bcrypt::hash(plain, cost)
}

pub fn verify_secret(plain: &str, hash: &str) -> bool {
    bcrypt::verify(plain, hash).unwrap_or(false)
}

/// HTTP-only money rounding helper: rounds a decimal to 2 fraction digits,
/// half-up (away from zero), suitable for receipt totals.
pub fn round2(value: Decimal) -> Decimal {
    value.round_dp_with_strategy(2, rust_decimal::RoundingStrategy::MidpointAwayFromZero)
}

/// Generate receipt / PO document numbers: `PREFIX-YYYYMMDD-RRRRRR`.
pub fn document_number(prefix: &str) -> String {
    let random: u32 = rand::random::<u32>() % 1_000_000;
    format!("{}-{}-{:06}", prefix, Utc::now().format("%Y%m%d"), random)
}
