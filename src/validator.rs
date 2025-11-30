use std::path::PathBuf;
use crate::browsers::BrowserType;

pub struct ValidationReport {
    browsers_detected: Vec<(BrowserType, PathBuf)>,
    browsers_not_detected: Vec<(BrowserType, String)>,
    bookmarks_read: Vec<(BrowserType, usize)>,
    read_errors: Vec<(BrowserType, String)>,
    validations_passed: Vec<BrowserType>,
    validations_failed: Vec<(BrowserType, String)>,
}

impl ValidationReport {
    pub fn new() -> Self {
        Self {
            browsers_detected: Vec::new(),
            browsers_not_detected: Vec::new(),
            bookmarks_read: Vec::new(),
            read_errors: Vec::new(),
            validations_passed: Vec::new(),
            validations_failed: Vec::new(),
        }
    }

    pub fn add_browser_detected(&mut self, browser: BrowserType, path: PathBuf) {
        self.browsers_detected.push((browser, path));
    }

    pub fn add_not_detected(&mut self, browser: BrowserType, reason: &str) {
        self.browsers_not_detected.push((browser, reason.to_string()));
    }

    pub fn add_bookmarks_read(&mut self, browser: BrowserType, count: usize) {
        self.bookmarks_read.push((browser, count));
    }

    pub fn add_read_error(&mut self, browser: BrowserType, error: &str) {
        self.read_errors.push((browser, error.to_string()));
    }

    pub fn add_validation_passed(&mut self, browser: BrowserType) {
        self.validations_passed.push(browser);
    }

    pub fn add_validation_failed(&mut self, browser: BrowserType, reason: &str) {
        self.validations_failed.push((browser, reason.to_string()));
    }

    pub fn format(&self, detailed: bool) -> String {
        let mut output = String::new();
        
        output.push_str("\n🔍 Bookmark Validation Report\n");
        output.push_str("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n\n");

        // Detected browsers
        output.push_str("✅ Detected Browsers:\n");
        for (browser, path) in &self.browsers_detected {
            output.push_str(&format!("  • {}\n", browser.name()));
            if detailed {
                output.push_str(&format!("    Path: {:?}\n", path));
            }
        }
        output.push('\n');

        // Not detected browsers
        if !self.browsers_not_detected.is_empty() {
            output.push_str("❌ Not Detected:\n");
            for (browser, reason) in &self.browsers_not_detected {
                output.push_str(&format!("  • {}\n", browser.name()));
                if detailed {
                    output.push_str(&format!("    Reason: {}\n", reason));
                }
            }
            output.push('\n');
        }

        // Bookmarks read
        if !self.bookmarks_read.is_empty() {
            output.push_str("📖 Bookmarks Read:\n");
            for (browser, count) in &self.bookmarks_read {
                output.push_str(&format!("  • {}: {} bookmarks\n", browser.name(), count));
            }
            output.push('\n');
        }

        // Read errors
        if !self.read_errors.is_empty() {
            output.push_str("⚠️  Read Errors:\n");
            for (browser, error) in &self.read_errors {
                output.push_str(&format!("  • {}\n", browser.name()));
                if detailed {
                    output.push_str(&format!("    Error: {}\n", error));
                }
            }
            output.push('\n');
        }

        // Validation results
        output.push_str("🔐 Validation Results:\n");
        for browser in &self.validations_passed {
            output.push_str(&format!("  ✅ {} - PASSED\n", browser.name()));
        }
        for (browser, reason) in &self.validations_failed {
            output.push_str(&format!("  ❌ {} - FAILED\n", browser.name()));
            if detailed {
                output.push_str(&format!("    Reason: {}\n", reason));
            }
        }

        output.push_str("\n━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n");
        
        // Summary
        let total_detected = self.browsers_detected.len();
        let total_validated = self.validations_passed.len();
        output.push_str(&format!("\n📊 Summary: {}/{} browsers validated successfully\n\n", 
            total_validated, total_detected));

        output
    }
}
