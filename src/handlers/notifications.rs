//! Notifications inbox, role-filtered.

use actix_web::{HttpResponse, get, post, web};
use serde_json::json;

use crate::app_state::AppState;
use crate::auth::AuthenticatedUser;
use crate::error::AppError;
use crate::handlers::{PageQuery, PageResult};
use crate::models::notification::NotificationRow;

#[derive(Debug, serde::Deserialize, Default)]
pub struct NotificationQuery {
    pub unread: Option<bool>,
    pub type_name: Option<String>,
    #[serde(flatten)]
    pub page: PageQuery,
}

/// Audience filter: rows addressed to everyone (`ALL`) or to the caller's role.
#[get("/notifications")]
pub async fn list_notifications(
    state: web::Data<AppState>,
    user: AuthenticatedUser,
    query: web::Query<NotificationQuery>,
) -> Result<HttpResponse, AppError> {
    user.ensure_staff()?;
    let (page, per_page) = query.page.resolve(&state.config);
    let offset = query.page.offset(&state.config);
    let NotificationQuery {
        unread, type_name, ..
    } = query.into_inner();

    let mut conditions = vec!["(n.audience = 'ALL' OR n.audience = ?)".to_string()];
    let mut binds = vec![user.role.clone()];
    if unread == Some(true) {
        conditions.push("n.is_read = FALSE".to_string());
    }
    if let Some(type_name) = &type_name {
        conditions.push("n.type_name = ?".to_string());
        binds.push(type_name.clone());
    }
    let where_clause = format!(" WHERE {}", conditions.join(" AND "));

    let count_sql = format!("SELECT CAST(COUNT(*) AS SIGNED) FROM `NOTIFICATION` n{where_clause}");
    let mut count_query = sqlx::query_scalar::<_, i64>(sqlx::AssertSqlSafe(count_sql.clone()));
    for bind in &binds {
        count_query = count_query.bind(bind);
    }
    let total = count_query
        .fetch_one(&state.db)
        .await
        .map_err(AppError::db)?;

    let unread_count: i64 = sqlx::query_scalar(
        "SELECT CAST(COUNT(*) AS SIGNED) FROM `NOTIFICATION` n
         WHERE (n.audience = 'ALL' OR n.audience = ?) AND n.is_read = FALSE",
    )
    .bind(&user.role)
    .fetch_one(&state.db)
    .await
    .map_err(AppError::db)?;

    let rows_sql = format!(
        "SELECT n.id, n.type_name, n.title, n.message, n.audience, n.entity_type, n.entity_id, n.is_read, n.created_at
         FROM `NOTIFICATION` n{where_clause}
         ORDER BY n.created_at DESC LIMIT ? OFFSET ?"
    );

    let mut rows_query =
        sqlx::query_as::<_, NotificationRow>(sqlx::AssertSqlSafe(rows_sql.clone()));
    for bind in &binds {
        rows_query = rows_query.bind(bind);
    }
    let rows = rows_query
        .bind(per_page)
        .bind(offset)
        .fetch_all(&state.db)
        .await
        .map_err(AppError::db)?;

    let mut body = serde_json::to_value(PageResult::new(rows, total, page, per_page))
        .map_err(|error| AppError::internal(error.to_string()))?;
    body["unread_count"] = json!(unread_count);

    Ok(HttpResponse::Ok().json(body))
}

#[post("/notifications/{id}/read")]
pub async fn mark_read(
    state: web::Data<AppState>,
    user: AuthenticatedUser,
    path: web::Path<String>,
) -> Result<HttpResponse, AppError> {
    user.ensure_staff()?;
    sqlx::query(
        "UPDATE `NOTIFICATION` SET is_read = TRUE
         WHERE id = ? AND (audience = 'ALL' OR audience = ?)",
    )
    .bind(path.into_inner())
    .bind(&user.role)
    .execute(&state.db)
    .await
    .map_err(AppError::db)?;
    Ok(HttpResponse::Ok().json(json!({ "read": true })))
}

#[post("/notifications/read-all")]
pub async fn mark_all_read(
    state: web::Data<AppState>,
    user: AuthenticatedUser,
) -> Result<HttpResponse, AppError> {
    user.ensure_staff()?;
    sqlx::query(
        "UPDATE `NOTIFICATION` SET is_read = TRUE
         WHERE is_read = FALSE AND (audience = 'ALL' OR audience = ?)",
    )
    .bind(&user.role)
    .execute(&state.db)
    .await
    .map_err(AppError::db)?;
    Ok(HttpResponse::Ok().json(json!({ "read": "all" })))
}
