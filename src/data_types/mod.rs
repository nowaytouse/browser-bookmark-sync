//! Data types module for browser data extraction and bookmark model abstractions

pub mod canonical;
pub mod cookie;
pub mod download;
pub mod manifest;
pub mod models;
pub mod password;

pub use cookie::extract_chromium_cookies;
pub use download::extract_chromium_downloads;
pub use password::extract_chromium_passwords;
