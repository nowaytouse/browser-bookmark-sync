use crate::browsers::BrowserType;
use std::path::PathBuf;

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
        self.browsers_not_detected
            .push((browser, reason.to_string()));
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
        output.push_str(&format!(
            "\n📊 Summary: {}/{} browsers validated successfully\n\n",
            total_validated, total_detected
        ));

        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_validation_report_new() {
        let report = ValidationReport::new();
        assert!(report.browsers_detected.is_empty());
        assert!(report.browsers_not_detected.is_empty());
        assert!(report.bookmarks_read.is_empty());
        assert!(report.read_errors.is_empty());
        assert!(report.validations_passed.is_empty());
        assert!(report.validations_failed.is_empty());
    }

    #[test]
    fn test_add_browser_detected() {
        let mut report = ValidationReport::new();
        let path = PathBuf::from("/test/path");

        report.add_browser_detected(BrowserType::Chrome, path.clone());

        assert_eq!(report.browsers_detected.len(), 1);
        assert_eq!(report.browsers_detected[0].0.name(), "Chrome");
        assert_eq!(report.browsers_detected[0].1, path);
    }

    #[test]
    fn test_add_not_detected() {
        let mut report = ValidationReport::new();

        report.add_not_detected(BrowserType::Safari, "Profile not found");

        assert_eq!(report.browsers_not_detected.len(), 1);
        assert_eq!(report.browsers_not_detected[0].0.name(), "Safari");
        assert_eq!(report.browsers_not_detected[0].1, "Profile not found");
    }

    #[test]
    fn test_add_bookmarks_read() {
        let mut report = ValidationReport::new();

        report.add_bookmarks_read(BrowserType::Waterfox, 150);

        assert_eq!(report.bookmarks_read.len(), 1);
        assert_eq!(report.bookmarks_read[0].1, 150);
    }

    #[test]
    fn test_add_read_error() {
        let mut report = ValidationReport::new();

        report.add_read_error(BrowserType::Brave, "Database locked");

        assert_eq!(report.read_errors.len(), 1);
        assert_eq!(report.read_errors[0].1, "Database locked");
    }

    #[test]
    fn test_add_validation_passed() {
        let mut report = ValidationReport::new();

        report.add_validation_passed(BrowserType::Chrome);
        report.add_validation_passed(BrowserType::Safari);

        assert_eq!(report.validations_passed.len(), 2);
    }

    #[test]
    fn test_add_validation_failed() {
        let mut report = ValidationReport::new();

        report.add_validation_failed(BrowserType::BraveNightly, "Invalid structure");

        assert_eq!(report.validations_failed.len(), 1);
        assert_eq!(report.validations_failed[0].1, "Invalid structure");
    }

    #[test]
    fn test_format_basic() {
        let mut report = ValidationReport::new();
        report.add_browser_detected(BrowserType::Chrome, PathBuf::from("/test"));
        report.add_validation_passed(BrowserType::Chrome);

        let output = report.format(false);

        assert!(output.contains("Validation Report"));
        assert!(output.contains("Chrome"));
        assert!(output.contains("PASSED"));
        assert!(output.contains("Summary"));
    }

    #[test]
    fn test_format_detailed() {
        let mut report = ValidationReport::new();
        report.add_browser_detected(BrowserType::Safari, PathBuf::from("/Library/Safari"));
        report.add_not_detected(BrowserType::Brave, "Not installed");
        report.add_bookmarks_read(BrowserType::Safari, 50);
        report.add_validation_passed(BrowserType::Safari);

        let output = report.format(true);

        assert!(output.contains("Safari"));
        assert!(output.contains("Brave"));
        assert!(output.contains("Not installed"));
        assert!(output.contains("/Library/Safari"));
    }

    #[test]
    fn test_summary_count() {
        let mut report = ValidationReport::new();
        report.add_browser_detected(BrowserType::Chrome, PathBuf::from("/test1"));
        report.add_browser_detected(BrowserType::Safari, PathBuf::from("/test2"));
        report.add_browser_detected(BrowserType::Waterfox, PathBuf::from("/test3"));
        report.add_validation_passed(BrowserType::Chrome);
        report.add_validation_passed(BrowserType::Safari);
        report.add_validation_failed(BrowserType::Waterfox, "Error");

        let output = report.format(false);

        assert!(output.contains("2/3 browsers validated successfully"));
    }

    #[test]
    fn test_empty_report_format() {
        let report = ValidationReport::new();
        let output = report.format(false);

        assert!(output.contains("Validation Report"));
        assert!(output.contains("0/0 browsers validated successfully"));
    }
}
