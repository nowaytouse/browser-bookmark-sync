use indicatif::{ProgressBar, ProgressStyle};

/// Create a progress bar for bookmark processing
pub fn create_bookmark_progress_bar(total: u64, message: &str) -> ProgressBar {
    let pb = ProgressBar::new(total);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} {msg}")
            .unwrap()
            .progress_chars("=>-")
    );
    pb.set_message(message.to_string());
    pb
}

/// Create a spinner for indeterminate operations
pub fn create_spinner(message: &str) -> ProgressBar {
    let pb = ProgressBar::new_spinner();
    pb.set_style(
        ProgressStyle::default_spinner()
            .template("{spinner:.green} {msg}")
            .unwrap()
    );
    pb.set_message(message.to_string());
    pb
}

/// Create a progress bar for file operations
pub fn create_file_progress_bar(total: u64, message: &str) -> ProgressBar {
    let pb = ProgressBar::new(total);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("{spinner:.green} [{elapsed_precise}] [{bar:40.green/yellow}] {bytes}/{total_bytes} {msg}")
            .unwrap()
            .progress_chars("█▓▒░ ")
    );
    pb.set_message(message.to_string());
    pb
}

/// Finish progress bar with success message
pub fn finish_with_success(pb: &ProgressBar, message: &str) {
    pb.finish_with_message(format!("✅ {}", message));
}

/// Finish progress bar with error message
pub fn finish_with_error(pb: &ProgressBar, message: &str) {
    pb.finish_with_message(format!("❌ {}", message));
}
