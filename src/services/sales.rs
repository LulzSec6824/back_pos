//! Transaction (sale) orchestration.
//!
//! `create_transaction` is the heart of the POS: it
//! 1. locks every involved `PRODUCT` row (`SELECT ... FOR UPDATE`),
//! 2. consumes stock from `STOCK_BATCH` lots FIFO under those locks,
//! 3. computes per-line discounts and taxes,
//! 4. inserts the `TRANSACTION`, `TRANSACTION_ITEM` and `PAYMENT` rows,
//! 5. credits loyalty points with a `CUSTOMER_LOYALTY_LOG` entry,
//! 6. runs the low-stock watcher (alerts + auto-draft POs).
//!
//! All of the above shares ONE MySQL transaction — any failure rolls the
//! whole checkout back, guaranteeing COMMIT/ROLLBACK semantics.

use std::collections::BTreeMap;

use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;
use sqlx::MySql;

use crate::app_state::AppState;
use crate::auth::AuthenticatedUser;
use crate::error::AppError;
use crate::models::sales::{
    CreateTransactionInput, PaymentRow, TransactionItemRow, TransactionRow,
};
use crate::services::stock::{check_low_stock, consume_batches};
use crate::utils::{new_id, now, round2};

pub const PAYMENT_METHODS: [&str; 3] = ["CASH", "CARD", "DIGITAL_WALLET"];

fn parse_payment_method(method: &str) -> Result<&'static str, AppError> {
    PAYMENT_METHODS
        .iter()
        .find(|candidate| candidate.eq_ignore_ascii_case(method))
        .copied()
        .ok_or_else(|| {
            AppError::bad_request(format!("payment method must be one of {PAYMENT_METHODS:?}"))
        })
}

/// Default store resolution order: request override -> user's home store ->
/// first store created (the bootstrap HQ).
pub async fn resolve_store_id(
    conn: &mut sqlx::MySqlConnection,
    app_state: &AppState,
    requested: Option<&str>,
    user_home: Option<&str>,
) -> Result<String, AppError> {
    if let Some(store_id) = requested {
        let exists: Option<i64> =
            sqlx::query_scalar("SELECT CAST(1 AS SIGNED) FROM `STORE` WHERE id = ?")
                .bind(store_id)
                .fetch_optional(&mut *conn)
                .await
                .map_err(AppError::db)?;
        if exists.is_none() {
            return Err(AppError::not_found("Store not found"));
        }
        return Ok(store_id.to_string());
    }

    if let Some(store_id) = user_home {
        let exists: Option<i64> =
            sqlx::query_scalar("SELECT CAST(1 AS SIGNED) FROM `STORE` WHERE id = ?")
                .bind(store_id)
                .fetch_optional(&mut *conn)
                .await
                .map_err(AppError::db)?;
        if exists.is_some() {
            return Ok(store_id.to_string());
        }
    }

    sqlx::query_scalar::<_, String>(
        "SELECT id FROM `STORE` WHERE code = ? OR is_online = FALSE ORDER BY created_at LIMIT 1",
    )
    .bind(&app_state.config.store_code)
    .fetch_optional(&mut *conn)
    .await
    .map_err(AppError::db)?
    .ok_or_else(|| AppError::internal("No store is configured; check bootstrap"))
}

/// Create a sale. Returns the new `TRANSACTION` id.
pub async fn create_transaction(
    pool: &sqlx::Pool<MySql>,
    app_state: &AppState,
    user: &AuthenticatedUser,
    input: &CreateTransactionInput,
) -> Result<String, AppError> {
    if input.items.is_empty() {
        return Err(AppError::bad_request("Transaction must contain items"));
    }

    // Merge duplicate lines per product.
    let mut quantities: BTreeMap<String, (i64, Decimal)> = BTreeMap::new();
    for item in &input.items {
        if item.quantity <= 0 {
            return Err(AppError::bad_request("Item quantity must be positive"));
        }
        let discount = item.line_discount.unwrap_or(Decimal::ZERO);
        let entry = quantities
            .entry(item.product_id.clone())
            .or_insert((0, Decimal::ZERO));
        entry.0 += item.quantity;
        entry.1 += discount;
    }

    let mut tx = pool.begin().await.map_err(AppError::db)?;

    // Cashier's home store, when set.
    let home_store: Option<String> =
        sqlx::query_scalar::<_, Option<String>>("SELECT store_id FROM `USER` WHERE id = ?")
            .bind(&user.user_id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(AppError::db)?
            .flatten();

    let store_id = resolve_store_id(
        &mut tx,
        app_state,
        input.store_id.as_deref(),
        home_store.as_deref(),
    )
    .await?;

    struct Line {
        product_id: String,
        quantity: i64,
        unit_price: Decimal,
        cost_price: Decimal,
        line_discount: Decimal,
        tax_rate: Decimal,
        line_total: Decimal,
    }

    let mut lines = Vec::with_capacity(quantities.len());
    let mut subtotal = Decimal::ZERO;
    let mut discount_total = Decimal::ZERO;
    let mut tax_total = Decimal::ZERO;

    for (product_id, (quantity, requested_discount)) in &quantities {
        // Lock the product row: snapshots price + blocks price changes mid-sale.
        let product =
            sqlx::query_as::<_, (String, String, Decimal, Decimal, Option<Decimal>, bool)>(
                "SELECT name, sku, selling_price, cost_price, tax_rate_override, is_active
             FROM `PRODUCT` WHERE id = ? FOR UPDATE",
            )
            .bind(product_id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(AppError::db)?
            .ok_or_else(|| AppError::not_found(format!("Product {product_id} not found")))?;

        if !product.5 {
            return Err(AppError::bad_request(format!(
                "Product {} is inactive",
                product.1
            )));
        }

        let (consumed, average_cost) =
            consume_batches(&mut tx, product_id, &store_id, *quantity).await?;
        let _ = consumed; // lots are audited via TRANSACTION_ITEM

        let unit_price = product.2;
        let line_base = round2(unit_price * Decimal::from(*quantity));
        let line_discount = round2((*requested_discount).clamp(Decimal::ZERO, line_base));
        let line_total = round2(line_base - line_discount);
        let tax_rate = product.4.unwrap_or(app_state.config.default_tax_rate);
        let tax_amount = round2(line_total * tax_rate / Decimal::from(100));

        subtotal += round2(line_base);
        discount_total += line_discount;
        tax_total += tax_amount;

        lines.push(Line {
            product_id: product_id.clone(),
            quantity: *quantity,
            unit_price,
            cost_price: average_cost,
            line_discount,
            tax_rate,
            line_total,
        });
    }

    let subtotal = round2(subtotal);
    let discount_total = round2(discount_total);
    let tax_total = round2(tax_total);
    let grand_total = round2(subtotal - discount_total + tax_total);

    let transaction_id = new_id();
    let receipt_number = crate::utils::document_number(&app_state.config.receipt_prefix);
    let timestamp = now();

    sqlx::query(
        "INSERT INTO `TRANSACTION`
         (id, receipt_number, store_id, cashier_id, customer_id, subtotal, discount_total, tax_total, grand_total, status, voided_by, void_reason, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, 'COMPLETED', NULL, NULL, ?)",
    )
    .bind(&transaction_id)
    .bind(&receipt_number)
    .bind(&store_id)
    .bind(&user.user_id)
    .bind(&input.customer_id)
    .bind(subtotal)
    .bind(discount_total)
    .bind(tax_total)
    .bind(grand_total)
    .bind(timestamp)
    .execute(&mut *tx)
    .await
    .map_err(AppError::db)?;

    for line in &lines {
        sqlx::query(
            "INSERT INTO `TRANSACTION_ITEM`
             (id, transaction_id, product_id, quantity, unit_price, cost_price, line_discount, tax_rate, line_total)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(new_id())
        .bind(&transaction_id)
        .bind(&line.product_id)
        .bind(line.quantity)
        .bind(line.unit_price)
        .bind(line.cost_price)
        .bind(line.line_discount)
        .bind(line.tax_rate)
        .bind(line.line_total)
        .execute(&mut *tx)
        .await
        .map_err(AppError::db)?;
    }

    // --- payments -----------------------------------------------------------
    if input.payments.is_empty() {
        let method = match &input.payment_method {
            Some(method) => parse_payment_method(method)?,
            None => "CASH",
        };
        sqlx::query(
            "INSERT INTO `PAYMENT` (id, transaction_id, method, amount, reference, created_at)
             VALUES (?, ?, ?, ?, NULL, ?)",
        )
        .bind(new_id())
        .bind(&transaction_id)
        .bind(method)
        .bind(grand_total)
        .bind(timestamp)
        .execute(&mut *tx)
        .await
        .map_err(AppError::db)?;
    } else {
        let mut paid = Decimal::ZERO;
        for payment in &input.payments {
            let method = parse_payment_method(&payment.method)?;
            let amount = round2(payment.amount.unwrap_or(Decimal::ZERO));
            if amount <= Decimal::ZERO {
                return Err(AppError::bad_request("payment amount must be positive"));
            }
            paid += amount;
            sqlx::query(
                "INSERT INTO `PAYMENT` (id, transaction_id, method, amount, reference, created_at)
                 VALUES (?, ?, ?, ?, ?, ?)",
            )
            .bind(new_id())
            .bind(&transaction_id)
            .bind(method)
            .bind(amount)
            .bind(&payment.reference)
            .bind(timestamp)
            .execute(&mut *tx)
            .await
            .map_err(AppError::db)?;
        }
        if round2(paid) != grand_total {
            return Err(AppError::bad_request(format!(
                "payments total {paid} does not match grand total {grand_total}"
            )));
        }
    }

    // --- loyalty ------------------------------------------------------------
    if let Some(customer_id) = &input.customer_id {
        let rate = Decimal::try_from(app_state.config.loyalty_earn_rate).unwrap_or(Decimal::ONE);
        let earned = (grand_total * rate).floor().to_i64().unwrap_or(0);
        if earned > 0 {
            sqlx::query("UPDATE `CUSTOMER` SET loyalty_points = loyalty_points + ? WHERE id = ?")
                .bind(earned)
                .bind(customer_id)
                .execute(&mut *tx)
                .await
                .map_err(AppError::db)?;

            sqlx::query(
                "INSERT INTO `CUSTOMER_LOYALTY_LOG`
                 (id, customer_id, points_delta, reason, transaction_id, created_by, created_at)
                 VALUES (?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(new_id())
            .bind(customer_id)
            .bind(earned)
            .bind(format!("Earned from receipt {receipt_number}"))
            .bind(&transaction_id)
            .bind(&user.user_id)
            .bind(timestamp)
            .execute(&mut *tx)
            .await
            .map_err(AppError::db)?;
        }
    }

    // --- low-stock watcher ---------------------------------------------------
    for line in &lines {
        check_low_stock(&mut tx, &app_state.config, &line.product_id, &user.user_id).await?;
    }

    tx.commit().await.map_err(AppError::db)?;

    Ok(transaction_id)
}

/// Full receipt: header + items + payments (outside the write transaction).
pub async fn transaction_detail(
    pool: &sqlx::Pool<MySql>,
    transaction_id: &str,
) -> Result<(TransactionRow, Vec<TransactionItemRow>, Vec<PaymentRow>), AppError> {
    let header = sqlx::query_as::<_, TransactionRow>(
        "SELECT
            t.id, t.receipt_number, t.store_id, s.name AS store_name,
            t.cashier_id, u.name AS cashier_name,
            t.customer_id, c.name AS customer_name,
            t.subtotal, t.discount_total, t.tax_total, t.grand_total, t.status,
            (SELECT CAST(COUNT(*) AS SIGNED) FROM `TRANSACTION_ITEM` ti WHERE ti.transaction_id = t.id) AS item_count,
            t.created_at
         FROM `TRANSACTION` t
         JOIN `STORE` s ON s.id = t.store_id
         JOIN `USER` u ON u.id = t.cashier_id
         LEFT JOIN `CUSTOMER` c ON c.id = t.customer_id
         WHERE t.id = ?",
    )
    .bind(transaction_id)
    .fetch_optional(pool)
    .await
    .map_err(AppError::db)?
    .ok_or_else(|| AppError::not_found("Transaction not found"))?;

    let items = sqlx::query_as::<_, TransactionItemRow>(
        "SELECT ti.id, ti.transaction_id, ti.product_id, p.sku, p.name AS name,
                ti.quantity, ti.unit_price, ti.cost_price, ti.line_discount, ti.tax_rate, ti.line_total
         FROM `TRANSACTION_ITEM` ti
         JOIN `PRODUCT` p ON p.id = ti.product_id
         WHERE ti.transaction_id = ?
         ORDER BY p.name",
    )
    .bind(transaction_id)
    .fetch_all(pool)
    .await
    .map_err(AppError::db)?;

    let payments = sqlx::query_as::<_, PaymentRow>(
        "SELECT id, transaction_id, method, amount, reference
         FROM `PAYMENT` WHERE transaction_id = ? ORDER BY created_at, id",
    )
    .bind(transaction_id)
    .fetch_all(pool)
    .await
    .map_err(AppError::db)?;

    Ok((header, items, payments))
}

/// Restore stock for a voided/refunded transaction: each line returns its
/// quantity as a fresh batch plus a `RETURN` adjustment row; loyalty points
/// earned on the receipt are reversed. Runs in the caller's transaction.
pub async fn restore_transaction_stock(
    tx: &mut sqlx::Transaction<'_, MySql>,
    _app_state: &AppState,
    transaction_id: &str,
    void_label: &str,
    reversed_by: &str,
) -> Result<(), AppError> {
    // Store + customer of the receipt (single read).
    let (store_id, customer_id): (String, Option<String>) =
        sqlx::query_as("SELECT store_id, customer_id FROM `TRANSACTION` WHERE id = ?")
            .bind(transaction_id)
            .fetch_one(&mut **tx)
            .await
            .map_err(AppError::db)?;

    let items = sqlx::query_as::<_, TransactionItemRow>(
        "SELECT ti.id, ti.transaction_id, ti.product_id, p.sku, p.name AS name,
                ti.quantity, ti.unit_price, ti.cost_price, ti.line_discount, ti.tax_rate, ti.line_total
         FROM `TRANSACTION_ITEM` ti
         JOIN `PRODUCT` p ON p.id = ti.product_id
         WHERE ti.transaction_id = ?",
    )
    .bind(transaction_id)
    .fetch_all(&mut **tx)
    .await
    .map_err(AppError::db)?;

    for item in items {
        crate::services::stock::add_batch(
            tx,
            &item.product_id,
            &store_id,
            i64::from(item.quantity),
            item.cost_price,
            Some("RESTORE"),
            None,
            "RETURN",
            Some(&format!("{void_label} of receipt line for {}", item.name)),
            Some(reversed_by),
        )
        .await?;
    }

    // Reverse loyalty points earned on this receipt.
    let points: i64 = sqlx::query_scalar(
        "SELECT CAST(COALESCE(SUM(points_delta), 0) AS SIGNED)
         FROM `CUSTOMER_LOYALTY_LOG` WHERE transaction_id = ? AND points_delta > 0",
    )
    .bind(transaction_id)
    .fetch_one(&mut **tx)
    .await
    .map_err(AppError::db)?;

    if let Some(customer_id) = customer_id
        && points > 0
    {
        sqlx::query(
            "UPDATE `CUSTOMER`
                 SET loyalty_points = GREATEST(0, loyalty_points - ?)
                 WHERE id = ?",
        )
        .bind(points)
        .bind(&customer_id)
        .execute(&mut **tx)
        .await
        .map_err(AppError::db)?;

        sqlx::query(
            "INSERT INTO `CUSTOMER_LOYALTY_LOG`
                 (id, customer_id, points_delta, reason, transaction_id, created_by, created_at)
                 VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(new_id())
        .bind(&customer_id)
        .bind(-points)
        .bind(format!("{void_label} reversal of receipt {transaction_id}"))
        .bind(transaction_id)
        .bind(reversed_by)
        .bind(now())
        .execute(&mut **tx)
        .await
        .map_err(AppError::db)?;
    }

    Ok(())
}
