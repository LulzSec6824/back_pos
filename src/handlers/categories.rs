//! Product categories.

use actix_web::{HttpResponse, delete, get, patch, post, web};
use serde::Deserialize;
use serde_json::json;

use crate::app_state::AppState;
use crate::auth::AuthenticatedUser;
use crate::error::AppError;
use crate::models::product::{CategoryInput, CategoryRow};
use crate::models::user::Role;
use crate::utils::new_id;

#[get("/categories")]
pub async fn list_categories(
    state: web::Data<AppState>,
    _user: AuthenticatedUser,
) -> Result<HttpResponse, AppError> {
    let rows = sqlx::query_as::<_, CategoryRow>(
        "SELECT c.id, c.name, c.description, c.parent_id,
                CAST(COALESCE(COUNT(p.id), 0) AS SIGNED) AS product_count
         FROM `CATEGORY` c
         LEFT JOIN `PRODUCT` p ON p.category_id = c.id
         GROUP BY c.id, c.name, c.description, c.parent_id
         ORDER BY c.name",
    )
    .fetch_all(&state.db)
    .await
    .map_err(AppError::db)?;
    Ok(HttpResponse::Ok().json(rows))
}

#[post("/categories")]
pub async fn create_category(
    state: web::Data<AppState>,
    user: AuthenticatedUser,
    body: web::Json<CategoryInput>,
) -> Result<HttpResponse, AppError> {
    user.ensure_role(&[Role::Admin, Role::Manager, Role::InventoryManager])?;
    let category_id = new_id();

    sqlx::query("INSERT INTO `CATEGORY` (id, name, description, parent_id) VALUES (?, ?, ?, ?)")
        .bind(&category_id)
        .bind(body.name.trim())
        .bind(&body.description)
        .bind(&body.parent_id)
        .execute(&state.db)
        .await
        .map_err(AppError::db)?;

    Ok(HttpResponse::Created().json(json!({ "id": category_id, "name": body.name.trim() })))
}

#[derive(Debug, Deserialize)]
pub struct UpdateCategoryInput {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub parent_id: Option<String>,
}

#[patch("/categories/{id}")]
pub async fn update_category(
    state: web::Data<AppState>,
    user: AuthenticatedUser,
    path: web::Path<String>,
    body: web::Json<UpdateCategoryInput>,
) -> Result<HttpResponse, AppError> {
    user.ensure_role(&[Role::Admin, Role::Manager, Role::InventoryManager])?;
    let id = path.into_inner();

    let existing = sqlx::query_as::<_, CategoryRow>(
        "SELECT c.id, c.name, c.description, c.parent_id, CAST(0 AS SIGNED) AS product_count
         FROM `CATEGORY` c WHERE c.id = ?",
    )
    .bind(&id)
    .fetch_optional(&state.db)
    .await
    .map_err(AppError::db)?
    .ok_or_else(|| AppError::not_found("Category not found"))?;

    sqlx::query("UPDATE `CATEGORY` SET name = ?, description = ?, parent_id = ? WHERE id = ?")
        .bind(body.name.clone().unwrap_or(existing.name))
        .bind(body.description.clone().or(existing.description))
        .bind(body.parent_id.clone().or(existing.parent_id))
        .bind(&id)
        .execute(&state.db)
        .await
        .map_err(AppError::db)?;

    Ok(HttpResponse::Ok().json(json!({ "id": id })))
}

#[delete("/categories/{id}")]
pub async fn delete_category(
    state: web::Data<AppState>,
    user: AuthenticatedUser,
    path: web::Path<String>,
) -> Result<HttpResponse, AppError> {
    user.ensure_role(&[Role::Admin, Role::Manager, Role::InventoryManager])?;
    let id = path.into_inner();

    let exists =
        sqlx::query_scalar::<_, i64>("SELECT CAST(1 AS SIGNED) FROM `CATEGORY` WHERE id = ?")
            .bind(&id)
            .fetch_optional(&state.db)
            .await
            .map_err(AppError::db)?;
    if exists.is_none() {
        return Err(AppError::not_found("Category not found"));
    }

    let in_use: i64 =
        sqlx::query_scalar("SELECT CAST(COUNT(*) AS SIGNED) FROM `PRODUCT` WHERE category_id = ?")
            .bind(&id)
            .fetch_one(&state.db)
            .await
            .map_err(AppError::db)?;
    if in_use > 0 {
        return Err(AppError::conflict(
            "category still has products; reassign them first",
        ));
    }

    // Children reference this parent; deleting would hit the FK as a raw 500.
    let children: i64 =
        sqlx::query_scalar("SELECT CAST(COUNT(*) AS SIGNED) FROM `CATEGORY` WHERE parent_id = ?")
            .bind(&id)
            .fetch_one(&state.db)
            .await
            .map_err(AppError::db)?;
    if children > 0 {
        return Err(AppError::conflict(
            "category still has subcategories; reassign them first",
        ));
    }

    sqlx::query("DELETE FROM `CATEGORY` WHERE id = ?")
        .bind(&id)
        .execute(&state.db)
        .await
        .map_err(AppError::db)?;
    Ok(HttpResponse::Ok().json(json!({ "deleted": true })))
}
