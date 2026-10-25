//! POS transactions: checkout, history, details, void/refund.

use actix_web::{HttpResponse, get, post, web};
use serde::Deserialize;

use crate::app_state::AppState;
use crate::auth::AuthenticatedUser;
use crate::error::AppError;
use crate::handlers::{PageQuery, PageResult};
use crate::models::sales::{
    CreateTransactionInput, PaymentRow, TransactionItemRow, TransactionRow, VoidInput,
};
use crate::models::user::Role;
use crate::services::sales::{create_transaction, restore_transaction_stock, transaction_detail};
use crate::services::stock::insert_notification;

#[post("/transactions")]
pub async fn create(
    state: web::Data<AppState>,
    user: AuthenticatedUser,
    body: web::Json<CreateTransactionInput>,
) -> Result<HttpResponse, AppError> {
    user.ensure_role(&[
        Role::Admin,
        Role::Manager,
        Role::InventoryManager,
        Role::Cashier,
    ])?;
    let input = body.into_inner();

    let transaction_id = create_transaction(&state.db, &state, &user, &input)
        .await
        .map_err(|error| {
            log::warn!("Transaction rejected for {}: {error}", user.email);
            error
        })?;

    let (header, items, payments) = transaction_detail(&state.db, &transaction_id).await?;
    Ok(HttpResponse::Created().json(TransactionDetail {
        header: header.serialize_wrapper(),
        items,
        payments,
    }))
}

use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct TransactionDetail {
    #[serde(flatten)]
    pub header: HeaderFields,
    pub items: Vec<TransactionItemRow>,
    pub payments: Vec<PaymentRow>,
}

#[derive(Debug, Serialize)]
pub struct HeaderFields {
    pub id: String,
    pub receipt_number: String,
    pub store_id: String,
    pub store_name: String,
    pub cashier_id: String,
    pub cashier_name: String,
    pub customer_id: Option<String>,
    pub customer_name: Option<String>,
    pub subtotal: rust_decimal::Decimal,
    pub discount_total: rust_decimal::Decimal,
    pub tax_total: rust_decimal::Decimal,
    pub grand_total: rust_decimal::Decimal,
    pub status: String,
    pub item_count: i64,
    pub created_at: chrono::NaiveDateTime,
}

impl TransactionRow {
    pub fn serialize_wrapper(&self) -> HeaderFields {
        HeaderFields {
            id: self.id.clone(),
            receipt_number: self.receipt_number.clone(),
            store_id: self.store_id.clone(),
            store_name: self.store_name.clone(),
            cashier_id: self.cashier_id.clone(),
            cashier_name: self.cashier_name.clone(),
            customer_id: self.customer_id.clone(),
            customer_name: self.customer_name.clone(),
            subtotal: self.subtotal,
            discount_total: self.discount_total,
            tax_total: self.tax_total,
            grand_total: self.grand_total,
            status: self.status.clone(),
            item_count: self.item_count,
            created_at: self.created_at,
        }
    }
}

#[derive(Debug, Deserialize, Default)]
pub struct TransactionQuery {
    /// Plain calendar dates (YYYY-MM-DD from the UI date inputs); expanded to
    /// full-day bounds server-side.
    pub from: Option<chrono::NaiveDate>,
    pub to: Option<chrono::NaiveDate>,
    pub store_id: Option<String>,
    pub cashier_id: Option<String>,
    pub customer_id: Option<String>,
    pub status: Option<String>,
    pub search: Option<String>,
    #[serde(flatten)]
    pub page: PageQuery,
}

#[get("/transactions")]
pub async fn list_transactions(
    state: web::Data<AppState>,
    user: AuthenticatedUser,
    query: web::Query<TransactionQuery>,
) -> Result<HttpResponse, AppError> {
    user.ensure_role(&[
        Role::Admin,
        Role::Manager,
        Role::InventoryManager,
        Role::Cashier,
    ])?;
    let (page, per_page) = query.page.resolve(&state.config);
    let offset = query.page.offset(&state.config);
    let TransactionQuery {
        from,
        to,
        store_id,
        cashier_id,
        customer_id,
        status,
        search,
        ..
    } = query.into_inner();

    if let Some(status) = &status
        && !["COMPLETED", "VOIDED", "REFUNDED"].contains(&status.as_str())
    {
        return Err(AppError::bad_request(format!("invalid status: {status}")));
    }

    // Suppliers may only ever see staff transactions through staff endpoints.
    if matches!(user.role(), Ok(Role::Supplier)) {
        return Err(AppError::forbidden("suppliers cannot browse transactions"));
    }

    let mut conditions: Vec<String> = Vec::new();
    let mut binds: Vec<String> = Vec::new();
    if let Some(from) = from {
        conditions.push("t.created_at >= ?".to_string());
        binds.push(from.and_hms_opt(0, 0, 0).unwrap().to_string());
    }
    if let Some(to) = to {
        conditions.push("t.created_at <= ?".to_string());
        binds.push(to.and_hms_opt(23, 59, 59).unwrap().to_string());
    }
    if let Some(store_id) = &store_id {
        conditions.push("t.store_id = ?".to_string());
        binds.push(store_id.clone());
    }
    if let Some(cashier_id) = &cashier_id {
        conditions.push("t.cashier_id = ?".to_string());
        binds.push(cashier_id.clone());
    }
    if let Some(customer_id) = &customer_id {
        conditions.push("t.customer_id = ?".to_string());
        binds.push(customer_id.clone());
    }
    if let Some(status) = &status {
        conditions.push("t.status = ?".to_string());
        binds.push(status.clone());
    }
    if let Some(search) = &search {
        conditions.push("t.receipt_number LIKE ?".to_string());
        binds.push(format!("%{}%", search.trim()));
    }

    let where_clause = if conditions.is_empty() {
        String::new()
    } else {
        format!(" WHERE {}", conditions.join(" AND "))
    };

    let count_sql = format!("SELECT CAST(COUNT(*) AS SIGNED) FROM `TRANSACTION` t{where_clause}");
    let mut count_query = sqlx::query_scalar::<_, i64>(sqlx::AssertSqlSafe(count_sql.clone()));
    for bind in &binds {
        count_query = count_query.bind(bind);
    }
    let total = count_query
        .fetch_one(&state.db)
        .await
        .map_err(AppError::db)?;

    let rows_sql = format!(
        "SELECT t.id, t.receipt_number, t.store_id, s.name AS store_name,
                t.cashier_id, u.name AS cashier_name,
                t.customer_id, c.name AS customer_name,
                t.subtotal, t.discount_total, t.tax_total, t.grand_total, t.status,
                (SELECT CAST(COUNT(*) AS SIGNED) FROM `TRANSACTION_ITEM` ti WHERE ti.transaction_id = t.id) AS item_count,
                t.created_at
         FROM `TRANSACTION` t
         JOIN `STORE` s ON s.id = t.store_id
         JOIN `USER` u ON u.id = t.cashier_id
         LEFT JOIN `CUSTOMER` c ON c.id = t.customer_id
         {where_clause}
         ORDER BY t.created_at DESC LIMIT ? OFFSET ?"
    );

    let mut rows_query = sqlx::query_as::<_, TransactionRow>(sqlx::AssertSqlSafe(rows_sql.clone()));
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

#[get("/transactions/{id}")]
pub async fn get_transaction(
    state: web::Data<AppState>,
    user: AuthenticatedUser,
    path: web::Path<String>,
) -> Result<HttpResponse, AppError> {
    user.ensure_role(&[
        Role::Admin,
        Role::Manager,
        Role::InventoryManager,
        Role::Cashier,
    ])?;
    let (header, items, payments) = transaction_detail(&state.db, &path.into_inner()).await?;

    Ok(HttpResponse::Ok().json(TransactionDetail {
        header: header.serialize_wrapper(),
        items,
        payments,
    }))
}

/// Void a completed transaction: restores stock, reverses loyalty points.
async fn reverse_transaction(
    state: web::Data<AppState>,
    user: AuthenticatedUser,
    transaction_id: String,
    action: &str,
    body: Option<VoidInput>,
) -> Result<HttpResponse, AppError> {
    user.ensure_role(&[Role::Admin, Role::Manager])?;
    let label = match action {
        "void" => "Void",
        _ => "Refund",
    };
    let status = match action {
        "void" => "VOIDED",
        _ => "REFUNDED",
    };

    let mut tx = state.db.begin().await.map_err(AppError::db)?;

    let current: Option<String> =
        sqlx::query_scalar("SELECT status FROM `TRANSACTION` WHERE id = ? FOR UPDATE")
            .bind(&transaction_id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(AppError::db)?;
    let Some(current) = current else {
        return Err(AppError::not_found("Transaction not found"));
    };
    if current != "COMPLETED" {
        return Err(AppError::conflict(format!(
            "only COMPLETED transactions can be {status:?} (current: {current})"
        )));
    }

    restore_transaction_stock(&mut tx, &state, &transaction_id, label, &user.user_id).await?;

    sqlx::query("UPDATE `TRANSACTION` SET status = ?, voided_by = ?, void_reason = ? WHERE id = ?")
        .bind(status)
        .bind(&user.user_id)
        .bind(body.and_then(|input| input.reason))
        .bind(&transaction_id)
        .execute(&mut *tx)
        .await
        .map_err(AppError::db)?;

    insert_notification(
        &mut tx,
        crate::models::notification::CreateNotification {
            type_name: "SYSTEM".to_string(),
            title: format!("{label} recorded"),
            message: format!(
                "Transaction {transaction_id} was {action}ed by {}.",
                user.name
            ),
            audience: "ALL".to_string(),
            entity_type: Some("TRANSACTION".to_string()),
            entity_id: Some(transaction_id.clone()),
        },
    )
    .await?;

    tx.commit().await.map_err(AppError::db)?;

    let (header, items, payments) = transaction_detail(&state.db, &transaction_id).await?;
    Ok(HttpResponse::Ok().json(TransactionDetail {
        header: header.serialize_wrapper(),
        items,
        payments,
    }))
}

#[post("/transactions/{id}/void")]
pub async fn void_transaction(
    state: web::Data<AppState>,
    user: AuthenticatedUser,
    path: web::Path<String>,
    body: web::Json<VoidInput>,
) -> Result<HttpResponse, AppError> {
    reverse_transaction(
        state,
        user,
        path.into_inner(),
        "void",
        Some(body.into_inner()),
    )
    .await
}

#[post("/transactions/{id}/refund")]
pub async fn refund_transaction(
    state: web::Data<AppState>,
    user: AuthenticatedUser,
    path: web::Path<String>,
    body: web::Json<VoidInput>,
) -> Result<HttpResponse, AppError> {
    reverse_transaction(
        state,
        user,
        path.into_inner(),
        "refund",
        Some(body.into_inner()),
    )
    .await
}
