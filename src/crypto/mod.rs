//! Crypto module for browser data decryption
//!
//! Supports Chromium-based browsers and Firefox on macOS.

pub mod chromium;

pub use chromium::{decrypt_chromium_data, get_chromium_master_key, is_encrypted};
