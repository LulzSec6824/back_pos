use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub enum BarcodeType {
    #[serde(rename = "UPC-A")]
    UpcA,
    #[serde(rename = "EAN-13")]
    Ean13,
    #[serde(rename = "EAN-8")]
    Ean8,
    #[serde(rename = "UNKNOWN")]
    Unknown,
}
