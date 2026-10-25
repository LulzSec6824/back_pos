//! Central, environment-driven configuration.
//!
//! Every connection detail (IP, port, database) and every tunable rule of the
//! system lives in an environment variable — nothing sensitive or likely to
//! change between machines is hard-coded.

use std::env;

use rust_decimal::Decimal;

#[derive(Clone, Debug)]
pub struct AppConfig {
    // --- Server binding
    pub host: String,
    pub port: u16,
    pub base_url: String,
    pub cors_origins: Vec<String>,

    // --- Database connection
    pub mysql_uri: String,
    pub mysql_database: String,
    pub mysql_max_connections: u32,
    pub mysql_min_connections: u32,
    pub mysql_acquire_timeout_secs: u64,

    // --- Security
    pub jwt_secret: String,
    pub access_token_ttl_hours: i64,
    pub refresh_token_ttl_days: i64,
    pub bcrypt_cost: u32,

    // --- Bootstrap admin
    pub admin_name: String,
    pub admin_email: String,
    pub admin_password: String,
    pub admin_pin: Option<String>,

    // --- Default store
    pub store_code: String,
    pub store_name: String,
    pub store_address: Option<String>,
    pub store_phone: Option<String>,
    pub create_online_store: bool,

    // --- Business rules
    pub currency: String,
    pub default_tax_rate: Decimal,
    pub receipt_prefix: String,
    pub po_prefix: String,
    pub pagination_default_limit: i64,
    pub pagination_max_limit: i64,
    pub low_stock_alert_enabled: bool,
    pub auto_draft_po_enabled: bool,
    pub loyalty_earn_rate: f64,
}

fn var(key: &str) -> Result<String, String> {
    env::var(key).map_err(|_| format!("{key} is required (set it in .env)"))
}

fn var_opt(key: &str) -> Option<String> {
    env::var(key).ok().filter(|value| !value.is_empty())
}

fn var_or(key: &str, default: &str) -> String {
    var_opt(key).unwrap_or_else(|| default.to_string())
}

fn parse<T: std::str::FromStr>(key: &str, default: &str, name: &str) -> Result<T, String> {
    var_or(key, default)
        .parse::<T>()
        .map_err(|_| format!("{key} is not a valid {name}"))
}

impl AppConfig {
    pub fn from_env() -> Result<Self, String> {
        let port = var("APP_PORT")
            .ok()
            .and_then(|value| value.parse::<u16>().ok())
            .unwrap_or(8080);

        let mysql_uri = var("MYSQL_URI")?;

        Ok(Self {
            host: var_or("APP_HOST", "0.0.0.0"),
            port,
            base_url: var_or("BASE_URL", &format!("http://127.0.0.1:{port}")),
            cors_origins: var_or("CORS_ORIGINS", "*")
                .split(',')
                .map(|item| item.trim().to_string())
                .filter(|item| !item.is_empty())
                .collect(),

            mysql_uri,
            mysql_database: var_or("MYSQL_DATABASE", "retail_pos"),
            mysql_max_connections: parse("MYSQL_MAX_CONNECTIONS", "10", "number")?,
            mysql_min_connections: parse("MYSQL_MIN_CONNECTIONS", "2", "number")?,
            mysql_acquire_timeout_secs: parse("MYSQL_ACQUIRE_TIMEOUT_SECS", "10", "number")?,

            jwt_secret: var("JWT_SECRET")?,
            access_token_ttl_hours: parse("ACCESS_TOKEN_TTL_HOURS", "12", "number")?,
            refresh_token_ttl_days: parse("REFRESH_TOKEN_TTL_DAYS", "30", "number")?,
            bcrypt_cost: parse("BCRYPT_COST", "10", "number")?,

            admin_name: var_or("ADMIN_NAME", "System Administrator"),
            admin_email: var_or("ADMIN_EMAIL", "admin@backpos.local"),
            admin_password: var_or("ADMIN_PASSWORD", "admin-12345678"),
            admin_pin: var_opt("ADMIN_PIN").filter(|pin| pin.len() >= 4),

            store_code: var_or("STORE_CODE", "HQ"),
            store_name: var_or("STORE_NAME", "BackPOS Store"),
            store_address: var_opt("STORE_ADDRESS"),
            store_phone: var_opt("STORE_PHONE"),
            create_online_store: var_or("CREATE_ONLINE_STORE", "true") == "true",

            currency: var_or("STORE_CURRENCY", "USD"),
            default_tax_rate: parse::<f64>("DEFAULT_TAX_RATE", "8.00", "tax rate")?
                .to_string()
                .parse::<Decimal>()
                .map_err(|_| "DEFAULT_TAX_RATE is not a valid decimal".to_string())?,
            receipt_prefix: var_or("RECEIPT_PREFIX", "RCP"),
            po_prefix: var_or("PO_NUMBER_PREFIX", "PO"),
            pagination_default_limit: parse("PAGINATION_DEFAULT_LIMIT", "20", "number")?,
            pagination_max_limit: parse("PAGINATION_MAX_LIMIT", "100", "number")?,
            low_stock_alert_enabled: var_or("LOW_STOCK_ALERT_ENABLED", "true") == "true",
            auto_draft_po_enabled: var_or("AUTO_DRAFT_PO_ENABLED", "true") == "true",
            loyalty_earn_rate: parse("LOYALTY_EARN_RATE", "1.0", "number")?,
        })
    }
}
