//! Stock operations on `STOCK_BATCH` lots with FIFO consumption.
//!
//! Every function here must be called **inside a MySQL transaction**
//! (`&mut MySqlConnection` obtained by dereferencing an open
//! `sqlx::Transaction`), because `consume_batches` takes record locks
//! (`SELECT ... FOR UPDATE`) that are committed or rolled back as a unit.
//! This is the concurrency-control mechanism that prevents overselling the
//! last unit when multiple cashiers scan the same product simultaneously.

use rust_decimal::Decimal;
use sqlx::MySqlConnection;

use crate::app_state::AppState;
use crate::config::AppConfig;
use crate::error::AppError;
use crate::models::inventory::{AdjustmentInput, TransferInput};
use crate::models::notification::CreateNotification;
use crate::utils::{new_id, now};

use chrono::NaiveDate;

/// One consumed lot. Returned for audit/COGS extension by callers; today only
/// the weighted-average cost is consumed downstream.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct ConsumedBatch {
    pub batch_id: String,
    pub quantity: i64,
    pub unit_cost: Decimal,
}

/// Consume `quantity` units from the oldest partially-full batches (FIFO) of
/// `product_id` at `store_id` under row locks.
///
/// Returns the consumed lots plus the weighted-average unit cost. Fails with
/// `Conflict` when there is not enough total stock — the caller should roll
/// the containing transaction back so all taken locks are released.
pub async fn consume_batches(
    conn: &mut MySqlConnection,
    product_id: &str,
    store_id: &str,
    quantity: i64,
) -> Result<(Vec<ConsumedBatch>, Decimal), AppError> {
    if quantity <= 0 {
        return Err(AppError::bad_request("quantity must be positive"));
    }

    // Lock all live batches of the product in this store at once.
    let batches = sqlx::query_as::<_, (String, i32, Decimal)>(
        "SELECT id, quantity, unit_cost FROM `STOCK_BATCH`
         WHERE product_id = ? AND store_id = ? AND quantity > 0
         ORDER BY received_at, id FOR UPDATE",
    )
    .bind(product_id)
    .bind(store_id)
    .fetch_all(&mut *conn)
    .await
    .map_err(AppError::db)?;

    let available: i64 = batches
        .iter()
        .map(|(_, quantity, _)| *quantity as i64)
        .sum();
    if available < quantity {
        return Err(AppError::conflict(format!(
            "Insufficient stock: requested {quantity}, available {available}"
        )));
    }

    let mut remaining = quantity;
    let mut consumed = Vec::new();
    let mut total_cost = Decimal::ZERO;

    for (batch_id, batch_quantity, unit_cost) in batches {
        if remaining == 0 {
            break;
        }
        let take = i64::from(batch_quantity).min(remaining);

        sqlx::query("UPDATE `STOCK_BATCH` SET quantity = quantity - ? WHERE id = ?")
            .bind(take)
            .bind(&batch_id)
            .execute(&mut *conn)
            .await
            .map_err(AppError::db)?;

        total_cost += unit_cost * Decimal::from(take);
        remaining -= take;
        consumed.push(ConsumedBatch {
            batch_id,
            quantity: take,
            unit_cost,
        });
    }

    let average_unit_cost = total_cost / Decimal::from(quantity);
    Ok((consumed, average_unit_cost))
}

/// Create a fresh stock batch for a product in a store (receipt / return /
/// positive correction) and log a matching `STOCK_ADJUSTMENT` row.
#[allow(clippy::too_many_arguments)]
pub async fn add_batch(
    conn: &mut MySqlConnection,
    product_id: &str,
    store_id: &str,
    quantity: i64,
    unit_cost: Decimal,
    batch_code: Option<&str>,
    expires_at: Option<NaiveDate>,
    adjustment_type: &str,
    reason: Option<&str>,
    created_by: Option<&str>,
) -> Result<(), AppError> {
    if quantity <= 0 {
        return Err(AppError::bad_request("quantity must be positive"));
    }

    sqlx::query(
        "INSERT INTO `STOCK_BATCH`
         (id, product_id, store_id, quantity, unit_cost, batch_code, expires_at, received_at, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(new_id())
    .bind(product_id)
    .bind(store_id)
    .bind(quantity)
    .bind(unit_cost)
    .bind(batch_code)
    .bind(expires_at)
    .bind(now())
    .bind(now())
    .execute(&mut *conn)
    .await
    .map_err(AppError::db)?;

    log_inbound(
        &mut *conn,
        product_id,
        store_id,
        adjustment_type,
        quantity,
        unit_cost,
        reason,
        created_by,
    )
    .await
}

/// Log a positive (stock-in) `STOCK_ADJUSTMENT` row with a NULL batch link
/// (a single row may span several lots).
#[allow(clippy::too_many_arguments)]
pub async fn log_inbound(
    conn: &mut MySqlConnection,
    product_id: &str,
    store_id: &str,
    adjustment_type: &str,
    quantity: i64,
    unit_cost: Decimal,
    reason: Option<&str>,
    created_by: Option<&str>,
) -> Result<(), AppError> {
    sqlx::query(
        "INSERT INTO `STOCK_ADJUSTMENT`
         (id, product_id, store_id, batch_id, adjustment_type, quantity_delta, unit_cost, reason, created_by, created_at)
         VALUES (?, ?, ?, NULL, ?, ?, ?, ?, ?, ?)",
    )
    .bind(new_id())
    .bind(product_id)
    .bind(store_id)
    .bind(adjustment_type)
    .bind(quantity)
    .bind(unit_cost)
    .bind(reason)
    .bind(created_by)
    .bind(now())
    .execute(&mut *conn)
    .await
    .map_err(AppError::db)?;
    Ok(())
}

/// Log a negative (stock-out) `STOCK_ADJUSTMENT` row after batch consumption.
#[allow(clippy::too_many_arguments)]
pub async fn log_outbound(
    conn: &mut MySqlConnection,
    product_id: &str,
    store_id: &str,
    adjustment_type: &str,
    quantity: i64,
    unit_cost: Decimal,
    reason: Option<&str>,
    created_by: Option<&str>,
) -> Result<(), AppError> {
    if quantity <= 0 {
        return Err(AppError::bad_request("quantity must be positive"));
    }
    sqlx::query(
        "INSERT INTO `STOCK_ADJUSTMENT`
         (id, product_id, store_id, batch_id, adjustment_type, quantity_delta, unit_cost, reason, created_by, created_at)
         VALUES (?, ?, ?, NULL, ?, ?, ?, ?, ?, ?)",
    )
    .bind(new_id())
    .bind(product_id)
    .bind(store_id)
    .bind(adjustment_type)
    .bind(-quantity)
    .bind(unit_cost)
    .bind(reason)
    .bind(created_by)
    .bind(now())
    .execute(&mut *conn)
    .await
    .map_err(AppError::db)?;
    Ok(())
}

pub async fn insert_notification(
    conn: &mut MySqlConnection,
    notification: CreateNotification,
) -> Result<(), AppError> {
    sqlx::query(
        "INSERT INTO `NOTIFICATION`
         (id, type_name, title, message, audience, entity_type, entity_id, is_read, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, FALSE, ?)",
    )
    .bind(new_id())
    .bind(&notification.type_name)
    .bind(&notification.title)
    .bind(&notification.message)
    .bind(&notification.audience)
    .bind(&notification.entity_type)
    .bind(&notification.entity_id)
    .bind(now())
    .execute(&mut *conn)
    .await
    .map_err(AppError::db)?;
    Ok(())
}

/// Low-stock watcher: deduplicates alerts per product (one unread alert at a
/// time) and optionally auto-drafts a purchase order against the supplier most
/// recently used for that product. Must run inside the caller's transaction.
pub async fn check_low_stock(
    conn: &mut MySqlConnection,
    config: &AppConfig,
    product_id: &str,
    caused_by: &str,
) -> Result<(), AppError> {
    if !config.low_stock_alert_enabled {
        return Ok(());
    }

    let Some((name, reorder_threshold, _is_active)) = sqlx::query_as::<_, (String, i32, bool)>(
        "SELECT name, reorder_threshold, is_active FROM `PRODUCT` WHERE id = ?",
    )
    .bind(product_id)
    .fetch_optional(&mut *conn)
    .await
    .map_err(AppError::db)?
    else {
        return Ok(()); // product vanished meanwhile
    };

    let total_stock: i64 = sqlx::query_scalar(
        "SELECT CAST(COALESCE(SUM(quantity), 0) AS SIGNED) FROM `STOCK_BATCH` WHERE product_id = ?",
    )
    .bind(product_id)
    .fetch_one(&mut *conn)
    .await
    .map_err(AppError::db)?;

    if total_stock > i64::from(reorder_threshold) {
        return Ok(());
    }

    // One unread alert per product, ever.
    let has_unread: i64 = sqlx::query_scalar(
        "SELECT CAST(COUNT(*) AS SIGNED) FROM `NOTIFICATION`
         WHERE type_name = 'LOW_STOCK' AND entity_id = ? AND is_read = FALSE",
    )
    .bind(product_id)
    .fetch_one(&mut *conn)
    .await
    .map_err(AppError::db)?;

    if has_unread == 0 {
        insert_notification(
            &mut *conn,
            CreateNotification {
                type_name: "LOW_STOCK".to_string(),
                title: format!("Low stock: {name}"),
                message: format!(
                    "{name} has {total_stock} units left (threshold {reorder_threshold})."
                ),
                audience: "ALL".to_string(),
                entity_type: Some("PRODUCT".to_string()),
                entity_id: Some(product_id.to_string()),
            },
        )
        .await?;
    }

    if !config.auto_draft_po_enabled {
        return Ok(());
    }

    // Skip if this product is already on an open draft PO.
    let already_drafted: i64 = sqlx::query_scalar(
        "SELECT CAST(COUNT(*) AS SIGNED) FROM `PURCHASE_ORDER_ITEM` poi
         JOIN `PURCHASE_ORDER` po ON po.id = poi.purchase_order_id
         WHERE poi.product_id = ? AND po.status = 'DRAFT'",
    )
    .bind(product_id)
    .fetch_one(&mut *conn)
    .await
    .map_err(AppError::db)?;
    if already_drafted > 0 {
        return Ok(());
    }

    // Supplier: most recent PO for this product, else first active supplier.
    // Tuple: (supplier_id, store_id)
    let supplier_store = sqlx::query_as::<_, (String, String)>(
        "SELECT po.supplier_id, po.store_id
         FROM `PURCHASE_ORDER` po
         JOIN `PURCHASE_ORDER_ITEM` poi ON poi.purchase_order_id = po.id
         WHERE poi.product_id = ?
         ORDER BY po.created_at DESC
         LIMIT 1",
    )
    .bind(product_id)
    .fetch_optional(&mut *conn)
    .await
    .map_err(AppError::db)?;

    let (supplier_id, store_id) = match supplier_store {
        Some(pair) => pair,
        None => match sqlx::query_as::<_, (String, String)>(
            "SELECT s.id,
                    (SELECT st.id FROM `STORE` st ORDER BY st.created_at LIMIT 1)
             FROM `SUPPLIER` s WHERE s.is_active = TRUE ORDER BY s.created_at LIMIT 1",
        )
        .fetch_optional(&mut *conn)
        .await
        .map_err(AppError::db)?
        {
            Some(pair) => pair,
            None => return Ok(()), // no supplier registered yet
        },
    };

    let quantity = i64::from(reorder_threshold).max(1) * 2;
    let unit_cost: Decimal = sqlx::query_scalar("SELECT cost_price FROM `PRODUCT` WHERE id = ?")
        .bind(product_id)
        .fetch_one(&mut *conn)
        .await
        .map_err(AppError::db)?;
    let line_total = unit_cost * Decimal::from(quantity);
    let timestamp = now();

    // Append to an existing draft PO for this supplier+store, else create one.
    let existing = sqlx::query_as::<_, (String, String)>(
        "SELECT id, po_number FROM `PURCHASE_ORDER`
         WHERE supplier_id = ? AND store_id = ? AND status = 'DRAFT'
         ORDER BY created_at DESC LIMIT 1",
    )
    .bind(&supplier_id)
    .bind(&store_id)
    .fetch_optional(&mut *conn)
    .await
    .map_err(AppError::db)?;

    let (po_id, po_number, created_new) = match existing {
        Some((id, po_number)) => {
            sqlx::query("UPDATE `PURCHASE_ORDER` SET total = total + ? WHERE id = ?")
                .bind(line_total)
                .bind(&id)
                .execute(&mut *conn)
                .await
                .map_err(AppError::db)?;
            (id, po_number, false)
        }
        None => {
            let po_id = new_id();
            let po_number = crate::utils::document_number(&config.po_prefix);
            sqlx::query(
                "INSERT INTO `PURCHASE_ORDER`
                 (id, po_number, store_id, supplier_id, created_by, status, total, expected_at, notes, created_at, updated_at)
                 VALUES (?, ?, ?, ?, ?, 'DRAFT', ?, NULL, ?, ?, ?)",
            )
            .bind(&po_id)
            .bind(&po_number)
            .bind(&store_id)
            .bind(&supplier_id)
            .bind(caused_by)
            .bind(line_total)
            .bind("Auto-generated for low stock")
            .bind(timestamp)
            .bind(timestamp)
            .execute(&mut *conn)
            .await
            .map_err(AppError::db)?;
            (po_id, po_number, true)
        }
    };

    sqlx::query(
        "INSERT INTO `PURCHASE_ORDER_ITEM`
         (id, purchase_order_id, product_id, quantity_ordered, quantity_received, unit_cost, line_total)
         VALUES (?, ?, ?, ?, 0, ?, ?)",
    )
    .bind(new_id())
    .bind(&po_id)
    .bind(product_id)
    .bind(quantity)
    .bind(unit_cost)
    .bind(line_total)
    .execute(&mut *conn)
    .await
    .map_err(AppError::db)?;

    insert_notification(&mut *conn,
        CreateNotification {
            type_name: "PO_STATUS".to_string(),
            title: format!(
                "{} PO {po_number} for {name}",
                if created_new { "Draft created" } else { "Draft updated" }
            ),
            message: format!(
                "Product {name} is low on stock; purchase order {po_number} covers {quantity} units."
            ),
            audience: "ALL".to_string(),
            entity_type: Some("PURCHASE_ORDER".to_string()),
            entity_id: Some(po_id),
        },
    )
    .await?;

    Ok(())
}

/// Manual ops adjustment: shrinkage, damage, return, correction.
/// Negative delta consumes stock (FIFO); positive delta re-receives into a
/// fresh batch.
pub async fn apply_adjustment(
    conn: &mut MySqlConnection,
    app_state: &AppState,
    input: &AdjustmentInput,
    caused_by: &str,
) -> Result<(), AppError> {
    if input.delta == 0 {
        return Err(AppError::bad_request("delta must not be zero"));
    }

    if input.delta > 0 {
        let unit_cost: Decimal =
            sqlx::query_scalar("SELECT cost_price FROM `PRODUCT` WHERE id = ?")
                .bind(&input.product_id)
                .fetch_optional(&mut *conn)
                .await
                .map_err(AppError::db)?
                .ok_or_else(|| AppError::not_found("Product not found"))?;
        add_batch(
            &mut *conn,
            &input.product_id,
            &input.store_id,
            i64::from(input.delta),
            unit_cost,
            Some("ADJUSTMENT"),
            None,
            &input.adjustment_type,
            input.reason.as_deref(),
            Some(caused_by),
        )
        .await?;
    } else {
        consume_batches(
            &mut *conn,
            &input.product_id,
            &input.store_id,
            i64::from(-input.delta),
        )
        .await?;
        check_low_stock(&mut *conn, &app_state.config, &input.product_id, caused_by).await?;
    }

    Ok(())
}

/// Store-to-store stock transfer (TRANSFER_OUT at source, TRANSFER_IN at
/// destination), fully atomic in the caller's transaction.
pub async fn apply_transfer(
    conn: &mut MySqlConnection,
    app_state: &AppState,
    input: &TransferInput,
    caused_by: &str,
) -> Result<(), AppError> {
    if input.from_store_id == input.to_store_id {
        return Err(AppError::bad_request(
            "from_store_id and to_store_id must differ",
        ));
    }
    if input.quantity <= 0 {
        return Err(AppError::bad_request("quantity must be positive"));
    }

    consume_batches(
        &mut *conn,
        &input.product_id,
        &input.from_store_id,
        input.quantity,
    )
    .await?;

    log_outbound(
        &mut *conn,
        &input.product_id,
        &input.from_store_id,
        "TRANSFER_OUT",
        input.quantity,
        Decimal::ZERO,
        input.reason.as_deref(),
        Some(caused_by),
    )
    .await?;

    // One fresh destination batch summarising the transfer.
    sqlx::query(
        "INSERT INTO `STOCK_BATCH`
         (id, product_id, store_id, quantity, unit_cost, batch_code, received_at, created_at)
         VALUES (?, ?, ?, ?, ?, 'TRANSFER', ?, ?)",
    )
    .bind(new_id())
    .bind(&input.product_id)
    .bind(&input.to_store_id)
    .bind(input.quantity)
    .bind(Decimal::ZERO)
    .bind(now())
    .bind(now())
    .execute(&mut *conn)
    .await
    .map_err(AppError::db)?;

    log_inbound(
        &mut *conn,
        &input.product_id,
        &input.to_store_id,
        "TRANSFER_IN",
        input.quantity,
        Decimal::ZERO,
        input.reason.as_deref(),
        Some(caused_by),
    )
    .await?;

    check_low_stock(&mut *conn, &app_state.config, &input.product_id, caused_by).await?;
    Ok(())
}
