use anyhow::Result;
use std::env;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    let args: Vec<String> = env::args().skip(1).collect();
    let mut args_with_cmd = vec!["pipeline".to_string()];
    args_with_cmd.extend(args);

    let current_exe = env::current_exe()?;
    let exe_dir = current_exe.parent().unwrap();
    let main_bin = exe_dir.join("browser-bookmark-sync");

    let status = std::process::Command::new(if main_bin.exists() {
        main_bin
    } else {
        current_exe.with_file_name("browser-bookmark-sync")
    })
    .args(&args_with_cmd)
    .status()?;

    if !status.success() {
        std::process::exit(status.code().unwrap_or(1));
    }
    Ok(())
}
