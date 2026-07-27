//! Data types module for browser data extraction and bookmark model abstractions

pub mod canonical;
pub mod cookie;
pub mod download;
pub mod manifest;
pub mod models;
pub mod password;

pub use cookie::extract_chromium_cookies;
pub use download::extract_chromium_downloads;
pub use password::*;

#[allow(unused_imports)]
pub use cookie::{extract_firefox_cookies, Cookie};
#[allow(unused_imports)]
pub use download::{extract_firefox_downloads, Download};
#[allow(unused_imports)]
pub use password::{extract_firefox_passwords, Password};
