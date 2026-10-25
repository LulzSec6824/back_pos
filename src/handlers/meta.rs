use actix_web::{HttpResponse, Responder, get, web};

use crate::app_state::AppState;
use crate::models::misc::MetaDto;

/// Public, auth-free store metadata for the POS UI.
#[get("/meta")]
pub async fn meta(state: web::Data<AppState>) -> impl Responder {
    HttpResponse::Ok().json(MetaDto::from_config(&state.config))
}
