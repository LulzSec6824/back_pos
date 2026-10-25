//! Stock levels, movements log, adjustments and transfers.

use actix_web::{HttpResponse, get, post, web};
use serde_json::json;

use crate::app_state::AppState;
use crate::auth::AuthenticatedUser;
use crate::error::AppError;
use crate::handlers::{PageQuery, PageResult};
use crate::models::inventory::{
    AdjustmentInput, StockAdjustmentRow, StockLevelRow, TransferInput, parse_adjustment_type,
};
use crate::models::user::Role;
use crate::services::stock::{apply_adjustment, apply_transfer};

#[derive(Debug, serde::Deserialize, Default)]
pub struct InventoryQuery {
    pub store_id: Option<String>,
    pub low_stock: Option<bool>,
    pub search: Option<String>,
    #[serde(flatten)]
    pub page: PageQuery,
}

#[get("/inventory")]
pub async fn list_stock(
    state: web::Data<AppState>,
    user: AuthenticatedUser,
    query: web::Query<InventoryQuery>,
) -> Result<HttpResponse, AppError> {
    user.ensure_staff()?;
    let (page, per_page) = query.page.resolve(&state.config);
    let offset = query.page.offset(&state.config);
    let InventoryQuery {
        store_id,
        low_stock,
        search,
        ..
    } = query.into_inner();

    let mut conditions = vec!["p.is_active = TRUE".to_string()];
    if store_id.is_some() {
        conditions.push("sb.store_id = ?".to_string());
    }
    if search.is_some() {
        conditions.push("(p.name LIKE ? OR p.sku LIKE ?)".to_string());
    }

    let having = if low_stock == Some(true) {
        " HAVING SUM(sb.quantity) <= p.reorder_threshold"
    } else {
        ""
    };

    let base_sql = "FROM `STOCK_BATCH` sb
         JOIN `PRODUCT` p ON p.id = sb.product_id
         JOIN `STORE` st ON st.id = sb.store_id";

    let where_clause = format!(" WHERE {}", conditions.join(" AND "));

    let count_sql = format!(
        "SELECT CAST(COUNT(*) AS SIGNED) FROM (
            SELECT sb.product_id, sb.store_id
            {base_sql}{where_clause}
            GROUP BY sb.product_id, sb.store_id, p.reorder_threshold{having}
         ) levels"
    );

    let mut count_query = sqlx::query_scalar::<_, i64>(sqlx::AssertSqlSafe(count_sql.clone()));
    if let Some(store_id) = &store_id {
        count_query = count_query.bind(store_id);
    }
    if let Some(search) = &search {
        let like = format!("%{}%", search.trim());
        count_query = count_query.bind(like.clone()).bind(like);
    }
    let total = count_query
        .fetch_one(&state.db)
        .await
        .map_err(AppError::db)?;

    let rows_sql = format!(
        "SELECT sb.product_id, p.sku, p.name, p.barcode, sb.store_id, st.name AS store_name,
                CAST(COALESCE(SUM(sb.quantity), 0) AS SIGNED) AS quantity, p.reorder_threshold
         {base_sql}{where_clause}
         GROUP BY sb.product_id, p.sku, p.name, p.barcode, sb.store_id, st.name, p.reorder_threshold{having}
         ORDER BY p.name LIMIT ? OFFSET ?"
    );

    let mut rows_query = sqlx::query_as::<_, StockLevelRow>(sqlx::AssertSqlSafe(rows_sql.clone()));
    if let Some(store_id) = &store_id {
        rows_query = rows_query.bind(store_id);
    }
    if let Some(search) = &search {
        let like = format!("%{}%", search.trim());
        rows_query = rows_query.bind(like.clone()).bind(like);
    }
    let rows = rows_query
        .bind(per_page)
        .bind(offset)
        .fetch_all(&state.db)
        .await
        .map_err(AppError::db)?;

    Ok(HttpResponse::Ok().json(PageResult::new(rows, total, page, per_page)))
}

#[derive(Debug, serde::Deserialize, Default)]
pub struct MovementQuery {
    pub product_id: Option<String>,
    pub store_id: Option<String>,
    pub adjustment_type: Option<String>,
    /// Plain calendar dates (YYYY-MM-DD), expanded to full-day bounds.
    pub from: Option<chrono::NaiveDate>,
    pub to: Option<chrono::NaiveDate>,
    #[serde(flatten)]
    pub page: PageQuery,
}

#[get("/inventory/movements")]
pub async fn list_movements(
    state: web::Data<AppState>,
    user: AuthenticatedUser,
    query: web::Query<MovementQuery>,
) -> Result<HttpResponse, AppError> {
    user.ensure_staff()?;
    let (page, per_page) = query.page.resolve(&state.config);
    let offset = query.page.offset(&state.config);
    let MovementQuery {
        product_id,
        store_id,
        adjustment_type,
        from,
        to,
        ..
    } = query.into_inner();

    let mut conditions: Vec<String> = Vec::new();
    let mut binds: Vec<String> = Vec::new();
    if let Some(product_id) = &product_id {
        conditions.push("sa.product_id = ?".to_string());
        binds.push(product_id.clone());
    }
    if let Some(store_id) = &store_id {
        conditions.push("sa.store_id = ?".to_string());
        binds.push(store_id.clone());
    }
    if let Some(kind) = &adjustment_type {
        conditions.push("sa.adjustment_type = ?".to_string());
        binds.push(kind.clone());
    }
    if let Some(from) = from {
        conditions.push("sa.created_at >= ?".to_string());
        binds.push(from.and_hms_opt(0, 0, 0).unwrap().to_string());
    }
    if let Some(to) = to {
        conditions.push("sa.created_at <= ?".to_string());
        binds.push(to.and_hms_opt(23, 59, 59).unwrap().to_string());
    }

    let where_clause = if conditions.is_empty() {
        String::new()
    } else {
        format!(" WHERE {}", conditions.join(" AND "))
    };

    let count_sql =
        format!("SELECT CAST(COUNT(*) AS SIGNED) FROM `STOCK_ADJUSTMENT` sa{where_clause}");
    let mut count_query = sqlx::query_scalar::<_, i64>(sqlx::AssertSqlSafe(count_sql.clone()));
    for bind in &binds {
        count_query = count_query.bind(bind);
    }
    let total = count_query
        .fetch_one(&state.db)
        .await
        .map_err(AppError::db)?;

    let rows_sql = format!(
        "SELECT sa.id, sa.product_id, p.sku, p.name AS product_name, sa.store_id, st.name AS store_name,
                sa.adjustment_type, sa.quantity_delta, sa.unit_cost, sa.reason,
                u.name AS created_by_name, sa.created_at
         FROM `STOCK_ADJUSTMENT` sa
         JOIN `PRODUCT` p ON p.id = sa.product_id
         JOIN `STORE` st ON st.id = sa.store_id
         LEFT JOIN `USER` u ON u.id = sa.created_by
         {where_clause}
         ORDER BY sa.created_at DESC LIMIT ? OFFSET ?"
    );

    let mut rows_query =
        sqlx::query_as::<_, StockAdjustmentRow>(sqlx::AssertSqlSafe(rows_sql.clone()));
    for bind in &binds {
        rows_query = rows_query.bind(bind);
    }
    let rows = rows_query
        .bind(per_page)
        .bind(offset)
        .fetch_all(&state.db)
        .await
        .map_err(AppError::db)?;

    Ok(HttpResponse::Ok().json(PageResult::new(rows, total, page, per_page)))
}

#[post("/inventory/adjustments")]
pub async fn create_adjustment(
    state: web::Data<AppState>,
    user: AuthenticatedUser,
    body: web::Json<AdjustmentInput>,
) -> Result<HttpResponse, AppError> {
    user.ensure_role(&[Role::Admin, Role::Manager, Role::InventoryManager])?;
    let input = body.into_inner();

    let system_type =
        parse_adjustment_type(&input.adjustment_type).map_err(AppError::bad_request)?;
    if system_type.is_none() {
        // parse_adjustment_type already errors for system-only types.
        return Err(AppError::bad_request("invalid adjustment type"));
    }

    let mut tx = state.db.begin().await.map_err(AppError::db)?;
    apply_adjustment(&mut tx, &state, &input, &user.user_id).await?;
    tx.commit().await.map_err(AppError::db)?;

    Ok(HttpResponse::Created().json(json!({ "adjusted": true })))
}

#[post("/inventory/transfers")]
pub async fn create_transfer(
    state: web::Data<AppState>,
    user: AuthenticatedUser,
    body: web::Json<TransferInput>,
) -> Result<HttpResponse, AppError> {
    user.ensure_role(&[Role::Admin, Role::Manager, Role::InventoryManager])?;
    let input = body.into_inner();

    let mut tx = state.db.begin().await.map_err(AppError::db)?;
    apply_transfer(&mut tx, &state, &input, &user.user_id).await?;
    tx.commit().await.map_err(AppError::db)?;

    Ok(HttpResponse::Created().json(json!({ "transferred": true })))
}
