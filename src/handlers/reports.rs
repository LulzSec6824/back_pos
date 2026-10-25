//! Analytics: daily sales, top products, profit, employee performance,
//! low stock and inventory value.

use actix_web::{HttpResponse, get, web};
use serde::Deserialize;

use crate::app_state::AppState;
use crate::auth::AuthenticatedUser;
use crate::error::AppError;
use crate::models::report::{
    DailySalesRow, EmployeePerformanceRow, InventoryValueRow, InventoryValueSummary, LowStockRow,
    ProfitRow, ProfitSummary, TopProductRow,
};
use crate::models::user::Role;
use rust_decimal::prelude::ToPrimitive;

#[derive(Debug, Deserialize, Default)]
pub struct ReportQuery {
    pub from: Option<chrono::NaiveDate>,
    pub to: Option<chrono::NaiveDate>,
    pub store_id: Option<String>,
    pub limit: Option<i64>,
}

fn date_clause(
    from: &Option<chrono::NaiveDate>,
    to: &Option<chrono::NaiveDate>,
) -> (String, Vec<String>) {
    // Dates arrive as NaiveDate; compare against the date part of created_at.
    let mut clause = String::new();
    let mut binds = Vec::new();
    if let Some(from) = from {
        clause.push_str(" AND DATE(t.created_at) >= ?");
        binds.push(from.to_string());
    }
    if let Some(to) = to {
        clause.push_str(" AND DATE(t.created_at) <= ?");
        binds.push(to.to_string());
    }
    (clause, binds)
}

const STATUS_COMPLETED: &str = "t.status = 'COMPLETED'";

/// Reports are management-only.
async fn ensure_reporting(user: &AuthenticatedUser) -> Result<(), AppError> {
    user.ensure_role(&[Role::Admin, Role::Manager, Role::InventoryManager])
}

#[get("/reports/daily-sales")]
pub async fn daily_sales(
    state: web::Data<AppState>,
    user: AuthenticatedUser,
    query: web::Query<ReportQuery>,
) -> Result<HttpResponse, AppError> {
    ensure_reporting(&user).await?;
    let ReportQuery {
        from, to, store_id, ..
    } = query.into_inner();
    let (date_clause, mut binds) = date_clause(&from, &to);
    let mut store_clause = String::new();
    if let Some(store_id) = &store_id {
        store_clause = " AND t.store_id = ?".to_string();
        binds.push(store_id.clone());
    }

    let sql = format!(
        "SELECT DATE(t.created_at) AS day,
                CAST(COUNT(*) AS SIGNED) AS transactions,
                CAST(COALESCE(SUM(t.grand_total), 0) AS DECIMAL(14,2)) AS revenue,
                CAST(COALESCE(SUM(t.tax_total), 0) AS DECIMAL(14,2)) AS tax,
                CAST(COALESCE(SUM(t.discount_total), 0) AS DECIMAL(14,2)) AS total_discount
         FROM `TRANSACTION` t
         WHERE {STATUS_COMPLETED}{date_clause}{store_clause}
         GROUP BY DATE(t.created_at)
         ORDER BY day DESC"
    );

    let mut rows_query = sqlx::query_as::<_, DailySalesRow>(sqlx::AssertSqlSafe(sql.clone()));
    for bind in &binds {
        rows_query = rows_query.bind(bind);
    }
    let rows = rows_query
        .fetch_all(&state.db)
        .await
        .map_err(AppError::db)?;
    Ok(HttpResponse::Ok().json(rows))
}

#[get("/reports/top-products")]
pub async fn top_products(
    state: web::Data<AppState>,
    user: AuthenticatedUser,
    query: web::Query<ReportQuery>,
) -> Result<HttpResponse, AppError> {
    ensure_reporting(&user).await?;
    let ReportQuery {
        from,
        to,
        store_id,
        limit,
        ..
    } = query.into_inner();
    let (date_clause, mut binds) = date_clause(&from, &to);
    let mut store_clause = String::new();
    if let Some(store_id) = &store_id {
        store_clause = " AND t.store_id = ?".to_string();
        binds.push(store_id.clone());
    }
    let limit = limit.unwrap_or(10).clamp(1, 100);

    let sql = format!(
        "SELECT t.product_id, p.sku, p.name,
                CAST(SUM(t.quantity) AS SIGNED) AS quantity_sold,
                CAST(COALESCE(SUM(t.line_total), 0) AS DECIMAL(14,2)) AS revenue,
                CAST(COALESCE(SUM(t.line_total - (t.cost_price * t.quantity)), 0) AS DECIMAL(14,2)) AS profit
         FROM `TRANSACTION_ITEM` t
         JOIN `TRANSACTION` tr ON tr.id = t.transaction_id
         JOIN `PRODUCT` p ON p.id = t.product_id
         WHERE {STATUS_COMPLETED}{date_clause}{store_clause}
         GROUP BY t.product_id, p.sku, p.name
         ORDER BY revenue DESC
         LIMIT ?"
    );

    let mut rows_query = sqlx::query_as::<_, TopProductRow>(sqlx::AssertSqlSafe(sql.clone()));
    for bind in &binds {
        rows_query = rows_query.bind(bind);
    }
    let rows = rows_query
        .bind(limit)
        .fetch_all(&state.db)
        .await
        .map_err(AppError::db)?;
    Ok(HttpResponse::Ok().json(rows))
}

#[get("/reports/profit")]
pub async fn profit(
    state: web::Data<AppState>,
    user: AuthenticatedUser,
    query: web::Query<ReportQuery>,
) -> Result<HttpResponse, AppError> {
    ensure_reporting(&user).await?;
    let ReportQuery {
        from, to, store_id, ..
    } = query.into_inner();
    let (date_clause, mut binds) = date_clause(&from, &to);
    let mut store_clause = String::new();
    if let Some(store_id) = &store_id {
        store_clause = " AND t.store_id = ?".to_string();
        binds.push(store_id.clone());
    }

    let sql = format!(
        "SELECT
            CAST(COALESCE(SUM(t.line_total), 0) AS DECIMAL(14,2)) AS revenue,
            CAST(COALESCE(SUM(t.cost_price * t.quantity), 0) AS DECIMAL(14,2)) AS cost
         FROM `TRANSACTION_ITEM` t
         JOIN `TRANSACTION` tr ON tr.id = t.transaction_id
         WHERE {STATUS_COMPLETED}{date_clause}{store_clause}"
    );

    let mut rows_query = sqlx::query_as::<_, ProfitRow>(sqlx::AssertSqlSafe(sql.clone()));
    for bind in &binds {
        rows_query = rows_query.bind(bind);
    }
    let row = rows_query
        .fetch_one(&state.db)
        .await
        .map_err(AppError::db)?;

    let revenue = row.revenue;
    let cost = row.cost;
    let profit = revenue - cost;
    let margin_pct = if revenue > rust_decimal::Decimal::ZERO {
        (profit / revenue * rust_decimal::Decimal::from(100))
            .to_f64()
            .unwrap_or(0.0)
    } else {
        0.0
    };

    Ok(HttpResponse::Ok().json(ProfitSummary {
        revenue,
        cost,
        profit,
        margin_pct,
    }))
}

#[get("/reports/employee-performance")]
pub async fn employee_performance(
    state: web::Data<AppState>,
    user: AuthenticatedUser,
    query: web::Query<ReportQuery>,
) -> Result<HttpResponse, AppError> {
    ensure_reporting(&user).await?;
    let ReportQuery { from, to, .. } = query.into_inner();
    let (date_clause, binds) = date_clause(&from, &to);

    let sql = format!(
        "SELECT u.id AS cashier_id, u.name,
                CAST(COUNT(*) AS SIGNED) AS transactions,
                CAST(COALESCE(SUM(t.grand_total), 0) AS DECIMAL(14,2)) AS revenue
         FROM `TRANSACTION` t
         JOIN `USER` u ON u.id = t.cashier_id
         WHERE {STATUS_COMPLETED}{date_clause}
         GROUP BY u.id, u.name
         ORDER BY revenue DESC"
    );

    let mut rows_query =
        sqlx::query_as::<_, EmployeePerformanceRow>(sqlx::AssertSqlSafe(sql.clone()));
    for bind in &binds {
        rows_query = rows_query.bind(bind);
    }
    let rows = rows_query
        .fetch_all(&state.db)
        .await
        .map_err(AppError::db)?;

    let body: Vec<serde_json::Value> = rows
        .iter()
        .map(|row| {
            serde_json::json!({
                "cashier_id": row.cashier_id,
                "name": row.name,
                "transactions": row.transactions,
                "revenue": row.revenue,
                "avg_ticket": if row.transactions > 0 {
                    row.revenue / rust_decimal::Decimal::from(row.transactions)
                } else {
                    rust_decimal::Decimal::ZERO
                }
            })
        })
        .collect();

    Ok(HttpResponse::Ok().json(body))
}

#[get("/reports/low-stock")]
pub async fn low_stock(
    state: web::Data<AppState>,
    user: AuthenticatedUser,
    query: web::Query<ReportQuery>,
) -> Result<HttpResponse, AppError> {
    ensure_reporting(&user).await?;
    let store_id = query.store_id.clone();

    let sql = "SELECT p.id AS product_id, p.sku, p.name, p.reorder_threshold,
                CAST(COALESCE(SUM(sb.quantity), 0) AS SIGNED) AS quantity
         FROM `PRODUCT` p
         LEFT JOIN `STOCK_BATCH` sb ON sb.product_id = p.id AND (? IS NULL OR sb.store_id = ?)
         WHERE p.is_active = TRUE
         GROUP BY p.id, p.sku, p.name, p.reorder_threshold
         HAVING CAST(COALESCE(SUM(sb.quantity), 0) AS SIGNED) <= p.reorder_threshold
         ORDER BY quantity ASC";

    let rows = sqlx::query_as::<_, LowStockRow>(sql)
        .bind(&store_id)
        .bind(&store_id)
        .fetch_all(&state.db)
        .await
        .map_err(AppError::db)?;
    Ok(HttpResponse::Ok().json(rows))
}

#[get("/reports/inventory-value")]
pub async fn inventory_value(
    state: web::Data<AppState>,
    user: AuthenticatedUser,
    query: web::Query<ReportQuery>,
) -> Result<HttpResponse, AppError> {
    ensure_reporting(&user).await?;
    let store_id = query.store_id.clone();

    let sql = "SELECT p.id AS product_id, p.sku, p.name,
                CAST(COALESCE(SUM(sb.quantity), 0) AS SIGNED) AS quantity,
                p.cost_price
         FROM `PRODUCT` p
         JOIN `STOCK_BATCH` sb ON sb.product_id = p.id AND (? IS NULL OR sb.store_id = ?)
         WHERE p.is_active = TRUE
         GROUP BY p.id, p.sku, p.name, p.cost_price
         HAVING CAST(COALESCE(SUM(sb.quantity), 0) AS SIGNED) > 0
         ORDER BY (p.cost_price * COALESCE(SUM(sb.quantity), 0)) DESC";

    let rows = sqlx::query_as::<_, InventoryValueRow>(sql)
        .bind(&store_id)
        .bind(&store_id)
        .fetch_all(&state.db)
        .await
        .map_err(AppError::db)?;

    let total_value = rows
        .iter()
        .try_fold(rust_decimal::Decimal::ZERO, |acc, row| {
            Ok::<_, ()>(acc + row.cost_price * rust_decimal::Decimal::from(row.quantity))
        })
        .unwrap_or_default();

    Ok(HttpResponse::Ok().json(InventoryValueSummary {
        items: rows,
        total_value,
    }))
}
