use crate::config::AppConfig;
use sqlx::{MySql, Pool};

/// Shared application state injected into every handler through
/// `web::Data<AppState>`.
#[derive(Clone)]
pub struct AppState {
    pub db: Pool<MySql>,
    pub config: AppConfig,
}
