use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    let current_exe = std::env::current_exe()?;
    let exe_dir = current_exe.parent().unwrap();
    let main_bin = exe_dir.join("browser-bookmark-sync");

    let status = std::process::Command::new(if main_bin.exists() { main_bin } else { current_exe.with_file_name("browser-bookmark-sync") })
        .arg("export")
        .arg("-o")
        .arg("~/Downloads/all_bookmarks.html")
        .arg("--all-profiles")
        .arg("--merge")
        .status()?;

    if !status.success() {
        std::process::exit(status.code().unwrap_or(1));
    }
    Ok(())
}
