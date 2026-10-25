//! BackPOS — Retail Store Point of Sale & Inventory Management System.
//!
//! Entry point: env loading, database pool (with retry), migrations,
//! first-boot seeding, CORS from env, and the Actix-web/HTTP router.
//! Actix-web runs on top of Tokio (its `actix-rt` reactor is a Tokio
//! multi-threaded runtime), so every async service here is Tokio-backed.

mod app_state;
mod auth;
mod config;
mod error;
mod handlers;
mod models;
mod services;
mod utils;

use std::str::FromStr;
use std::time::Duration;

use actix_cors::Cors;
use actix_web::http::header;
use actix_web::middleware::Logger;
use actix_web::{App, HttpServer, web};
use sqlx::MySqlPool;
use sqlx::mysql::{MySqlConnectOptions, MySqlPoolOptions};

use crate::app_state::AppState;
use crate::config::AppConfig;
use crate::models::user::UserRow;
use crate::utils::{hash_secret, new_id, now};

fn main() -> std::io::Result<()> {
    // `#[actix_web::main]` cannot wrap a non-async main with early env checks
    // cleanly, so the async body lives in `async_main` below.
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    runtime.block_on(entry())
}

async fn entry() -> std::io::Result<()> {
    dotenvy::dotenv().ok();
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let config = match AppConfig::from_env() {
        Ok(config) => config,
        Err(message) => {
            eprintln!("Configuration error: {message}");
            std::process::exit(1);
        }
    };

    async_main(config).await
}

async fn async_main(config: AppConfig) -> std::io::Result<()> {
    log::info!(
        "Connecting to MySQL database '{}' at {}",
        config.mysql_database,
        redact_uri(&config.mysql_uri)
    );
    let pool = connect_with_retry(&config).await;

    match sqlx::migrate!("./migrations").run(&pool).await {
        Ok(summary) => log::info!("Database migrations applied ({summary:?})"),
        Err(error) => {
            eprintln!("Migration failed: {error}");
            std::process::exit(1);
        }
    }

    if let Err(message) = seed(&pool, &config).await {
        eprintln!("First-boot seeding failed: {message}");
        std::process::exit(1);
    }

    let bind_address = format!("{}:{}", config.host, config.port);
    log::info!(
        "BackPOS API listening on http://{bind_address} (base: {})",
        config.base_url
    );

    let state = web::Data::new(AppState {
        db: pool,
        config: config.clone(),
    });

    HttpServer::new(move || {
        let cors = build_cors(&state.config.cors_origins);
        App::new()
            .app_data(state.clone())
            .app_data(web::JsonConfig::default().limit(2_097_152))
            .wrap(cors)
            .wrap(Logger::default())
            .service(handlers::health::health_check)
            .service(handlers::health::health_check_legacy)
            .service(
                web::scope("/api/v1")
                    .service(handlers::meta::meta)
                    .service(handlers::auth::login)
                    .service(handlers::auth::pin_login)
                    .service(handlers::auth::refresh)
                    .service(handlers::auth::me)
                    .service(handlers::auth::register)
                    .service(handlers::stores::list_stores)
                    .service(handlers::stores::create_store)
                    .service(handlers::users::list_users)
                    .service(handlers::users::get_user)
                    .service(handlers::users::update_user)
                    .service(handlers::users::delete_user)
                    .service(handlers::categories::list_categories)
                    .service(handlers::categories::create_category)
                    .service(handlers::categories::update_category)
                    .service(handlers::categories::delete_category)
                    .service(handlers::suppliers::list_suppliers)
                    .service(handlers::suppliers::create_supplier)
                    .service(handlers::suppliers::update_supplier)
                    .service(handlers::products::list_products)
                    .service(handlers::products::get_product_by_barcode)
                    .service(handlers::products::create_product)
                    .service(handlers::products::update_product)
                    .service(handlers::products::delete_product)
                    .service(handlers::products::check_product_stock)
                    .service(handlers::inventory::list_stock)
                    .service(handlers::inventory::list_movements)
                    .service(handlers::inventory::create_adjustment)
                    .service(handlers::inventory::create_transfer)
                    .service(handlers::transactions::create)
                    .service(handlers::transactions::list_transactions)
                    .service(handlers::transactions::get_transaction)
                    .service(handlers::transactions::void_transaction)
                    .service(handlers::transactions::refund_transaction)
                    .service(handlers::purchases::list_purchase_orders)
                    .service(handlers::purchases::get_purchase_order)
                    .service(handlers::purchases::create)
                    .service(handlers::purchases::submit)
                    .service(handlers::purchases::approve)
                    .service(handlers::purchases::cancel)
                    .service(handlers::purchases::receive)
                    .service(handlers::customers::list_customers)
                    .service(handlers::customers::create_customer)
                    .service(handlers::customers::update_customer)
                    .service(handlers::notifications::list_notifications)
                    .service(handlers::notifications::mark_read)
                    .service(handlers::notifications::mark_all_read)
                    .service(handlers::reports::daily_sales)
                    .service(handlers::reports::top_products)
                    .service(handlers::reports::profit)
                    .service(handlers::reports::employee_performance)
                    .service(handlers::reports::low_stock)
                    .service(handlers::reports::inventory_value),
            )
    })
    .bind(&bind_address)?
    .run()
    .await
}

/// Connect with patient retries so a database that boots slower than the API
/// (docker-compose, systemd ordering) never crashes the service.
async fn connect_with_retry(config: &AppConfig) -> MySqlPool {
    let options = MySqlConnectOptions::from_str(&config.mysql_uri)
        .expect("MYSQL_URI is not a valid mysql:// connection string")
        .database(&config.mysql_database);

    for attempt in 1..=30 {
        match MySqlPoolOptions::new()
            .max_connections(config.mysql_max_connections)
            .min_connections(config.mysql_min_connections)
            .acquire_timeout(Duration::from_secs(config.mysql_acquire_timeout_secs))
            .connect_with(options.clone())
            .await
        {
            Ok(pool) => return pool,
            Err(error) => {
                log::warn!("Database connection attempt {attempt}/30 failed: {error}");
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        }
    }

    eprintln!("Could not connect to MySQL after 30 attempts — aborting");
    std::process::exit(1);
}

/// CORS allowlist straight from the `CORS_ORIGINS` env var ("*" = permissive).
fn build_cors(origins: &[String]) -> Cors {
    if origins.iter().any(|origin| origin == "*") {
        return Cors::permissive();
    }

    let allowlist: std::collections::HashSet<String> = origins.iter().cloned().collect();
    Cors::default()
        .allowed_origin_fn(move |origin, _request| {
            origin
                .to_str()
                .map(|value| allowlist.contains(value))
                .unwrap_or(false)
        })
        .allowed_methods(vec!["GET", "POST", "PATCH", "DELETE", "OPTIONS"])
        .allowed_headers(vec![header::AUTHORIZATION, header::CONTENT_TYPE])
        .max_age(3600)
}

/// First-boot seeding: default store(s), admin account, default tax rule.
async fn seed(pool: &MySqlPool, config: &AppConfig) -> Result<(), String> {
    let mut tx = pool.begin().await.map_err(|error| error.to_string())?;

    // --- default physical store --------------------------------------------
    let store_id: Option<String> = sqlx::query_scalar("SELECT id FROM `STORE` WHERE code = ?")
        .bind(&config.store_code)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|error| error.to_string())?;

    let store_id = match store_id {
        Some(store_id) => store_id,
        None => {
            let store_id = new_id();
            sqlx::query(
                "INSERT INTO `STORE` (id, code, name, address, phone, is_online, created_at)
                 VALUES (?, ?, ?, ?, ?, FALSE, ?)",
            )
            .bind(&store_id)
            .bind(&config.store_code)
            .bind(&config.store_name)
            .bind(&config.store_address)
            .bind(&config.store_phone)
            .bind(now())
            .execute(&mut *tx)
            .await
            .map_err(|error| error.to_string())?;
            log::info!("Seeded store {} ({})", config.store_name, config.store_code);
            store_id
        }
    };

    // --- online storefront ---------------------------------------------------
    if config.create_online_store {
        let exists: Option<String> =
            sqlx::query_scalar("SELECT id FROM `STORE` WHERE code = 'WEB'")
                .fetch_optional(&mut *tx)
                .await
                .map_err(|error| error.to_string())?;
        if exists.is_none() {
            sqlx::query(
                "INSERT INTO `STORE` (id, code, name, address, phone, is_online, created_at)
                 VALUES (?, 'WEB', ?, NULL, NULL, TRUE, ?)",
            )
            .bind(new_id())
            .bind(format!("{} Online", config.store_name))
            .bind(now())
            .execute(&mut *tx)
            .await
            .map_err(|error| error.to_string())?;
        }
    }

    // --- bootstrap admin ------------------------------------------------------
    let admin: Option<UserRow> = sqlx::query_as::<_, UserRow>(
        "SELECT id, name, email, phone, password_hash, pin_hash, role, account_status, store_id, supplier_id, created_at
         FROM `USER` WHERE email = ?",
    )
    .bind(&config.admin_email)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|error| error.to_string())?;

    if admin.is_none() {
        let password_hash = hash_secret(&config.admin_password, config.bcrypt_cost)
            .map_err(|error| error.to_string())?;
        let pin_hash = match &config.admin_pin {
            Some(pin) => {
                Some(hash_secret(pin, config.bcrypt_cost).map_err(|error| error.to_string())?)
            }
            None => None,
        };

        let admin_id = new_id();
        sqlx::query(
            "INSERT INTO `USER`
             (id, name, email, phone, password_hash, pin_hash, role, account_status, store_id, supplier_id, created_at, updated_at)
             VALUES (?, ?, ?, NULL, ?, ?, 'ADMIN', 'ACTIVE', ?, NULL, ?, ?)",
        )
        .bind(&admin_id)
        .bind(&config.admin_name)
        .bind(&config.admin_email)
        .bind(&password_hash)
        .bind(&pin_hash)
        .bind(&store_id)
        .bind(now())
        .bind(now())
        .execute(&mut *tx)
        .await
        .map_err(|error| error.to_string())?;
        log::info!(
            "Seeded admin account {} (change ADMIN_PASSWORD in .env!)",
            config.admin_email
        );
    }

    // --- default tax configuration -------------------------------------------
    let tax_rows: i64 =
        sqlx::query_scalar("SELECT CAST(COUNT(*) AS SIGNED) FROM `TAX_CONFIGURATION`")
            .fetch_one(&mut *tx)
            .await
            .map_err(|error| error.to_string())?;

    if tax_rows == 0 {
        sqlx::query(
            "INSERT INTO `TAX_CONFIGURATION` (id, name, rate, is_default, is_active, created_at)
             VALUES (?, 'Default Tax', ?, TRUE, TRUE, ?)",
        )
        .bind(new_id())
        .bind(config.default_tax_rate)
        .bind(now())
        .execute(&mut *tx)
        .await
        .map_err(|error| error.to_string())?;
        log::info!("Seeded default tax rate {}%", config.default_tax_rate);
    }

    tx.commit().await.map_err(|error| error.to_string())?;
    Ok(())
}

/// Hide credentials when logging the DB URI.
fn redact_uri(uri: &str) -> String {
    let Some(scheme_end) = uri.find("://") else {
        return "<invalid>".to_string();
    };
    match uri[scheme_end + 3..].find('@') {
        Some(offset) => format!(
            "{}://***@{}",
            &uri[..scheme_end],
            &uri[scheme_end + 3 + offset + 1..]
        ),
        None => uri.to_string(),
    }
}
