use actix_web::{HttpResponse, Responder, get};
use serde_json::json;

#[get("/health")]
pub async fn health_check() -> impl Responder {
    HttpResponse::Ok().json(json!(
        {
            "status": "ok"
        }
    ))
}

/// Legacy alias kept for compatibility with earlier deployments.
#[get("/heath")]
pub async fn health_check_legacy() -> impl Responder {
    HttpResponse::Ok().json(json!(
        {
            "status": "ok"
        }
    ))
}
