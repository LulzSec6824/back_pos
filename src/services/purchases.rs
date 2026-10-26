//! Purchase order lifecycle: draft -> submitted -> approved -> received.
//!
//! Receiving a delivery is fully atomic: it creates `STOCK_BATCH` lots,
//! writes `PURCHASE_ORDER_ITEM` progress, `SUPPLIER_DELIVERY`, and
//! `STOCK_ADJUSTMENT` (PURCHASE) audit rows, updates the product cost price
//! with a `PRODUCT_PRICE_HISTORY` entry, and flips the PO status to
//! `RECEIVED` when everything has arrived.

use rust_decimal::Decimal;
use sqlx::{MySql, MySqlConnection};

use crate::app_state::AppState;
use crate::auth::AuthenticatedUser;
use crate::error::AppError;
use crate::models::purchases::{
    CreatePurchaseOrderInput, DeliveryRow, PurchaseOrderItemRow, PurchaseOrderRow,
    ReceiveDeliveryInput,
};
use crate::services::stock::{add_batch, insert_notification};
use crate::utils::{document_number, new_id, now, round2};

async fn resolve_default_store(
    conn: &mut MySqlConnection,
    user: &AuthenticatedUser,
) -> Result<String, AppError> {
    // Home store of the acting user, else bootstrap store, else first store.
    let home: Option<String> =
        sqlx::query_scalar::<_, Option<String>>("SELECT store_id FROM `USER` WHERE id = ?")
            .bind(&user.user_id)
            .fetch_one(&mut *conn)
            .await
            .map_err(AppError::db)?;
    if let Some(home) = home {
        return Ok(home);
    }

    sqlx::query_scalar::<_, String>("SELECT id FROM `STORE` ORDER BY created_at LIMIT 1")
        .fetch_optional(&mut *conn)
        .await
        .map_err(AppError::db)?
        .ok_or_else(|| AppError::internal("No store configured"))
}

/// Create a `DRAFT` purchase order. Returns its id.
pub async fn create_purchase_order(
    pool: &sqlx::Pool<MySql>,
    app_state: &AppState,
    user: &AuthenticatedUser,
    input: &CreatePurchaseOrderInput,
) -> Result<String, AppError> {
    if input.items.is_empty() {
        return Err(AppError::bad_request("Purchase order must contain items"));
    }

    let mut tx = pool.begin().await.map_err(AppError::db)?;

    let store_id = match &input.store_id {
        Some(store_id) => {
            // Validate now so an unknown id is a 404 instead of an FK
            // violation when the PO row is inserted below.
            let store =
                sqlx::query_scalar::<_, i64>("SELECT CAST(1 AS SIGNED) FROM `STORE` WHERE id = ?")
                    .bind(store_id)
                    .fetch_optional(&mut *tx)
                    .await
                    .map_err(AppError::db)?;
            if store.is_none() {
                return Err(AppError::not_found(format!("Store {store_id} not found")));
            }
            store_id.clone()
        }
        None => resolve_default_store(&mut tx, user).await?,
    };

    let supplier: Option<i64> = sqlx::query_scalar(
        "SELECT CAST(1 AS SIGNED) FROM `SUPPLIER` WHERE id = ? AND is_active = TRUE",
    )
    .bind(&input.supplier_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(AppError::db)?;
    if supplier.is_none() {
        return Err(AppError::not_found("Supplier not found or inactive"));
    }

    let po_id = new_id();
    let po_number = document_number(&app_state.config.po_prefix);
    let timestamp = now();
    let mut total = Decimal::ZERO;

    sqlx::query(
        "INSERT INTO `PURCHASE_ORDER`
         (id, po_number, store_id, supplier_id, created_by, status, total, expected_at, notes, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, 'DRAFT', ?, ?, ?, ?, ?)",
    )
    .bind(&po_id)
    .bind(&po_number)
    .bind(&store_id)
    .bind(&input.supplier_id)
    .bind(&user.user_id)
    .bind(Decimal::ZERO)
    .bind(input.expected_at)
    .bind(&input.notes)
    .bind(timestamp)
    .bind(timestamp)
    .execute(&mut *tx)
    .await
    .map_err(AppError::db)?;

    for item in &input.items {
        if item.quantity <= 0 {
            return Err(AppError::bad_request("Item quantity must be positive"));
        }

        let product = sqlx::query_as::<_, (Decimal, Option<String>)>(
            "SELECT cost_price, name FROM `PRODUCT` WHERE id = ?",
        )
        .bind(&item.product_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(AppError::db)?
        .ok_or_else(|| AppError::not_found(format!("Product {} not found", item.product_id)))?;

        let unit_cost = round2(item.unit_cost.unwrap_or(product.0));
        if unit_cost < Decimal::ZERO {
            return Err(AppError::bad_request("unit_cost must not be negative"));
        }
        let line_total = round2(unit_cost * Decimal::from(item.quantity));
        total += line_total;

        sqlx::query(
            "INSERT INTO `PURCHASE_ORDER_ITEM`
             (id, purchase_order_id, product_id, quantity_ordered, quantity_received, unit_cost, line_total)
             VALUES (?, ?, ?, ?, 0, ?, ?)",
        )
        .bind(new_id())
        .bind(&po_id)
        .bind(&item.product_id)
        .bind(item.quantity)
        .bind(unit_cost)
        .bind(line_total)
        .execute(&mut *tx)
        .await
        .map_err(AppError::db)?;
    }

    sqlx::query("UPDATE `PURCHASE_ORDER` SET total = ? WHERE id = ?")
        .bind(round2(total))
        .bind(&po_id)
        .execute(&mut *tx)
        .await
        .map_err(AppError::db)?;

    insert_notification(
        &mut tx,
        crate::models::notification::CreateNotification {
            type_name: "PO_STATUS".to_string(),
            title: format!("Purchase order {po_number} created"),
            message: format!("Purchase order {po_number} was drafted and awaits submission."),
            audience: "ALL".to_string(),
            entity_type: Some("PURCHASE_ORDER".to_string()),
            entity_id: Some(po_id.clone()),
        },
    )
    .await?;

    tx.commit().await.map_err(AppError::db)?;
    Ok(po_id)
}

/// Status transition: `action` ∈ {"submit", "approve", "cancel"}.
pub async fn transition_purchase_order(
    pool: &sqlx::Pool<MySql>,
    user: &AuthenticatedUser,
    po_id: &str,
    action: &str,
) -> Result<(), AppError> {
    let mut tx = pool.begin().await.map_err(AppError::db)?;

    let current: Option<String> =
        sqlx::query_scalar("SELECT status FROM `PURCHASE_ORDER` WHERE id = ? FOR UPDATE")
            .bind(po_id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(AppError::db)?;
    let Some(current) = current else {
        return Err(AppError::not_found("Purchase order not found"));
    };

    let next = match (action, current.as_str()) {
        ("submit", "DRAFT") => "SUBMITTED",
        ("approve", "SUBMITTED") => {
            user.ensure_role(&[
                crate::models::user::Role::Admin,
                crate::models::user::Role::Manager,
            ])?;
            "APPROVED"
        }
        ("cancel", "DRAFT" | "SUBMITTED" | "APPROVED") => "CANCELLED",
        _ => {
            return Err(AppError::conflict(format!(
                "cannot {action} a purchase order in status {current}"
            )));
        }
    };

    sqlx::query("UPDATE `PURCHASE_ORDER` SET status = ? WHERE id = ?")
        .bind(next)
        .bind(po_id)
        .execute(&mut *tx)
        .await
        .map_err(AppError::db)?;

    if next != "DRAFT" {
        let po_number: String =
            sqlx::query_scalar("SELECT po_number FROM `PURCHASE_ORDER` WHERE id = ?")
                .bind(po_id)
                .fetch_one(&mut *tx)
                .await
                .map_err(AppError::db)?;
        // Verbs match the new status; appending "-ed" to the action would
        // produce misspellings like "approveed"/"submited"/"canceled".
        let verb = match next {
            "SUBMITTED" => "submitted",
            "APPROVED" => "approved",
            "CANCELLED" => "cancelled",
            _ => action,
        };
        crate::services::stock::insert_notification(
            &mut tx,
            crate::models::notification::CreateNotification {
                type_name: "PO_STATUS".to_string(),
                title: format!("Purchase order {po_number} {verb}"),
                message: format!("Purchase order {po_number} moved from {current} to {next}."),
                audience: "ALL".to_string(),
                entity_type: Some("PURCHASE_ORDER".to_string()),
                entity_id: Some(po_id.to_string()),
            },
        )
        .await?;
    }

    tx.commit().await.map_err(AppError::db)?;
    Ok(())
}

/// Receive goods against an approved purchase order.
pub async fn receive_delivery(
    pool: &sqlx::Pool<MySql>,
    _app_state: &AppState,
    user: &AuthenticatedUser,
    po_id: &str,
    input: &ReceiveDeliveryInput,
) -> Result<(), AppError> {
    user.ensure_role(&[
        crate::models::user::Role::Admin,
        crate::models::user::Role::Manager,
        crate::models::user::Role::InventoryManager,
    ])?;

    let mut tx = pool.begin().await.map_err(AppError::db)?;

    let Some((status, store_id, po_number)) = sqlx::query_as::<_, (String, String, String)>(
        "SELECT status, store_id, po_number FROM `PURCHASE_ORDER` WHERE id = ? FOR UPDATE",
    )
    .bind(po_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(AppError::db)?
    else {
        return Err(AppError::not_found("Purchase order not found"));
    };
    if status != "APPROVED" {
        return Err(AppError::conflict(format!(
            "deliveries can only be received for APPROVED orders (current: {status})"
        )));
    }

    // Outstanding quantities per PO item: item_id -> (product_id, remaining, unit_cost).
    let rows = sqlx::query_as::<_, (String, String, i32, i32, Decimal)>(
        "SELECT id, product_id, quantity_ordered, quantity_received, unit_cost
         FROM `PURCHASE_ORDER_ITEM` WHERE purchase_order_id = ?",
    )
    .bind(po_id)
    .fetch_all(&mut *tx)
    .await
    .map_err(AppError::db)?;

    let mut remaining: std::collections::BTreeMap<String, (String, i64, Decimal)> = rows
        .into_iter()
        .map(|(item_id, product_id, ordered, received, unit_cost)| {
            (
                item_id,
                (product_id, i64::from(ordered - received), unit_cost),
            )
        })
        .collect();

    let timestamp = now();
    let mut received_any = false;
    let mut delivery_codes: Vec<String> = Vec::new();

    for group in &input.deliveries {
        let delivery_code = group
            .delivery_code
            .clone()
            .unwrap_or_else(|| document_number("DLV"));

        sqlx::query(
            "INSERT INTO `SUPPLIER_DELIVERY`
             (id, purchase_order_id, delivery_code, received_by, notes, received_at)
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(new_id())
        .bind(po_id)
        .bind(&delivery_code)
        .bind(&user.user_id)
        .bind(&group.notes)
        .bind(timestamp)
        .execute(&mut *tx)
        .await
        .map_err(AppError::db)?;

        // Resolve this group's receive list (empty list = receive all outstanding).
        let group_items: Vec<(String, i64)> = if group.items.is_empty() {
            remaining
                .iter()
                .filter(|(_, (_, rem, _))| *rem > 0)
                .map(|(item_id, (_, rem, _))| (item_id.clone(), *rem))
                .collect()
        } else {
            group
                .items
                .iter()
                .filter(|item| item.quantity_received > 0)
                .map(|item| (item.purchase_order_item_id.clone(), item.quantity_received))
                .collect()
        };

        for (item_id, quantity) in group_items {
            let Some((product_id, outstanding, unit_cost)) = remaining.get(&item_id).cloned()
            else {
                return Err(AppError::bad_request(format!(
                    "item {item_id} does not belong to this purchase order"
                )));
            };
            if outstanding <= 0 {
                return Err(AppError::bad_request(format!(
                    "item {item_id} is already fully received"
                )));
            }
            if quantity > outstanding {
                return Err(AppError::bad_request(format!(
                    "cannot receive {quantity} units of {product_id}: only {outstanding} outstanding"
                )));
            }

            add_batch(
                &mut tx,
                &product_id,
                &store_id,
                quantity,
                unit_cost,
                Some(&delivery_code),
                group.expires_at,
                "PURCHASE",
                Some(&format!("Delivery {delivery_code} for PO {po_number}")),
                Some(&user.user_id),
            )
            .await?;

            // Cost price refresh + history (historical data management).
            let (old_cost, old_selling): (Decimal, Decimal) = sqlx::query_as(
                "SELECT cost_price, selling_price FROM `PRODUCT` WHERE id = ? FOR UPDATE",
            )
            .bind(&product_id)
            .fetch_one(&mut *tx)
            .await
            .map_err(AppError::db)?;

            sqlx::query(
                "UPDATE `PURCHASE_ORDER_ITEM` SET quantity_received = quantity_received + ? WHERE id = ?",
            )
            .bind(quantity)
            .bind(&item_id)
            .execute(&mut *tx)
            .await
            .map_err(AppError::db)?;

            if unit_cost != old_cost {
                sqlx::query("UPDATE `PRODUCT` SET cost_price = ? WHERE id = ?")
                    .bind(unit_cost)
                    .bind(&product_id)
                    .execute(&mut *tx)
                    .await
                    .map_err(AppError::db)?;

                sqlx::query(
                    "INSERT INTO `PRODUCT_PRICE_HISTORY`
                     (id, product_id, old_cost_price, old_selling_price, new_cost_price, new_selling_price, changed_by, changed_at)
                     VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
                )
                .bind(new_id())
                .bind(&product_id)
                .bind(old_cost)
                .bind(old_selling)
                .bind(unit_cost)
                .bind(old_selling)
                .bind(&user.user_id)
                .bind(timestamp)
                .execute(&mut *tx)
                .await
                .map_err(AppError::db)?;
            }

            remaining.insert(item_id, (product_id, outstanding - quantity, unit_cost));
            received_any = true;
        }

        delivery_codes.push(delivery_code);
    }

    if !received_any {
        return Err(AppError::bad_request(
            "nothing was received (check quantities and outstanding items)",
        ));
    }

    // Auto-complete the PO when fully received.
    let unfinished: i64 = sqlx::query_scalar(
        "SELECT CAST(COUNT(*) AS SIGNED) FROM `PURCHASE_ORDER_ITEM`
         WHERE purchase_order_id = ? AND quantity_received < quantity_ordered",
    )
    .bind(po_id)
    .fetch_one(&mut *tx)
    .await
    .map_err(AppError::db)?;

    let final_status = if unfinished == 0 {
        "RECEIVED"
    } else {
        "APPROVED"
    };
    sqlx::query("UPDATE `PURCHASE_ORDER` SET status = ? WHERE id = ?")
        .bind(final_status)
        .bind(po_id)
        .execute(&mut *tx)
        .await
        .map_err(AppError::db)?;

    insert_notification(
        &mut tx,
        crate::models::notification::CreateNotification {
            type_name: "DELIVERY".to_string(),
            title: format!("Delivery received for PO {po_number}"),
            message: format!(
                "Delivery(s) {} were received for purchase order {po_number}; stock updated.",
                delivery_codes.join(", ")
            ),
            audience: "ALL".to_string(),
            entity_type: Some("SUPPLIER_DELIVERY".to_string()),
            entity_id: Some(po_id.to_string()),
        },
    )
    .await?;

    tx.commit().await.map_err(AppError::db)?;
    Ok(())
}

/// List helpers reused by handlers.
pub async fn fetch_order_items(
    pool: &sqlx::Pool<MySql>,
    po_id: &str,
) -> Result<Vec<PurchaseOrderItemRow>, AppError> {
    sqlx::query_as::<_, PurchaseOrderItemRow>(
        "SELECT poi.id, poi.purchase_order_id, poi.product_id, p.name AS product_name, p.sku,
                poi.quantity_ordered, poi.quantity_received, poi.unit_cost, poi.line_total
         FROM `PURCHASE_ORDER_ITEM` poi
         JOIN `PRODUCT` p ON p.id = poi.product_id
         WHERE poi.purchase_order_id = ?
         ORDER BY p.name",
    )
    .bind(po_id)
    .fetch_all(pool)
    .await
    .map_err(AppError::db)
}

pub async fn fetch_order_deliveries(
    pool: &sqlx::Pool<MySql>,
    po_id: &str,
) -> Result<Vec<DeliveryRow>, AppError> {
    sqlx::query_as::<_, DeliveryRow>(
        "SELECT d.id, d.purchase_order_id, d.delivery_code, u.name AS received_by_name, d.notes, d.received_at
         FROM `SUPPLIER_DELIVERY` d
         JOIN `USER` u ON u.id = d.received_by
         WHERE d.purchase_order_id = ?
         ORDER BY d.received_at DESC",
    )
    .bind(po_id)
    .fetch_all(pool)
    .await
    .map_err(AppError::db)
}

/// Combined PO header fetch (with joins) used by detail endpoints.
pub async fn fetch_order_header(
    pool: &sqlx::Pool<MySql>,
    po_id: &str,
) -> Result<PurchaseOrderRow, AppError> {
    sqlx::query_as::<_, PurchaseOrderRow>(
        "SELECT po.id, po.po_number, po.store_id, s.name AS store_name,
                po.supplier_id, sp.name AS supplier_name, po.status, po.total,
                po.expected_at, u.name AS created_by_name,
                (SELECT CAST(COUNT(*) AS SIGNED) FROM `PURCHASE_ORDER_ITEM` poi WHERE poi.purchase_order_id = po.id) AS item_count,
                po.created_at
         FROM `PURCHASE_ORDER` po
         JOIN `STORE` s ON s.id = po.store_id
         JOIN `SUPPLIER` sp ON sp.id = po.supplier_id
         JOIN `USER` u ON u.id = po.created_by
         WHERE po.id = ?",
    )
    .bind(po_id)
    .fetch_optional(pool)
    .await
    .map_err(AppError::db)?
    .ok_or_else(|| AppError::not_found("Purchase order not found"))
}
