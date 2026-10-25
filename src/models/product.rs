//! Product catalog, categories and barcode helpers.

use chrono::NaiveDateTime;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use sqlx::FromRow;

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub enum BarcodeType {
    #[serde(rename = "UPC-A")]
    UpcA,
    #[serde(rename = "EAN-13")]
    Ean13,
    #[serde(rename = "EAN-8")]
    Ean8,
    #[serde(rename = "CODE39")]
    Code39,
    #[serde(rename = "UNKNOWN")]
    Unknown,
}

impl BarcodeType {
    pub fn as_str(&self) -> &'static str {
        match self {
            BarcodeType::UpcA => "UPC-A",
            BarcodeType::Ean13 => "EAN-13",
            BarcodeType::Ean8 => "EAN-8",
            BarcodeType::Code39 => "CODE39",
            BarcodeType::Unknown => "UNKNOWN",
        }
    }

    pub fn parse(text: &str) -> BarcodeType {
        match text {
            "UPC-A" => BarcodeType::UpcA,
            "EAN-13" => BarcodeType::Ean13,
            "EAN-8" => BarcodeType::Ean8,
            "CODE39" => BarcodeType::Code39,
            _ => BarcodeType::Unknown,
        }
    }

    /// Cheap heuristic used when a product is created without an explicit type.
    pub fn detect(code: &str) -> BarcodeType {
        match code.trim().len() {
            8 => BarcodeType::Ean8,
            12 => BarcodeType::UpcA,
            13 => BarcodeType::Ean13,
            5.. => BarcodeType::Code39,
            _ => BarcodeType::Unknown,
        }
    }
}

#[derive(Debug, FromRow, Serialize, Clone)]
pub struct ProductRow {
    pub id: String,
    pub sku: String,
    pub name: String,
    pub barcode: Option<String>,
    pub barcode_type: String,
    pub category_id: Option<String>,
    pub category_name: Option<String>,
    pub cost_price: Decimal,
    pub selling_price: Decimal,
    pub tax_rate_override: Option<Decimal>,
    pub unit: String,
    pub reorder_threshold: i32,
    pub is_active: bool,
    /// Total on-hand across stores (or the queried store).
    pub stock: i64,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

#[derive(Debug, Deserialize)]
pub struct CreateProductInput {
    pub sku: String,
    pub name: String,
    #[serde(default)]
    pub barcode: Option<String>,
    #[serde(default)]
    pub barcode_type: Option<String>,
    pub category_id: Option<String>,
    pub cost_price: Decimal,
    pub selling_price: Decimal,
    #[serde(default)]
    pub tax_rate_override: Option<Decimal>,
    #[serde(default)]
    pub unit: Option<String>,
    #[serde(default)]
    pub reorder_threshold: i32,
    #[serde(default)]
    pub initial_stock: i64,
    #[serde(default)]
    pub store_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateProductInput {
    #[serde(default)]
    pub sku: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub barcode: Option<String>,
    #[serde(default)]
    pub barcode_type: Option<String>,
    #[serde(default)]
    pub category_id: Option<String>,
    #[serde(default)]
    pub cost_price: Option<Decimal>,
    #[serde(default)]
    pub selling_price: Option<Decimal>,
    #[serde(default)]
    pub tax_rate_override: Option<Decimal>,
    #[serde(default)]
    pub unit: Option<String>,
    #[serde(default)]
    pub reorder_threshold: Option<i32>,
    #[serde(default)]
    pub is_active: Option<bool>,
}

#[derive(Debug, FromRow, Serialize, Clone)]
pub struct CategoryRow {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub parent_id: Option<String>,
    pub product_count: i64,
}

#[derive(Debug, Deserialize)]
pub struct CategoryInput {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub parent_id: Option<String>,
}

#[derive(Debug, FromRow, Serialize, Clone)]
pub struct SupplierRow {
    pub id: String,
    pub name: String,
    pub contact_name: Option<String>,
    pub email: Option<String>,
    pub phone: Option<String>,
    pub address: Option<String>,
    pub is_active: bool,
    pub created_at: NaiveDateTime,
}

#[derive(Debug, Deserialize)]
pub struct SupplierInput {
    pub name: String,
    #[serde(default)]
    pub contact_name: Option<String>,
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub phone: Option<String>,
    #[serde(default)]
    pub address: Option<String>,
}
