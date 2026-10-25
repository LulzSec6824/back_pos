//! Store locations.

use actix_web::{HttpResponse, get, post, web};
use serde_json::json;

use crate::app_state::AppState;
use crate::auth::AuthenticatedUser;
use crate::error::AppError;
use crate::models::misc::StoreRow;
use crate::models::user::Role;
use crate::utils::{new_id, now};

#[get("/stores")]
pub async fn list_stores(
    state: web::Data<AppState>,
    _user: AuthenticatedUser,
) -> Result<HttpResponse, AppError> {
    let rows = sqlx::query_as::<_, StoreRow>(
        "SELECT id, code, name, address, phone, is_online, created_at FROM `STORE` ORDER BY created_at",
    )
    .fetch_all(&state.db)
    .await
    .map_err(AppError::db)?;
    Ok(HttpResponse::Ok().json(rows))
}

#[derive(Debug, serde::Deserialize)]
pub struct CreateStoreInput {
    pub code: String,
    pub name: String,
    #[serde(default)]
    pub address: Option<String>,
    #[serde(default)]
    pub phone: Option<String>,
    #[serde(default)]
    pub is_online: Option<bool>,
}

#[post("/stores")]
pub async fn create_store(
    state: web::Data<AppState>,
    user: AuthenticatedUser,
    body: web::Json<CreateStoreInput>,
) -> Result<HttpResponse, AppError> {
    user.ensure_role(&[Role::Admin])?;

    let store_id = new_id();
    sqlx::query(
        "INSERT INTO `STORE` (id, code, name, address, phone, is_online, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&store_id)
    .bind(body.code.trim().to_uppercase())
    .bind(body.name.trim())
    .bind(&body.address)
    .bind(&body.phone)
    .bind(body.is_online.unwrap_or(false))
    .bind(now())
    .execute(&state.db)
    .await
    .map_err(AppError::db)?;

    let row = sqlx::query_as::<_, StoreRow>(
        "SELECT id, code, name, address, phone, is_online, created_at FROM `STORE` WHERE id = ?",
    )
    .bind(&store_id)
    .fetch_one(&state.db)
    .await
    .map_err(AppError::db)?;

    Ok(HttpResponse::Created().json(json!(row)))
}
