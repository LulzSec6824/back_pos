//! Report DTOs — all aggregates are CAST to explicit numeric types in SQL.

use chrono::NaiveDate;
use rust_decimal::Decimal;
use serde::Serialize;
use sqlx::FromRow;

#[derive(Debug, FromRow, Serialize, Clone)]
pub struct DailySalesRow {
    pub day: NaiveDate,
    pub transactions: i64,
    pub revenue: Decimal,
    pub tax: Decimal,
    pub total_discount: Decimal,
}

#[derive(Debug, FromRow, Serialize, Clone)]
pub struct TopProductRow {
    pub product_id: String,
    pub sku: String,
    pub name: String,
    pub quantity_sold: i64,
    pub revenue: Decimal,
    pub profit: Decimal,
}

#[derive(Debug, FromRow, Serialize, Clone)]
pub struct ProfitRow {
    pub revenue: Decimal,
    pub cost: Decimal,
}

#[derive(Debug, FromRow, Serialize, Clone)]
pub struct EmployeePerformanceRow {
    pub cashier_id: String,
    pub name: String,
    pub transactions: i64,
    pub revenue: Decimal,
}

#[derive(Debug, FromRow, Serialize, Clone)]
pub struct LowStockRow {
    pub product_id: String,
    pub sku: String,
    pub name: String,
    pub reorder_threshold: i32,
    pub quantity: i64,
}

#[derive(Debug, FromRow, Serialize, Clone)]
pub struct InventoryValueRow {
    pub product_id: String,
    pub sku: String,
    pub name: String,
    pub quantity: i64,
    pub cost_price: Decimal,
}

#[derive(Debug, Serialize)]
pub struct ProfitSummary {
    pub revenue: Decimal,
    pub cost: Decimal,
    pub profit: Decimal,
    /// JSON number for chart friendliness.
    pub margin_pct: f64,
}

#[derive(Debug, Serialize)]
pub struct InventoryValueSummary {
    pub items: Vec<InventoryValueRow>,
    pub total_value: Decimal,
}
