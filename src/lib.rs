#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::as_conversions,
        clippy::pedantic,
        clippy::nursery,
        clippy::collection_is_never_read,
        clippy::unnecessary_wraps,
        clippy::needless_continue,
    )
)]

pub mod browser_utils;
pub mod browsers;
pub mod chromium_sync;
pub mod cleanup;
pub mod cloud_reset;
pub mod crypto;
pub mod data_types;
pub mod db_safety;
pub mod enhanced_rules;
pub mod firefox_sync;
pub mod firefox_sync_api;
pub mod gui;
pub mod hackbrowserdata;
pub mod scheduler;
pub mod sync;
pub mod sync_flags;
pub mod url_checker;
pub mod validator;

#[must_use]
pub fn expand_path(path: &str) -> String {
    if let Some(stripped) = path.strip_prefix("~/") {
        let home = std::env::var("HOME").unwrap_or_default();
        format!("{home}/{stripped}")
    } else {
        path.to_string()
    }
}
