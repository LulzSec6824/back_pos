use crate::app_state::AppState;
use actix_web::{Error, FromRequest, HttpRequest, dev::Payload, error::ErrorUnauthorized, web};
use jsonwebtoken::{DecodingKey, Validation, decode};
use serde::{Deserialize, Serialize};
use std::future::{Ready, ready};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Claims {
    pub sub: String,
    pub email: String,
    pub token_type: String,
    pub exp: usize,
}

#[derive(Debug, Clone)]
pub struct AuthenticatedUser {
    pub user_id: String,
    pub _email: String,
}

impl AuthenticatedUser {
    pub fn from_claims(claims: Claims) -> Self {
        Self {
            user_id: claims.sub,
            _email: claims.email,
        }
    }
}

impl FromRequest for AuthenticatedUser {
    type Error = Error;
    type Future = Ready<Result<Self, Self::Error>>;

    fn from_request(req: &HttpRequest, _payload: &mut Payload) -> Self::Future {
        let Some(app_state) = req.app_data::<web::Data<AppState>>() else {
            return ready(Err(ErrorUnauthorized("Missing Application State")));
        };

        let Some(header_value) = req
            .headers()
            .get("Authorization")
            .and_then(|value| value.to_str().ok())
        else {
            return ready(Err(ErrorUnauthorized("Missing Authorization Headers")));
        };

        let Some(token) = header_value.strip_prefix("Bearer ") else {
            return ready(Err(ErrorUnauthorized("Invalid Authorization Scheme")));
        };

        let decoded = decode::<Claims>(
            token,
            &DecodingKey::from_secret(app_state.config.jwt_secret.as_bytes()),
            &Validation::default(),
        )
        .map_err(|_| ErrorUnauthorized("Invalid Acess Token"));

        match decoded {
            Ok(data) if data.claims.token_type == "acess" => {
                ready(Ok(AuthenticatedUser::from_claims(data.claims)))
            }
            Ok(_) => ready(Err(ErrorUnauthorized("Invalid Acess Token Type"))),
            Err(error) => ready(Err(error)),
        }
    }
}
