//! Customers.

use chrono::NaiveDateTime;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use sqlx::FromRow;

#[derive(Debug, FromRow, Serialize, Clone)]
pub struct CustomerRow {
    pub id: String,
    pub name: String,
    pub phone: Option<String>,
    pub email: Option<String>,
    pub loyalty_points: i64,
    pub is_member: bool,
    pub created_at: NaiveDateTime,
    /// Spent shown on the list view; 0 when not queried.
    #[sqlx(default)]
    pub total_spent: Option<Decimal>,
}

#[derive(Debug, Deserialize)]
pub struct CustomerInput {
    pub name: String,
    #[serde(default)]
    pub phone: Option<String>,
    #[serde(default)]
    pub email: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateCustomerInput {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub phone: Option<String>,
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub loyalty_points: Option<i64>,
}
