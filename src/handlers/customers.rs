//! Customers + loyalty points.

use actix_web::{HttpResponse, get, patch, post, web};

use crate::app_state::AppState;
use crate::auth::AuthenticatedUser;
use crate::error::AppError;
use crate::handlers::{PageQuery, PageResult};
use crate::models::customer::{CustomerInput, CustomerRow, UpdateCustomerInput};

#[derive(Debug, serde::Deserialize, Default)]
pub struct CustomerQuery {
    pub search: Option<String>,
    #[serde(flatten)]
    pub page: PageQuery,
}

#[get("/customers")]
pub async fn list_customers(
    state: web::Data<AppState>,
    user: AuthenticatedUser,
    query: web::Query<CustomerQuery>,
) -> Result<HttpResponse, AppError> {
    user.ensure_staff()?;

    let mut conditions: Vec<String> = Vec::new();
    let mut binds: Vec<String> = Vec::new();
    if let Some(search) = &query.search {
        conditions.push("(c.name LIKE ? OR c.phone LIKE ? OR c.email LIKE ?)".to_string());
        let like = format!("%{}%", search.trim());
        binds.push(like.clone());
        binds.push(like.clone());
        binds.push(like);
    }
    let where_clause = if conditions.is_empty() {
        String::new()
    } else {
        format!(" WHERE {}", conditions.join(" AND "))
    };

    let count_sql = format!("SELECT CAST(COUNT(*) AS SIGNED) FROM `CUSTOMER` c{where_clause}");
    let mut count_query = sqlx::query_scalar::<_, i64>(sqlx::AssertSqlSafe(count_sql.clone()));
    for bind in &binds {
        count_query = count_query.bind(bind);
    }
    let total = count_query
        .fetch_one(&state.db)
        .await
        .map_err(AppError::db)?;

    let (page, per_page) = query.page.resolve(&state.config);
    let offset = query.page.offset(&state.config);

    let rows_sql = format!(
        "SELECT c.id, c.name, c.phone, c.email, c.loyalty_points, c.is_member, c.created_at,
                COALESCE((SELECT SUM(t.grand_total) FROM `TRANSACTION` t
                          WHERE t.customer_id = c.id AND t.status = 'COMPLETED'), 0) AS total_spent
         FROM `CUSTOMER` c{where_clause}
         ORDER BY c.created_at DESC LIMIT ? OFFSET ?"
    );

    let mut rows_query = sqlx::query_as::<_, CustomerRow>(sqlx::AssertSqlSafe(rows_sql.clone()));
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

#[post("/customers")]
pub async fn create_customer(
    state: web::Data<AppState>,
    user: AuthenticatedUser,
    body: web::Json<CustomerInput>,
) -> Result<HttpResponse, AppError> {
    user.ensure_staff()?;
    let input = body.into_inner();
    let customer_id = crate::utils::new_id();

    sqlx::query(
        "INSERT INTO `CUSTOMER` (id, name, phone, email, loyalty_points, is_member, created_at)
         VALUES (?, ?, ?, ?, 0, TRUE, ?)",
    )
    .bind(&customer_id)
    .bind(input.name.trim())
    .bind(&input.phone)
    .bind(&input.email)
    .bind(crate::utils::now())
    .execute(&state.db)
    .await
    .map_err(AppError::db)?;

    let row = sqlx::query_as::<_, CustomerRow>(
        "SELECT c.id, c.name, c.phone, c.email, c.loyalty_points, c.is_member, c.created_at,
                CAST(0 AS SIGNED) AS total_spent
         FROM `CUSTOMER` c WHERE c.id = ?",
    )
    .bind(&customer_id)
    .fetch_one(&state.db)
    .await
    .map_err(AppError::db)?;
    Ok(HttpResponse::Created().json(row))
}

#[patch("/customers/{id}")]
pub async fn update_customer(
    state: web::Data<AppState>,
    user: AuthenticatedUser,
    path: web::Path<String>,
    body: web::Json<UpdateCustomerInput>,
) -> Result<HttpResponse, AppError> {
    user.ensure_staff()?;
    let id = path.into_inner();

    let existing = sqlx::query_as::<_, CustomerRow>(
        "SELECT c.id, c.name, c.phone, c.email, c.loyalty_points, c.is_member, c.created_at,
                CAST(0 AS SIGNED) AS total_spent
         FROM `CUSTOMER` c WHERE c.id = ?",
    )
    .bind(&id)
    .fetch_optional(&state.db)
    .await
    .map_err(AppError::db)?
    .ok_or_else(|| AppError::not_found("Customer not found"))?;

    sqlx::query(
        "UPDATE `CUSTOMER` SET name = ?, phone = ?, email = ?, loyalty_points = ? WHERE id = ?",
    )
    .bind(body.name.clone().unwrap_or(existing.name))
    .bind(body.phone.clone().or(existing.phone))
    .bind(body.email.clone().or(existing.email))
    .bind(body.loyalty_points.unwrap_or(existing.loyalty_points))
    .bind(&id)
    .execute(&state.db)
    .await
    .map_err(AppError::db)?;

    let row = sqlx::query_as::<_, CustomerRow>(
        "SELECT c.id, c.name, c.phone, c.email, c.loyalty_points, c.is_member, c.created_at,
                CAST(0 AS SIGNED) AS total_spent
         FROM `CUSTOMER` c WHERE c.id = ?",
    )
    .bind(&id)
    .fetch_one(&state.db)
    .await
    .map_err(AppError::db)?;
    Ok(HttpResponse::Ok().json(row))
}
