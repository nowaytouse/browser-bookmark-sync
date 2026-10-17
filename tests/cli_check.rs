use std::{fs, process::Command};

#[test]
fn disabled_delete_fails_before_checking_and_preserves_output(
) -> Result<(), Box<dyn std::error::Error>> {
    let temp = tempfile::tempdir()?;
    let output_path = temp.path().join("existing-output.html");
    fs::write(&output_path, "preserve this output")?;
    let output = Command::new(env!("CARGO_BIN_EXE_browser-bookmark-sync"))
        .current_dir(temp.path())
        .arg("--log-file")
        .arg(temp.path().join("cli.log"))
        .args(["check", "--delete", "--file"])
        .arg(temp.path().join("not-read.html"))
        .arg("--output")
        .arg(&output_path)
        .output()?;

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("--delete is disabled"));
    assert_eq!(fs::read_to_string(output_path)?, "preserve this output");
    assert!(!fs::read_to_string(temp.path().join("cli.log"))?.contains("读取收藏夹"));
    Ok(())
}
