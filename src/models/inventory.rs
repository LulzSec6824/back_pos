//! Stock batches, stock levels and adjustments (shrinkage, damage, transfers).

use chrono::NaiveDateTime;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use sqlx::FromRow;

/// Aggregated per-product stock level for a given store.
#[derive(Debug, FromRow, Serialize, Clone)]
pub struct StockLevelRow {
    pub product_id: String,
    pub sku: String,
    pub name: String,
    pub barcode: Option<String>,
    pub store_id: String,
    pub store_name: String,
    pub quantity: i64,
    pub reorder_threshold: i32,
}

#[derive(Debug, FromRow, Serialize, Clone)]
pub struct StockAdjustmentRow {
    pub id: String,
    pub product_id: String,
    pub sku: String,
    pub product_name: String,
    pub store_id: String,
    pub store_name: String,
    pub adjustment_type: String,
    pub quantity_delta: i32,
    pub unit_cost: Decimal,
    pub reason: Option<String>,
    pub created_by_name: Option<String>,
    pub created_at: NaiveDateTime,
}

#[derive(Debug, Deserialize)]
pub struct AdjustmentInput {
    pub product_id: String,
    pub store_id: String,
    pub adjustment_type: String,
    pub delta: i32,
    #[serde(default)]
    pub reason: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct TransferInput {
    pub product_id: String,
    pub from_store_id: String,
    pub to_store_id: String,
    pub quantity: i64,
    #[serde(default)]
    pub reason: Option<String>,
}

/// Adjustment types accepted on the write path.
pub fn parse_adjustment_type(text: &str) -> Result<Option<&'static str>, String> {
    match text {
        "SHRINKAGE" => Ok(Some("SHRINKAGE")),
        "DAMAGE" => Ok(Some("DAMAGE")),
        "RETURN" => Ok(Some("RETURN")),
        "CORRECTION" => Ok(Some("CORRECTION")),
        "SALE" | "PURCHASE" | "TRANSFER_IN" | "TRANSFER_OUT" => Err(format!(
            "adjustment_type {text} can only be written by the system"
        )),
        _ => Err(format!("unknown adjustment_type: {text}")),
    }
}
