//! Data types module for browser data extraction
//!
//! Supports: passwords, cookies, downloads, localStorage, extensions

pub mod cookie;
pub mod download;
pub mod password;

pub use cookie::{extract_chromium_cookies, extract_firefox_cookies, Cookie};
pub use download::{extract_chromium_downloads, extract_firefox_downloads, Download};
pub use password::{extract_chromium_passwords, extract_firefox_passwords, Password};
