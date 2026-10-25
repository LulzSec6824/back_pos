//! Users, roles and account status.

use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};
use sqlx::FromRow;

/// Fixed role set mirrored by the `USER.role` MySQL enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Admin,
    Manager,
    InventoryManager,
    Cashier,
    Supplier,
}

impl Role {
    pub const ALL: [Role; 5] = [
        Role::Admin,
        Role::Manager,
        Role::InventoryManager,
        Role::Cashier,
        Role::Supplier,
    ];

    /// The exact string stored in the database enum.
    pub fn as_str(&self) -> &'static str {
        match self {
            Role::Admin => "ADMIN",
            Role::Manager => "MANAGER",
            Role::InventoryManager => "INVENTORY_MANAGER",
            Role::Cashier => "CASHIER",
            Role::Supplier => "SUPPLIER",
        }
    }

    pub fn parse(text: &str) -> Option<Role> {
        match text {
            "ADMIN" => Some(Role::Admin),
            "MANAGER" => Some(Role::Manager),
            "INVENTORY_MANAGER" => Some(Role::InventoryManager),
            "CASHIER" => Some(Role::Cashier),
            "SUPPLIER" => Some(Role::Supplier),
            _ => None,
        }
    }

    pub fn parse_or(text: &str) -> Result<Role, String> {
        Role::parse(text).ok_or_else(|| format!("unknown role: {text}"))
    }
}

impl Serialize for Role {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

#[derive(Debug, FromRow, Serialize, Clone)]
pub struct UserRow {
    pub id: String,
    pub name: String,
    pub email: String,
    pub phone: Option<String>,
    #[serde(skip_serializing)]
    pub password_hash: Option<String>,
    #[serde(skip_serializing)]
    pub pin_hash: Option<String>,
    pub role: String,
    pub account_status: String,
    pub store_id: Option<String>,
    pub supplier_id: Option<String>,
    pub created_at: NaiveDateTime,
}

#[derive(Debug, Deserialize)]
pub struct RegisterInput {
    pub name: String,
    pub email: String,
    pub phone: Option<String>,
    #[serde(default)]
    pub password: Option<String>,
    pub role: String,
    #[serde(default)]
    pub pin: Option<String>,
    #[serde(default)]
    pub store_id: Option<String>,
    #[serde(default)]
    pub supplier_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateUserInput {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub phone: Option<String>,
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub account_status: Option<String>,
    #[serde(default)]
    pub password: Option<String>,
    #[serde(default)]
    pub pin: Option<String>,
    #[serde(default)]
    pub store_id: Option<String>,
    #[serde(default)]
    pub supplier_id: Option<String>,
}
