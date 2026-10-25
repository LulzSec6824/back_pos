//! Product catalog CRUD + barcode lookup + price history.

use actix_web::{HttpResponse, delete, get, patch, post, web};
use rust_decimal::Decimal;
use serde_json::json;

use crate::app_state::AppState;
use crate::auth::AuthenticatedUser;
use crate::error::AppError;
use crate::handlers::{PageQuery, PageResult};
use crate::models::product::{BarcodeType, CreateProductInput, ProductRow, UpdateProductInput};
use crate::models::user::Role;
use crate::services::sales::resolve_store_id;
use crate::services::stock::{add_batch, check_low_stock};
use crate::utils::{new_id, now};

#[derive(Debug, serde::Deserialize, Default)]
pub struct ProductQuery {
    pub search: Option<String>,
    pub category_id: Option<String>,
    pub barcode: Option<String>,
    pub low_stock: Option<bool>,
    /// true = only active, false = only inactive, omitted = all.
    pub active: Option<bool>,
    pub store_id: Option<String>,
    #[serde(flatten)]
    pub page: PageQuery,
}

/// Product list select. `stock` comes from a correlated subquery (per-store
/// when `store_scoped`), so no GROUP BY is needed anywhere.
fn product_select(store_scoped: bool) -> String {
    let store_clause = if store_scoped {
        "AND sb.store_id = ?"
    } else {
        ""
    };
    format!(
        "SELECT p.id, p.sku, p.name, p.barcode, p.barcode_type, p.category_id, c.name AS category_name,
                p.cost_price, p.selling_price, p.tax_rate_override, p.unit, p.reorder_threshold, p.is_active,
                CAST(COALESCE((SELECT SUM(sb.quantity) FROM `STOCK_BATCH` sb
                               WHERE sb.product_id = p.id {store_clause}), 0) AS SIGNED) AS stock,
                p.created_at, p.updated_at
         FROM `PRODUCT` p
         LEFT JOIN `CATEGORY` c ON c.id = p.category_id"
    )
}

/// Store-agnostic low-stock predicate for WHERE clauses.
const LOW_STOCK_PREDICATE: &str = "CAST(COALESCE((SELECT SUM(sb2.quantity) FROM `STOCK_BATCH` sb2 WHERE sb2.product_id = p.id), 0) AS SIGNED) <= p.reorder_threshold";

#[get("/products")]
pub async fn list_products(
    state: web::Data<AppState>,
    user: AuthenticatedUser,
    query: web::Query<ProductQuery>,
) -> Result<HttpResponse, AppError> {
    user.ensure_staff()?;
    let (page, per_page) = query.page.resolve(&state.config);
    let offset = query.page.offset(&state.config);
    let ProductQuery {
        search,
        category_id,
        barcode,
        low_stock,
        active,
        store_id,
        ..
    } = query.into_inner();

    let mut conditions: Vec<String> = Vec::new();
    let mut binds: Vec<String> = Vec::new();

    if let Some(search) = &search {
        let like = format!("%{}%", search.trim());
        conditions.push("(p.name LIKE ? OR p.sku LIKE ? OR p.barcode LIKE ?)".to_string());
        binds.push(like.clone());
        binds.push(like.clone());
        binds.push(like);
    }
    if let Some(category_id) = &category_id {
        conditions.push("p.category_id = ?".to_string());
        binds.push(category_id.clone());
    }
    if let Some(barcode) = &barcode {
        conditions.push("p.barcode = ?".to_string());
        binds.push(barcode.clone());
    }
    match active {
        Some(true) => conditions.push("p.is_active = TRUE".to_string()),
        Some(false) => conditions.push("p.is_active = FALSE".to_string()),
        None => {}
    }
    if low_stock == Some(true) {
        conditions.push(LOW_STOCK_PREDICATE.to_string());
    }

    let where_clause = if conditions.is_empty() {
        String::new()
    } else {
        format!(" WHERE {}", conditions.join(" AND "))
    };

    let count_sql = format!("SELECT CAST(COUNT(*) AS SIGNED) FROM `PRODUCT` p{where_clause}");
    let mut count_query = sqlx::query_scalar::<_, i64>(sqlx::AssertSqlSafe(count_sql.clone()));
    for bind in &binds {
        count_query = count_query.bind(bind);
    }
    let total = count_query
        .fetch_one(&state.db)
        .await
        .map_err(AppError::db)?;

    let rows_sql = format!(
        "{}{} ORDER BY p.name LIMIT ? OFFSET ?",
        product_select(store_id.is_some()),
        if where_clause.is_empty() {
            String::new()
        } else {
            format!(" {where_clause}")
        }
    )
    .to_string();

    let mut rows_query = sqlx::query_as::<_, ProductRow>(sqlx::AssertSqlSafe(rows_sql.clone()));
    for bind in &binds {
        rows_query = rows_query.bind(bind);
    }
    if let Some(store_id) = &store_id {
        rows_query = rows_query.bind(store_id);
    }
    let rows = rows_query
        .bind(per_page)
        .bind(offset)
        .fetch_all(&state.db)
        .await
        .map_err(AppError::db)?;

    Ok(HttpResponse::Ok().json(PageResult::new(rows, total, page, per_page)))
}

/// Barcode scan lookup — exact barcode match first, then SKU.
#[get("/products/barcode/{code}")]
pub async fn get_product_by_barcode(
    state: web::Data<AppState>,
    user: AuthenticatedUser,
    path: web::Path<String>,
) -> Result<HttpResponse, AppError> {
    user.ensure_staff()?;
    let code = path.into_inner();

    let row = sqlx::query_as::<_, ProductRow>(sqlx::AssertSqlSafe(format!(
        "{} WHERE p.is_active = TRUE AND (p.barcode = ? OR p.sku = ?) LIMIT 1",
        product_select(false)
    )))
    .bind(&code)
    .bind(&code)
    .fetch_optional(&state.db)
    .await
    .map_err(AppError::db)?
    .ok_or_else(|| AppError::not_found(format!("No product with barcode/SKU {code}")))?;

    Ok(HttpResponse::Ok().json(row))
}

async fn fetch_product(state: &AppState, id: &str) -> Result<ProductRow, AppError> {
    sqlx::query_as::<_, ProductRow>(sqlx::AssertSqlSafe(format!(
        "{} WHERE p.id = ?",
        product_select(false)
    )))
    .bind(id)
    .fetch_optional(&state.db)
    .await
    .map_err(AppError::db)?
    .ok_or_else(|| AppError::not_found("Product not found"))
}

#[post("/products")]
pub async fn create_product(
    state: web::Data<AppState>,
    user: AuthenticatedUser,
    body: web::Json<CreateProductInput>,
) -> Result<HttpResponse, AppError> {
    user.ensure_role(&[Role::Admin, Role::Manager, Role::InventoryManager])?;
    let input = body.into_inner();

    if input.cost_price < Decimal::ZERO || input.selling_price < Decimal::ZERO {
        return Err(AppError::bad_request("prices must not be negative"));
    }

    let product_id = new_id();
    let barcode_type = input
        .barcode_type
        .as_deref()
        .map(BarcodeType::parse)
        .map(|kind| kind.as_str().to_string())
        .or_else(|| {
            input
                .barcode
                .as_deref()
                .map(|code| BarcodeType::detect(code).as_str().to_string())
        })
        .unwrap_or_else(|| BarcodeType::Unknown.as_str().to_string());

    let mut tx = state.db.begin().await.map_err(AppError::db)?;

    sqlx::query(
        "INSERT INTO `PRODUCT`
         (id, sku, name, barcode, barcode_type, category_id, cost_price, selling_price, tax_rate_override, unit, reorder_threshold, is_active, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, TRUE, ?, ?)",
    )
    .bind(&product_id)
    .bind(input.sku.trim().to_uppercase())
    .bind(input.name.trim())
    .bind(&input.barcode)
    .bind(&barcode_type)
    .bind(&input.category_id)
    .bind(input.cost_price)
    .bind(input.selling_price)
    .bind(input.tax_rate_override)
    .bind(input.unit.as_deref().unwrap_or("PIECE"))
    .bind(input.reorder_threshold)
    .bind(now())
    .bind(now())
    .execute(&mut *tx)
    .await
    .map_err(AppError::db)?;

    if input.initial_stock > 0 {
        let store_id = resolve_store_id(&mut tx, &state, input.store_id.as_deref(), None).await?;
        add_batch(
            &mut tx,
            &product_id,
            &store_id,
            input.initial_stock,
            input.cost_price,
            Some("INITIAL"),
            None,
            "PURCHASE",
            Some("Initial stock at product creation"),
            Some(&user.user_id),
        )
        .await?;
    }

    tx.commit().await.map_err(AppError::db)?;

    let row = fetch_product(&state, &product_id).await?;
    Ok(HttpResponse::Created().json(row))
}

#[patch("/products/{id}")]
pub async fn update_product(
    state: web::Data<AppState>,
    user: AuthenticatedUser,
    path: web::Path<String>,
    body: web::Json<UpdateProductInput>,
) -> Result<HttpResponse, AppError> {
    user.ensure_role(&[Role::Admin, Role::Manager, Role::InventoryManager])?;
    let id = path.into_inner();

    let existing = fetch_product(&state, &id).await?;

    let new_cost = body.cost_price.unwrap_or(existing.cost_price);
    let new_selling = body.selling_price.unwrap_or(existing.selling_price);
    if new_cost < Decimal::ZERO || new_selling < Decimal::ZERO {
        return Err(AppError::bad_request("prices must not be negative"));
    }

    sqlx::query(
        "UPDATE `PRODUCT` SET sku = ?, name = ?, barcode = ?, barcode_type = ?, category_id = ?,
         cost_price = ?, selling_price = ?, tax_rate_override = ?, unit = ?, reorder_threshold = ?, is_active = ?
         WHERE id = ?",
    )
    .bind(body.sku.clone().map(|sku| sku.trim().to_uppercase()).unwrap_or(existing.sku.clone()))
    .bind(body.name.clone().unwrap_or(existing.name.clone()))
    .bind(body.barcode.clone().or(existing.barcode.clone()))
    .bind(body.barcode_type.clone().unwrap_or(existing.barcode_type.clone()))
    .bind(body.category_id.clone().or(existing.category_id))
    .bind(new_cost)
    .bind(new_selling)
    .bind(body.tax_rate_override.or(existing.tax_rate_override))
    .bind(body.unit.clone().unwrap_or(existing.unit.clone()))
    .bind(body.reorder_threshold.unwrap_or(existing.reorder_threshold))
    .bind(body.is_active.unwrap_or(existing.is_active))
    .bind(&id)
    .execute(&state.db)
    .await
    .map_err(AppError::db)?;

    // Historical data management: record price changes.
    if new_cost != existing.cost_price || new_selling != existing.selling_price {
        sqlx::query(
            "INSERT INTO `PRODUCT_PRICE_HISTORY`
             (id, product_id, old_cost_price, old_selling_price, new_cost_price, new_selling_price, changed_by, changed_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(new_id())
        .bind(&id)
        .bind(existing.cost_price)
        .bind(existing.selling_price)
        .bind(new_cost)
        .bind(new_selling)
        .bind(&user.user_id)
        .bind(now())
        .execute(&state.db)
        .await
        .map_err(AppError::db)?;
    }

    let row = fetch_product(&state, &id).await?;
    Ok(HttpResponse::Ok().json(row))
}

/// Soft delete (deactivate) — receipts reference products forever.
#[delete("/products/{id}")]
pub async fn delete_product(
    state: web::Data<AppState>,
    user: AuthenticatedUser,
    path: web::Path<String>,
) -> Result<HttpResponse, AppError> {
    user.ensure_role(&[Role::Admin, Role::Manager])?;
    sqlx::query("UPDATE `PRODUCT` SET is_active = FALSE WHERE id = ?")
        .bind(path.into_inner())
        .execute(&state.db)
        .await
        .map_err(AppError::db)?;
    Ok(HttpResponse::Ok().json(json!({ "is_active": false })))
}

/// Trigger the low-stock watcher manually for a product (admin/keeper tool).
#[post("/products/{id}/check-stock")]
pub async fn check_product_stock(
    state: web::Data<AppState>,
    user: AuthenticatedUser,
    path: web::Path<String>,
) -> Result<HttpResponse, AppError> {
    user.ensure_role(&[Role::Admin, Role::Manager, Role::InventoryManager])?;
    let product_id = path.into_inner();

    let mut tx = state.db.begin().await.map_err(AppError::db)?;
    check_low_stock(&mut tx, &state.config, &product_id, &user.user_id).await?;
    tx.commit().await.map_err(AppError::db)?;

    Ok(HttpResponse::Ok().json(json!({ "checked": true })))
}
