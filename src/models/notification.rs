//! Notifications (low-stock alerts, PO status changes, deliveries).

use chrono::NaiveDateTime;
use serde::Serialize;
use sqlx::FromRow;

#[derive(Debug, FromRow, Serialize, Clone)]
pub struct NotificationRow {
    pub id: String,
    /// Serialized as `type` for the frontend contract.
    #[serde(rename = "type")]
    pub type_name: String,
    pub title: String,
    pub message: String,
    pub audience: String,
    pub entity_type: Option<String>,
    pub entity_id: Option<String>,
    pub is_read: bool,
    pub created_at: NaiveDateTime,
}

#[derive(Debug, Clone)]
pub struct CreateNotification {
    pub type_name: String,
    pub title: String,
    pub message: String,
    pub audience: String,
    pub entity_type: Option<String>,
    pub entity_id: Option<String>,
}
