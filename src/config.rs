use std::env;

#[derive(Clone)]
pub struct AppConfig {
    pub host: String,
    pub port: u16,
    pub mysql_uri: String,
    pub mysql_database: String,
    pub jwt_secret: String,
    pub access_token_ttl_hours: i64,
    pub refresh_token_ttl_days: i64,
}

impl AppConfig {
    pub fn from_env() -> Result<Self, String> {
        let host = env::var("APP_HOST").unwrap_or_else(|_| "0.0.0.0".to_string());
        let port = env::var("APP_PORT")
            .unwrap_or_else(|_| "8080".to_string())
            .parse::<u16>()
            .map_err(|error| format!("APP_PORT must be a valid port: {error}"))?;

        let mysql_uri = env::var("MYSQL_URI").map_err(|_| "MONGODB_URI is required".to_string())?;

        let mysql_database = env::var("MYSQL_DATABASE").unwrap_or_else(|_| "todo_app".to_string());

        let jwt_secret =
            env::var("JWT_SECRET").map_err(|_| "JWT_SECRET is required".to_string())?;

        let access_token_ttl_hours = env::var("ACCESS_TOKEN_TTL_HOURS")
            .unwrap_or_else(|_| "12".to_string())
            .parse::<i64>()
            .map_err(|error| format!("ACCESS_TOKEN_TTL_HOURS must be a number: {error}"))?;

        let refresh_token_ttl_days = env::var("REFRESH_TOKEN_TTL_DAYS")
            .unwrap_or_else(|_| "30".to_string())
            .parse::<i64>()
            .map_err(|error| format!("REFRESH_TOKEN_TTL_DAYS must be a number: {error}"))?;

        Ok(Self {
            host,
            port,
            mysql_uri,
            mysql_database,
            jwt_secret,

            access_token_ttl_hours,
            refresh_token_ttl_days,
        })
    }
}
