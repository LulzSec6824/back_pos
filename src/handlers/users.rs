//! User administration (admin/manager only).

use actix_web::{HttpResponse, delete, get, patch, web};
use serde_json::json;

use crate::app_state::AppState;
use crate::auth::AuthenticatedUser;
use crate::error::AppError;
use crate::handlers::{PageQuery, PageResult};
use crate::models::user::{Role, UpdateUserInput, UserRow};
use crate::utils::hash_secret;

const SELECT_USER: &str = "SELECT id, name, email, phone, password_hash, pin_hash, role, account_status, store_id, supplier_id, created_at FROM `USER`";

#[get("/users")]
pub async fn list_users(
    state: web::Data<AppState>,
    user: AuthenticatedUser,
    query: web::Query<PageQuery>,
    role: web::Query<RoleFilter>,
) -> Result<HttpResponse, AppError> {
    user.ensure_role(&[Role::Admin, Role::Manager])?;
    let (page, per_page) = query.resolve(&state.config);
    let offset = query.offset(&state.config);

    let total: i64 = if let Some(role_filter) = role.role.as_deref() {
        let role = Role::parse_or(role_filter).map_err(AppError::bad_request)?;
        sqlx::query_scalar("SELECT CAST(COUNT(*) AS SIGNED) FROM `USER` WHERE role = ?")
            .bind(role.as_str())
            .fetch_one(&state.db)
            .await
            .map_err(AppError::db)?
    } else {
        sqlx::query_scalar("SELECT CAST(COUNT(*) AS SIGNED) FROM `USER`")
            .fetch_one(&state.db)
            .await
            .map_err(AppError::db)?
    };

    let rows = if let Some(role_filter) = role.role.as_deref() {
        let role = Role::parse_or(role_filter).map_err(AppError::bad_request)?;
        sqlx::query_as::<_, UserRow>(sqlx::AssertSqlSafe(format!(
            "{SELECT_USER} WHERE role = ? ORDER BY created_at DESC LIMIT ? OFFSET ?"
        )))
        .bind(role.as_str())
        .bind(per_page)
        .bind(offset)
        .fetch_all(&state.db)
        .await
        .map_err(AppError::db)?
    } else {
        sqlx::query_as::<_, UserRow>(sqlx::AssertSqlSafe(format!(
            "{SELECT_USER} ORDER BY created_at DESC LIMIT ? OFFSET ?"
        )))
        .bind(per_page)
        .bind(offset)
        .fetch_all(&state.db)
        .await
        .map_err(AppError::db)?
    };

    Ok(HttpResponse::Ok().json(PageResult::new(rows, total, page, per_page)))
}

#[derive(Debug, serde::Deserialize)]
pub struct RoleFilter {
    pub role: Option<String>,
}

#[get("/users/{id}")]
pub async fn get_user(
    state: web::Data<AppState>,
    user: AuthenticatedUser,
    path: web::Path<String>,
) -> Result<HttpResponse, AppError> {
    user.ensure_role(&[Role::Admin, Role::Manager])?;
    let row =
        sqlx::query_as::<_, UserRow>(sqlx::AssertSqlSafe(format!("{SELECT_USER} WHERE id = ?")))
            .bind(path.into_inner())
            .fetch_optional(&state.db)
            .await
            .map_err(AppError::db)?
            .ok_or_else(|| AppError::not_found("User not found"))?;
    Ok(HttpResponse::Ok().json(json!({ "user": row })))
}

#[patch("/users/{id}")]
pub async fn update_user(
    state: web::Data<AppState>,
    user: AuthenticatedUser,
    path: web::Path<String>,
    body: web::Json<UpdateUserInput>,
) -> Result<HttpResponse, AppError> {
    user.ensure_role(&[Role::Admin, Role::Manager])?;
    let target_id = path.into_inner();

    let target =
        sqlx::query_as::<_, UserRow>(sqlx::AssertSqlSafe(format!("{SELECT_USER} WHERE id = ?")))
            .bind(&target_id)
            .fetch_optional(&state.db)
            .await
            .map_err(AppError::db)?
            .ok_or_else(|| AppError::not_found("User not found"))?;

    if target.role == "ADMIN" && !matches!(user.role(), Ok(Role::Admin)) {
        return Err(AppError::forbidden("managers cannot modify admin accounts"));
    }
    let role = match &body.role {
        Some(role) => Role::parse_or(role)
            .map_err(AppError::bad_request)?
            .as_str()
            .to_string(),
        None => target.role.clone(),
    };
    // Mirror the register rules: only admins may grant ADMIN or SUPPLIER, so
    // a manager can never escalate an account (or their own) into them.
    if (role == "ADMIN" || role == "SUPPLIER") && !matches!(user.role(), Ok(Role::Admin)) {
        return Err(AppError::forbidden(
            "only admins can assign ADMIN or SUPPLIER roles",
        ));
    }
    let account_status = match &body.account_status {
        Some(status) if ["ACTIVE", "SUSPENDED", "PENDING"].contains(&status.as_str()) => {
            status.clone()
        }
        Some(status) => {
            return Err(AppError::bad_request(format!(
                "invalid account_status: {status}"
            )));
        }
        None => target.account_status.clone(),
    };

    let password_hash = match &body.password {
        Some(password) if password.len() >= 8 => Some(
            hash_secret(password, state.config.bcrypt_cost)
                .map_err(|error| AppError::internal(error.to_string()))?,
        ),
        Some(_) => {
            return Err(AppError::bad_request(
                "password must be at least 8 characters",
            ));
        }
        None => target.password_hash.clone(),
    };
    let pin_hash = match &body.pin {
        Some(pin) if pin.len() >= 4 => Some(
            hash_secret(pin, state.config.bcrypt_cost)
                .map_err(|error| AppError::internal(error.to_string()))?,
        ),
        Some(_) => return Err(AppError::bad_request("pin must be at least 4 digits")),
        None => target.pin_hash.clone(),
    };

    sqlx::query(
        "UPDATE `USER` SET name = ?, phone = ?, role = ?, account_status = ?, password_hash = ?, pin_hash = ?, store_id = ?, supplier_id = ?
         WHERE id = ?",
    )
    .bind(body.name.clone().unwrap_or(target.name))
    .bind(body.phone.clone().or(target.phone))
    .bind(&role)
    .bind(&account_status)
    .bind(&password_hash)
    .bind(&pin_hash)
    .bind(body.store_id.clone().or(target.store_id))
    .bind(body.supplier_id.clone().or(target.supplier_id))
    .bind(&target_id)
    .execute(&state.db)
    .await
    .map_err(AppError::db)?;

    let row =
        sqlx::query_as::<_, UserRow>(sqlx::AssertSqlSafe(format!("{SELECT_USER} WHERE id = ?")))
            .bind(&target_id)
            .fetch_one(&state.db)
            .await
            .map_err(AppError::db)?;

    Ok(HttpResponse::Ok().json(json!({ "user": row })))
}

/// Soft delete — the account is suspended, never removed (transaction audit
/// trail must keep referencing it).
#[delete("/users/{id}")]
pub async fn delete_user(
    state: web::Data<AppState>,
    user: AuthenticatedUser,
    path: web::Path<String>,
) -> Result<HttpResponse, AppError> {
    user.ensure_role(&[Role::Admin])?;
    sqlx::query("UPDATE `USER` SET account_status = 'SUSPENDED' WHERE id = ?")
        .bind(path.into_inner())
        .execute(&state.db)
        .await
        .map_err(AppError::db)?;
    Ok(HttpResponse::Ok().json(json!({ "status": "SUSPENDED" })))
}
