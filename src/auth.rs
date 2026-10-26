//! JWT issuing / verification plus the `AuthenticatedUser` extractor.
//!
//! Access tokens carry the user's role and account status so every request
//! can be authorized without a database round-trip; refresh tokens are a
//! separate `token_type` claim so they can never be used as access tokens.

use actix_web::{Error, FromRequest, HttpRequest, dev::Payload, error::ErrorUnauthorized, web};
use chrono::{Duration, Utc};
use jsonwebtoken::{DecodingKey, EncodingKey, Header, Validation, decode, encode};
use serde::{Deserialize, Serialize};
use std::future::{Ready, ready};

use crate::app_state::AppState;
use crate::error::AppError;
use crate::models::user::Role;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Claims {
    pub sub: String,
    pub email: String,
    pub name: String,
    pub role: String,
    pub account_status: String,
    pub supplier_id: Option<String>,
    pub token_type: String,
    pub exp: usize,
}

#[derive(Debug, Clone)]
pub struct TokenPair {
    pub access_token: String,
    pub refresh_token: String,
}

/// Build a JWT with the given TTL and claims.
#[allow(clippy::too_many_arguments)]
fn build_token(
    config: &crate::config::AppConfig,
    user_id: &str,
    email: &str,
    name: &str,
    role: &str,
    account_status: &str,
    supplier_id: Option<&str>,
    token_type: &str,
    ttl: Duration,
) -> Result<String, AppError> {
    let claims = Claims {
        sub: user_id.to_string(),
        email: email.to_string(),
        name: name.to_string(),
        role: role.to_string(),
        account_status: account_status.to_string(),
        supplier_id: supplier_id.map(|value| value.to_string()),
        token_type: token_type.to_string(),
        exp: (Utc::now() + ttl).timestamp() as usize,
    };

    encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(config.jwt_secret.as_bytes()),
    )
    .map_err(|error| AppError::internal(format!("token signing failed: {error}")))
}

/// Issue an access + refresh pair for a user row.
pub fn issue_tokens(
    config: &crate::config::AppConfig,
    user: &crate::models::user::UserRow,
) -> Result<TokenPair, AppError> {
    let common = (
        user.id.as_str(),
        user.email.as_str(),
        user.name.as_str(),
        user.role.as_str(),
        user.account_status.as_str(),
        user.supplier_id.as_deref(),
    );

    let access_token = build_token(
        config,
        common.0,
        common.1,
        common.2,
        common.3,
        common.4,
        common.5,
        "access",
        Duration::hours(config.access_token_ttl_hours),
    )?;

    let refresh_token = build_token(
        config,
        common.0,
        common.1,
        common.2,
        common.3,
        common.4,
        common.5,
        "refresh",
        Duration::days(config.refresh_token_ttl_days),
    )?;

    Ok(TokenPair {
        access_token,
        refresh_token,
    })
}

pub fn decode_claims(config: &crate::config::AppConfig, token: &str) -> Result<Claims, AppError> {
    decode::<Claims>(
        token,
        &DecodingKey::from_secret(config.jwt_secret.as_bytes()),
        &Validation::default(),
    )
    .map(|data| data.claims)
    .map_err(|_| AppError::unauthorized("Invalid token"))
}

/// Request-scoped authenticated identity (extractor).
#[derive(Debug, Clone)]
pub struct AuthenticatedUser {
    pub user_id: String,
    pub email: String,
    pub name: String,
    pub role: String,
    /// Carried for completeness; the token layer already enforces ACTIVE.
    #[allow(dead_code)]
    pub account_status: String,
    pub supplier_id: Option<String>,
}

impl AuthenticatedUser {
    pub fn role(&self) -> Result<Role, AppError> {
        Role::parse_or(&self.role).map_err(|_| AppError::unauthorized("Unknown role claim"))
    }

    /// Role gate used inside handlers.
    pub fn ensure_role(&self, allowed: &[Role]) -> Result<(), AppError> {
        let role = self.role()?;
        if allowed.contains(&role) {
            Ok(())
        } else {
            Err(AppError::forbidden(format!(
                "requires role {:?}",
                allowed.iter().map(Role::as_str).collect::<Vec<_>>()
            )))
        }
    }

    /// Staff = every role except portal suppliers.
    pub fn ensure_staff(&self) -> Result<(), AppError> {
        self.ensure_role(&Role::STAFF)
    }
}

impl FromRequest for AuthenticatedUser {
    type Error = Error;
    type Future = Ready<Result<Self, Self::Error>>;

    fn from_request(req: &HttpRequest, _payload: &mut Payload) -> Self::Future {
        let Some(app_state) = req.app_data::<web::Data<AppState>>() else {
            return ready(Err(ErrorUnauthorized("Missing application state")));
        };

        let Some(header_value) = req
            .headers()
            .get("Authorization")
            .and_then(|value| value.to_str().ok())
        else {
            return ready(Err(ErrorUnauthorized("Missing Authorization header")));
        };

        let Some(token) = header_value.strip_prefix("Bearer ") else {
            return ready(Err(ErrorUnauthorized("Invalid authorization scheme")));
        };

        let result = match decode_claims(&app_state.config, token) {
            Ok(claims) if claims.token_type != "access" => {
                Err(ErrorUnauthorized("Invalid access token type"))
            }
            Ok(claims) if claims.account_status != "ACTIVE" => {
                Err(ErrorUnauthorized("Account is suspended"))
            }
            Ok(claims) => Ok(AuthenticatedUser {
                user_id: claims.sub,
                email: claims.email,
                name: claims.name,
                role: claims.role,
                account_status: claims.account_status,
                supplier_id: claims.supplier_id,
            }),
            Err(_) => Err(ErrorUnauthorized("Invalid access token")),
        };

        ready(result)
    }
}
