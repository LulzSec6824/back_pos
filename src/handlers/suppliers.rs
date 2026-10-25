//! Suppliers.

use actix_web::{HttpResponse, get, patch, post, web};
use serde::Deserialize;

use crate::app_state::AppState;
use crate::auth::AuthenticatedUser;
use crate::error::AppError;
use crate::models::product::{SupplierInput, SupplierRow};
use crate::models::user::Role;
use crate::utils::{new_id, now};

const SELECT_SUPPLIER: &str =
    "SELECT id, name, contact_name, email, phone, address, is_active, created_at FROM `SUPPLIER`";

#[get("/suppliers")]
pub async fn list_suppliers(
    state: web::Data<AppState>,
    _user: AuthenticatedUser,
) -> Result<HttpResponse, AppError> {
    let rows = sqlx::query_as::<_, SupplierRow>(sqlx::AssertSqlSafe(format!(
        "{SELECT_SUPPLIER} ORDER BY name"
    )))
    .fetch_all(&state.db)
    .await
    .map_err(AppError::db)?;
    Ok(HttpResponse::Ok().json(rows))
}

#[post("/suppliers")]
pub async fn create_supplier(
    state: web::Data<AppState>,
    user: AuthenticatedUser,
    body: web::Json<SupplierInput>,
) -> Result<HttpResponse, AppError> {
    user.ensure_role(&[Role::Admin, Role::Manager, Role::InventoryManager])?;
    let supplier_id = new_id();

    sqlx::query(
        "INSERT INTO `SUPPLIER` (id, name, contact_name, email, phone, address, is_active, created_at)
         VALUES (?, ?, ?, ?, ?, ?, TRUE, ?)",
    )
    .bind(&supplier_id)
    .bind(body.name.trim())
    .bind(&body.contact_name)
    .bind(&body.email)
    .bind(&body.phone)
    .bind(&body.address)
    .bind(now())
    .execute(&state.db)
    .await
    .map_err(AppError::db)?;

    let row = sqlx::query_as::<_, SupplierRow>(sqlx::AssertSqlSafe(format!(
        "{SELECT_SUPPLIER} WHERE id = ?"
    )))
    .bind(&supplier_id)
    .fetch_one(&state.db)
    .await
    .map_err(AppError::db)?;
    Ok(HttpResponse::Created().json(row))
}

#[derive(Debug, Deserialize)]
pub struct UpdateSupplierInput {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub contact_name: Option<String>,
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub phone: Option<String>,
    #[serde(default)]
    pub address: Option<String>,
    #[serde(default)]
    pub is_active: Option<bool>,
}

#[patch("/suppliers/{id}")]
pub async fn update_supplier(
    state: web::Data<AppState>,
    user: AuthenticatedUser,
    path: web::Path<String>,
    body: web::Json<UpdateSupplierInput>,
) -> Result<HttpResponse, AppError> {
    user.ensure_role(&[Role::Admin, Role::Manager, Role::InventoryManager])?;
    let id = path.into_inner();

    let existing = sqlx::query_as::<_, SupplierRow>(sqlx::AssertSqlSafe(format!(
        "{SELECT_SUPPLIER} WHERE id = ?"
    )))
    .bind(&id)
    .fetch_optional(&state.db)
    .await
    .map_err(AppError::db)?
    .ok_or_else(|| AppError::not_found("Supplier not found"))?;

    sqlx::query(
        "UPDATE `SUPPLIER` SET name = ?, contact_name = ?, email = ?, phone = ?, address = ?, is_active = ?
         WHERE id = ?",
    )
    .bind(body.name.clone().unwrap_or(existing.name))
    .bind(body.contact_name.clone().or(existing.contact_name.clone()))
    .bind(body.email.clone().or(existing.email))
    .bind(body.phone.clone().or(existing.phone))
    .bind(body.address.clone().or(existing.address))
    .bind(body.is_active.unwrap_or(existing.is_active))
    .bind(&id)
    .execute(&state.db)
    .await
    .map_err(AppError::db)?;

    let row = sqlx::query_as::<_, SupplierRow>(sqlx::AssertSqlSafe(format!(
        "{SELECT_SUPPLIER} WHERE id = ?"
    )))
    .bind(&id)
    .fetch_one(&state.db)
    .await
    .map_err(AppError::db)?;
    Ok(HttpResponse::Ok().json(row))
}
