//! Data types module for browser data extraction
//!
//! Supports: passwords, cookies, downloads, localStorage, extensions

pub mod cookie;
pub mod download;
pub mod password;

pub use cookie::extract_chromium_cookies;
pub use download::extract_chromium_downloads;
pub use password::extract_chromium_passwords;

// Firefox support (currently unused but kept for future)
#[allow(unused_imports)]
pub use cookie::{extract_firefox_cookies, Cookie};
#[allow(unused_imports)]
pub use download::{extract_firefox_downloads, Download};
#[allow(unused_imports)]
pub use password::{extract_firefox_passwords, Password};
