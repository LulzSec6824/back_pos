//! Sales transactions, items and payments (the POS checkout core).

use chrono::NaiveDateTime;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use sqlx::FromRow;

#[derive(Debug, FromRow, Serialize, Clone)]
pub struct TransactionRow {
    pub id: String,
    pub receipt_number: String,
    pub store_id: String,
    pub store_name: String,
    pub cashier_id: String,
    pub cashier_name: String,
    pub customer_id: Option<String>,
    pub customer_name: Option<String>,
    pub subtotal: Decimal,
    pub discount_total: Decimal,
    pub tax_total: Decimal,
    pub grand_total: Decimal,
    pub status: String,
    pub item_count: i64,
    pub created_at: NaiveDateTime,
}

#[derive(Debug, FromRow, Serialize, Clone)]
pub struct TransactionItemRow {
    pub id: String,
    pub transaction_id: String,
    pub product_id: String,
    pub sku: String,
    pub name: String,
    pub quantity: i32,
    pub unit_price: Decimal,
    pub cost_price: Decimal,
    pub line_discount: Decimal,
    pub tax_rate: Decimal,
    pub line_total: Decimal,
}

#[derive(Debug, FromRow, Serialize, Clone)]
pub struct PaymentRow {
    pub id: String,
    pub transaction_id: String,
    pub method: String,
    pub amount: Decimal,
    pub reference: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ItemInput {
    pub product_id: String,
    pub quantity: i64,
    #[serde(default)]
    pub line_discount: Option<Decimal>,
}

#[derive(Debug, Deserialize)]
pub struct PaymentInput {
    pub method: String,
    #[serde(default)]
    pub amount: Option<Decimal>,
    #[serde(default)]
    pub reference: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CreateTransactionInput {
    pub items: Vec<ItemInput>,
    #[serde(default)]
    pub customer_id: Option<String>,
    #[serde(default)]
    pub payments: Vec<PaymentInput>,
    /// Ignored when `payments` are given explicitly.
    #[serde(default)]
    pub payment_method: Option<String>,
    #[serde(default)]
    pub store_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct VoidInput {
    #[serde(default)]
    pub reason: Option<String>,
}
