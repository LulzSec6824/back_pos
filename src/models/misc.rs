//! Stores and public metadata exposed to the frontend.

use chrono::NaiveDateTime;
use rust_decimal::Decimal;
use serde::Serialize;
use sqlx::FromRow;

use crate::config::AppConfig;

#[derive(Debug, FromRow, Serialize, Clone)]
pub struct StoreRow {
    pub id: String,
    pub code: String,
    pub name: String,
    pub address: Option<String>,
    pub phone: Option<String>,
    pub is_online: bool,
    pub created_at: NaiveDateTime,
}

/// Public (auth-free) metadata so the UI can render the POS before login.
#[derive(Debug, Serialize)]
pub struct MetaDto {
    pub store_name: String,
    pub currency: String,
    pub default_tax_rate: Decimal,
    pub receipt_prefix: String,
}

impl MetaDto {
    pub fn from_config(config: &AppConfig) -> Self {
        Self {
            store_name: config.store_name.clone(),
            currency: config.currency.clone(),
            default_tax_rate: config.default_tax_rate,
            receipt_prefix: config.receipt_prefix.clone(),
        }
    }
}
