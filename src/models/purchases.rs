//! Purchase orders and supplier deliveries.

use chrono::{NaiveDate, NaiveDateTime};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use sqlx::FromRow;

#[derive(Debug, FromRow, Serialize, Clone)]
pub struct PurchaseOrderRow {
    pub id: String,
    pub po_number: String,
    pub store_id: String,
    pub store_name: String,
    pub supplier_id: String,
    pub supplier_name: String,
    pub status: String,
    pub total: Decimal,
    pub expected_at: Option<NaiveDateTime>,
    pub created_by_name: String,
    pub item_count: i64,
    pub created_at: NaiveDateTime,
}

#[derive(Debug, FromRow, Serialize, Clone)]
pub struct PurchaseOrderItemRow {
    pub id: String,
    pub purchase_order_id: String,
    pub product_id: String,
    pub product_name: String,
    pub sku: String,
    pub quantity_ordered: i32,
    pub quantity_received: i32,
    pub unit_cost: Decimal,
    pub line_total: Decimal,
}

#[derive(Debug, FromRow, Serialize, Clone)]
pub struct DeliveryRow {
    pub id: String,
    pub purchase_order_id: String,
    pub delivery_code: Option<String>,
    pub received_by_name: String,
    pub notes: Option<String>,
    pub received_at: NaiveDateTime,
}

#[derive(Debug, Deserialize)]
pub struct CreatePurchaseOrderInput {
    pub supplier_id: String,
    #[serde(default)]
    pub store_id: Option<String>,
    #[serde(default)]
    pub expected_at: Option<NaiveDateTime>,
    #[serde(default)]
    pub notes: Option<String>,
    pub items: Vec<PurchaseOrderItemInput>,
}

#[derive(Debug, Deserialize)]
pub struct PurchaseOrderItemInput {
    pub product_id: String,
    pub quantity: i64,
    #[serde(default)]
    pub unit_cost: Option<Decimal>,
}

#[derive(Debug, Deserialize, Default)]
pub struct ReceiveDeliveryInput {
    /// One entry per physical shipment; a PO may arrive in several groups.
    #[serde(default)]
    pub deliveries: Vec<DeliveryGroupInput>,
}

#[derive(Debug, Deserialize)]
pub struct DeliveryGroupInput {
    /// Item-level receipt quantities; empty = receive all outstanding.
    #[serde(default)]
    pub items: Vec<ReceiveDeliveryItemInput>,
    #[serde(default)]
    pub delivery_code: Option<String>,
    #[serde(default)]
    pub notes: Option<String>,
    #[serde(default)]
    pub expires_at: Option<NaiveDate>,
}

#[derive(Debug, Deserialize)]
pub struct ReceiveDeliveryItemInput {
    pub purchase_order_item_id: String,
    pub quantity_received: i64,
}
