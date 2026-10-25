//! Authentication: password login, staff PIN login, refresh, me, register.

use actix_web::{HttpResponse, get, post, web};
use serde::Deserialize;
use serde::Serialize;

use crate::app_state::AppState;
use crate::auth::{AuthenticatedUser, issue_tokens};
use crate::error::AppError;
use crate::models::user::{RegisterInput, Role, UserRow};
use crate::utils::{hash_secret, new_id, now, verify_secret};

#[derive(Debug, Deserialize)]
pub struct LoginInput {
    pub email: String,
    pub password: String,
}

#[derive(Debug, Deserialize)]
pub struct PinLoginInput {
    pub pin: String,
}

#[derive(Debug, Deserialize)]
pub struct RefreshInput {
    pub refresh_token: String,
}

#[derive(Debug, Serialize)]
pub struct LoginResponse {
    pub access_token: String,
    pub refresh_token: String,
    pub user: UserRow,
}

async fn respond_with_user(state: &AppState, user: &UserRow) -> Result<HttpResponse, AppError> {
    let tokens = issue_tokens(&state.config, user)?;
    Ok(HttpResponse::Ok().json(LoginResponse {
        access_token: tokens.access_token,
        refresh_token: tokens.refresh_token,
        user: user.clone(),
    }))
}

#[post("/auth/login")]
pub async fn login(
    state: web::Data<AppState>,
    body: web::Json<LoginInput>,
) -> Result<HttpResponse, AppError> {
    let user = sqlx::query_as::<_, UserRow>(
        "SELECT id, name, email, phone, password_hash, pin_hash, role, account_status, store_id, supplier_id, created_at
         FROM `USER` WHERE email = ?",
    )
    .bind(body.email.trim().to_lowercase())
    .fetch_optional(&state.db)
    .await
    .map_err(AppError::db)?
    .ok_or_else(|| AppError::unauthorized("Invalid email or password"))?;

    if user.account_status != "ACTIVE" {
        return Err(AppError::forbidden("Account is not active"));
    }

    let Some(password_hash) = &user.password_hash else {
        return Err(AppError::unauthorized("Invalid email or password"));
    };
    if !verify_secret(&body.password, password_hash) {
        return Err(AppError::unauthorized("Invalid email or password"));
    }

    respond_with_user(&state, &user).await
}

/// Fast PIN login for retail staff (cashiers at the register).
#[post("/auth/pin")]
pub async fn pin_login(
    state: web::Data<AppState>,
    body: web::Json<PinLoginInput>,
) -> Result<HttpResponse, AppError> {
    if body.pin.trim().is_empty() {
        return Err(AppError::bad_request("PIN is required"));
    }

    // Staff rows with a PIN; suppliers authenticate by password only.
    let staff = sqlx::query_as::<_, UserRow>(
        "SELECT id, name, email, phone, password_hash, pin_hash, role, account_status, store_id, supplier_id, created_at
         FROM `USER`
         WHERE pin_hash IS NOT NULL AND account_status = 'ACTIVE' AND role != 'SUPPLIER'",
    )
    .fetch_all(&state.db)
    .await
    .map_err(AppError::db)?;

    let matched = staff
        .into_iter()
        .find(|user| {
            user.pin_hash
                .as_deref()
                .map(|hash| verify_secret(body.pin.trim(), hash))
                .unwrap_or(false)
        })
        .ok_or_else(|| AppError::unauthorized("Invalid PIN"))?;

    respond_with_user(&state, &matched).await
}

/// Rotate tokens. Refresh tokens are single-purpose (`token_type = refresh`)
/// and re-validated against the account status.
#[post("/auth/refresh")]
pub async fn refresh(
    state: web::Data<AppState>,
    body: web::Json<RefreshInput>,
) -> Result<HttpResponse, AppError> {
    let claims = crate::auth::decode_claims(&state.config, &body.refresh_token)?;
    if claims.token_type != "refresh" {
        return Err(AppError::unauthorized("Invalid refresh token"));
    }

    let user = sqlx::query_as::<_, UserRow>(
        "SELECT id, name, email, phone, password_hash, pin_hash, role, account_status, store_id, supplier_id, created_at
         FROM `USER` WHERE id = ?",
    )
    .bind(&claims.sub)
    .fetch_optional(&state.db)
    .await
    .map_err(AppError::db)?
    .ok_or_else(|| AppError::unauthorized("Invalid refresh token"))?;

    if user.account_status != "ACTIVE" {
        return Err(AppError::forbidden("Account is not active"));
    }

    respond_with_user(&state, &user).await
}

/// Current identity, re-read from the database.
#[get("/auth/me")]
pub async fn me(
    state: web::Data<AppState>,
    user: AuthenticatedUser,
) -> Result<HttpResponse, AppError> {
    let row = sqlx::query_as::<_, UserRow>(
        "SELECT id, name, email, phone, password_hash, pin_hash, role, account_status, store_id, supplier_id, created_at
         FROM `USER` WHERE id = ?",
    )
    .bind(&user.user_id)
    .fetch_optional(&state.db)
    .await
    .map_err(AppError::db)?
    .ok_or_else(|| AppError::unauthorized("Invalid token"))?;

    Ok(HttpResponse::Ok().json(serde_json::json!({ "user": row })))
}

/// Create a user account (admin/manager only). Managers cannot create ADMIN
/// or SUPPLIER accounts.
#[post("/auth/register")]
pub async fn register(
    state: web::Data<AppState>,
    user: AuthenticatedUser,
    body: web::Json<RegisterInput>,
) -> Result<HttpResponse, AppError> {
    user.ensure_role(&[Role::Admin, Role::Manager])?;
    let role = Role::parse_or(&body.role).map_err(AppError::bad_request)?;

    if role == Role::Admin && !matches!(user.role(), Ok(Role::Admin)) {
        return Err(AppError::forbidden("only admins can create admin accounts"));
    }
    if role == Role::Supplier && !matches!(user.role(), Ok(Role::Admin)) {
        return Err(AppError::forbidden(
            "only admins can create supplier accounts",
        ));
    }
    if body.password.is_none() && body.pin.is_none() {
        return Err(AppError::bad_request("password or pin is required"));
    }
    if let Some(pin) = &body.pin
        && pin.len() < 4
    {
        return Err(AppError::bad_request("pin must be at least 4 digits"));
    }

    let user_id = new_id();
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
        None => None,
    };
    let pin_hash = match &body.pin {
        Some(pin) => Some(
            hash_secret(pin, state.config.bcrypt_cost)
                .map_err(|error| AppError::internal(error.to_string()))?,
        ),
        None => None,
    };

    let mut tx = state.db.begin().await.map_err(AppError::db)?;

    sqlx::query(
        "INSERT INTO `USER`
         (id, name, email, phone, password_hash, pin_hash, role, account_status, store_id, supplier_id, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, 'ACTIVE', ?, ?, ?, ?)",
    )
    .bind(&user_id)
    .bind(body.name.trim())
    .bind(body.email.trim().to_lowercase())
    .bind(&body.phone)
    .bind(&password_hash)
    .bind(&pin_hash)
    .bind(role.as_str())
    .bind(&body.store_id)
    .bind(&body.supplier_id)
    .bind(now())
    .bind(now())
    .execute(&mut *tx)
    .await
    .map_err(AppError::db)?;

    // Role subtype rows (1:1 specialisation).
    let employee_code = format!("EMP-{}", user_id[..8].to_uppercase());
    if role == Role::Cashier {
        sqlx::query("INSERT INTO `CASHIER` (user_id, employee_code) VALUES (?, ?)")
            .bind(&user_id)
            .bind(&employee_code)
            .execute(&mut *tx)
            .await
            .map_err(AppError::db)?;
    }
    if role == Role::InventoryManager {
        sqlx::query(
            "INSERT INTO `INVENTORY_MANAGER` (user_id, employee_code, department) VALUES (?, ?, 'Inventory')",
        )
        .bind(&user_id)
        .bind(&employee_code)
        .execute(&mut *tx)
        .await
        .map_err(AppError::db)?;
    }

    tx.commit().await.map_err(AppError::db)?;

    let row = sqlx::query_as::<_, UserRow>(
        "SELECT id, name, email, phone, password_hash, pin_hash, role, account_status, store_id, supplier_id, created_at
         FROM `USER` WHERE id = ?",
    )
    .bind(&user_id)
    .fetch_one(&state.db)
    .await
    .map_err(AppError::db)?;

    Ok(HttpResponse::Created().json(serde_json::json!({ "user": row })))
}
