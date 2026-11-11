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
///
/// Values are captured as raw strings and parsed in [`PageQuery::resolve`].
/// These structs are merged into endpoint queries with `#[serde(flatten)]`,
/// and flattened fields are deserialized through serde's buffered `Content`
/// format, which carries every scalar as a JSON value — an `Option<i64>`
/// field would then fail on real query strings (`?per_page=5`) with
/// `invalid type: string "5", expected i64`. Parsing here keeps flatten
/// compatible.
#[derive(Debug, Deserialize, Default)]
pub struct PageQuery {
    pub page: Option<String>,
    pub per_page: Option<String>,
}

impl PageQuery {
    fn parse(value: &Option<String>) -> Option<i64> {
        value
            .as_deref()
            .and_then(|raw| raw.trim().parse::<i64>().ok())
    }

    /// Clamp to configured bounds; `page` is 1-based.
    pub fn resolve(&self, config: &AppConfig) -> (i64, i64) {
        let page = Self::parse(&self.page).unwrap_or(1).max(1);
        let per_page = Self::parse(&self.per_page)
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
