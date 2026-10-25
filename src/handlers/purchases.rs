//! Purchase orders + supplier deliveries.

use actix_web::{HttpResponse, get, post, web};
use serde::Serialize;
use serde_json::json;

use crate::app_state::AppState;
use crate::auth::AuthenticatedUser;
use crate::error::AppError;
use crate::handlers::{PageQuery, PageResult};
use crate::models::purchases::{
    CreatePurchaseOrderInput, DeliveryRow, PurchaseOrderItemRow, PurchaseOrderRow,
    ReceiveDeliveryInput,
};
use crate::models::user::Role;
use crate::services::purchases::{
    create_purchase_order, fetch_order_deliveries, fetch_order_header, fetch_order_items,
    receive_delivery, transition_purchase_order,
};

/// Flat PO detail: `{...header fields, items, deliveries}`.
#[derive(Debug, Serialize)]
pub struct PurchaseOrderDetail {
    #[serde(flatten)]
    pub order: PurchaseOrderRow,
    pub items: Vec<PurchaseOrderItemRow>,
    pub deliveries: Vec<DeliveryRow>,
}

#[derive(Debug, serde::Deserialize, Default)]
pub struct PurchaseQuery {
    pub status: Option<String>,
    pub supplier_id: Option<String>,
    pub store_id: Option<String>,
    #[serde(flatten)]
    pub page: PageQuery,
}

#[get("/purchase-orders")]
pub async fn list_purchase_orders(
    state: web::Data<AppState>,
    user: AuthenticatedUser,
    query: web::Query<PurchaseQuery>,
) -> Result<HttpResponse, AppError> {
    user.ensure_role(&[
        Role::Admin,
        Role::Manager,
        Role::InventoryManager,
        Role::Supplier,
    ])?;

    // Destructure once so every field is usable after the move.
    let PurchaseQuery {
        status,
        supplier_id,
        store_id,
        page,
        ..
    } = query.into_inner();

    // Suppliers are scoped to their own vendor record.
    let supplier_view = matches!(user.role(), Ok(Role::Supplier));
    let forced_supplier = if supplier_view {
        user.supplier_id.clone().ok_or_else(|| {
            AppError::forbidden("supplier account is not linked to a vendor record")
        })?
    } else {
        supplier_id.unwrap_or_default()
    };

    if let Some(status) = &status
        && !["DRAFT", "SUBMITTED", "APPROVED", "RECEIVED", "CANCELLED"].contains(&status.as_str())
    {
        return Err(AppError::bad_request(format!("invalid status: {status}")));
    }

    let mut conditions: Vec<String> = Vec::new();
    let mut binds: Vec<String> = Vec::new();
    if let Some(status) = &status {
        conditions.push("po.status = ?".to_string());
        binds.push(status.clone());
    }
    if !forced_supplier.is_empty() {
        conditions.push("po.supplier_id = ?".to_string());
        binds.push(forced_supplier.clone());
    }
    if let Some(store_id) = &store_id {
        conditions.push("po.store_id = ?".to_string());
        binds.push(store_id.clone());
    }

    let where_clause = if conditions.is_empty() {
        String::new()
    } else {
        format!(" WHERE {}", conditions.join(" AND "))
    };

    let (page, per_page) = page.resolve(&state.config);
    let offset = (page - 1) * per_page;

    let count_sql =
        format!("SELECT CAST(COUNT(*) AS SIGNED) FROM `PURCHASE_ORDER` po{where_clause}");
    let mut count_query = sqlx::query_scalar::<_, i64>(sqlx::AssertSqlSafe(count_sql.clone()));
    for bind in &binds {
        count_query = count_query.bind(bind);
    }
    let total = count_query
        .fetch_one(&state.db)
        .await
        .map_err(AppError::db)?;

    let rows_sql = format!(
        "SELECT po.id, po.po_number, po.store_id, s.name AS store_name,
                po.supplier_id, sp.name AS supplier_name, po.status, po.total,
                po.expected_at, u.name AS created_by_name,
                (SELECT CAST(COUNT(*) AS SIGNED) FROM `PURCHASE_ORDER_ITEM` poi WHERE poi.purchase_order_id = po.id) AS item_count,
                po.created_at
         FROM `PURCHASE_ORDER` po
         JOIN `STORE` s ON s.id = po.store_id
         JOIN `SUPPLIER` sp ON sp.id = po.supplier_id
         JOIN `USER` u ON u.id = po.created_by
         {where_clause}
         ORDER BY po.created_at DESC LIMIT ? OFFSET ?"
    );

    let mut rows_query =
        sqlx::query_as::<_, PurchaseOrderRow>(sqlx::AssertSqlSafe(rows_sql.clone()));
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

/// Detail: header + items + delivery history.
#[get("/purchase-orders/{id}")]
pub async fn get_purchase_order(
    state: web::Data<AppState>,
    user: AuthenticatedUser,
    path: web::Path<String>,
) -> Result<HttpResponse, AppError> {
    user.ensure_role(&[
        Role::Admin,
        Role::Manager,
        Role::InventoryManager,
        Role::Supplier,
    ])?;
    let po_id = path.into_inner();

    let header = fetch_order_header(&state.db, &po_id).await?;
    if matches!(user.role(), Ok(Role::Supplier)) {
        let own = user
            .supplier_id
            .clone()
            .ok_or_else(|| AppError::forbidden("supplier account is not linked to a vendor"))?;
        if header.supplier_id != own {
            return Err(AppError::forbidden(
                "this purchase order belongs to another supplier",
            ));
        }
    }

    let items = fetch_order_items(&state.db, &po_id).await?;
    let deliveries = fetch_order_deliveries(&state.db, &po_id).await?;

    Ok(HttpResponse::Ok().json(PurchaseOrderDetail {
        order: header,
        items,
        deliveries,
    }))
}

#[post("/purchase-orders")]
pub async fn create(
    state: web::Data<AppState>,
    user: AuthenticatedUser,
    body: web::Json<CreatePurchaseOrderInput>,
) -> Result<HttpResponse, AppError> {
    user.ensure_role(&[Role::Admin, Role::Manager, Role::InventoryManager])?;
    let po_id = create_purchase_order(&state.db, &state, &user, &body.into_inner()).await?;
    let header = fetch_order_header(&state.db, &po_id).await?;
    let items = fetch_order_items(&state.db, &po_id).await?;
    Ok(HttpResponse::Created().json(PurchaseOrderDetail {
        order: header,
        items,
        deliveries: Vec::new(),
    }))
}

#[post("/purchase-orders/{id}/submit")]
pub async fn submit(
    state: web::Data<AppState>,
    user: AuthenticatedUser,
    path: web::Path<String>,
) -> Result<HttpResponse, AppError> {
    user.ensure_role(&[Role::Admin, Role::Manager, Role::InventoryManager])?;
    transition_purchase_order(&state.db, &user, &path.into_inner(), "submit").await?;
    Ok(HttpResponse::Ok().json(json!({ "status": "SUBMITTED" })))
}

#[post("/purchase-orders/{id}/approve")]
pub async fn approve(
    state: web::Data<AppState>,
    user: AuthenticatedUser,
    path: web::Path<String>,
) -> Result<HttpResponse, AppError> {
    user.ensure_role(&[Role::Admin, Role::Manager])?;
    transition_purchase_order(&state.db, &user, &path.into_inner(), "approve").await?;
    Ok(HttpResponse::Ok().json(json!({ "status": "APPROVED" })))
}

#[post("/purchase-orders/{id}/cancel")]
pub async fn cancel(
    state: web::Data<AppState>,
    user: AuthenticatedUser,
    path: web::Path<String>,
) -> Result<HttpResponse, AppError> {
    user.ensure_role(&[Role::Admin, Role::Manager, Role::InventoryManager])?;
    transition_purchase_order(&state.db, &user, &path.into_inner(), "cancel").await?;
    Ok(HttpResponse::Ok().json(json!({ "status": "CANCELLED" })))
}

#[post("/purchase-orders/{id}/deliveries")]
pub async fn receive(
    state: web::Data<AppState>,
    user: AuthenticatedUser,
    path: web::Path<String>,
    body: web::Json<ReceiveDeliveryInput>,
) -> Result<HttpResponse, AppError> {
    let po_id = path.into_inner();
    receive_delivery(&state.db, &state, &user, &po_id, &body.into_inner()).await?;

    let header = fetch_order_header(&state.db, &po_id).await?;
    let items = fetch_order_items(&state.db, &po_id).await?;
    let deliveries = fetch_order_deliveries(&state.db, &po_id).await?;
    Ok(HttpResponse::Ok().json(PurchaseOrderDetail {
        order: header,
        items,
        deliveries,
    }))
}
