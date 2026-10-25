pub mod auth;
pub mod categories;
pub mod customers;
pub mod health;
pub mod inventory;
pub mod meta;
pub mod notifications;
pub mod products;
pub mod purchases;
pub mod reports;
pub mod stores;
pub mod suppliers;
pub mod transactions;
pub mod users;

use serde::Deserialize;

use crate::config::AppConfig;

/// Standard pagination query params.
#[derive(Debug, Deserialize, Default)]
pub struct PageQuery {
    pub page: Option<i64>,
    pub per_page: Option<i64>,
}

impl PageQuery {
    /// Clamp to configured bounds; `page` is 1-based.
    pub fn resolve(&self, config: &AppConfig) -> (i64, i64) {
        let page = self.page.unwrap_or(1).max(1);
        let per_page = self
            .per_page
            .unwrap_or(config.pagination_default_limit)
            .clamp(1, config.pagination_max_limit);
        (page, per_page)
    }

    pub fn offset(&self, config: &AppConfig) -> i64 {
        let (page, per_page) = self.resolve(config);
        (page - 1) * per_page
    }
}

#[derive(Debug, serde::Serialize)]
pub struct PageResult<T> {
    pub items: Vec<T>,
    pub total: i64,
    pub page: i64,
    pub per_page: i64,
}

impl<T> PageResult<T> {
    pub fn new(items: Vec<T>, total: i64, page: i64, per_page: i64) -> Self {
        Self {
            items,
            total,
            page,
            per_page,
        }
    }
}
