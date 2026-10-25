use crate::handlers::barcode::BarcodeType;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct GroceryItem {
    pub event: String,
    pub barcode_type: BarcodeType,
    pub product_id: String,
}

impl GroceryItem {
    /// Creates a typed `GroceryItem` from a raw scanned string matrix.
    pub fn from_scan(raw_barcode: &str) -> Self {
        let clean_barcode = raw_barcode.trim().to_string();

        let barcode_type = match clean_barcode.len() {
            8 => BarcodeType::Ean8,
            12 => BarcodeType::UpcA,
            13 => BarcodeType::Ean13,
            _ => BarcodeType::Unknown,
        };

        Self {
            event: "item_scanned".to_string(),
            barcode_type,
            product_id: clean_barcode,
        }
    }

    /// Converts the struct instance into a structured JSON string.
    pub fn to_json_string(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }

    /// Converts the struct instance into a pretty-printed, human-readable JSON string.
    pub fn to_json_pretty(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }
}
