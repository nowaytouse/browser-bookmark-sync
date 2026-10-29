use std::{
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};

fn run_export(
    directory: &Path,
    output_path: &Path,
    extra_args: &[OsString],
) -> std::io::Result<(Output, PathBuf)> {
    let log_path = directory.join("cli.log");
    let output = Command::new(env!("CARGO_BIN_EXE_browser-bookmark-sync"))
        .current_dir(directory)
        .arg("--log-file")
        .arg(&log_path)
        .arg("export")
        .arg("--output")
        .arg(output_path)
        .args(extra_args)
        .output()?;
    Ok((output, log_path))
}

fn diagnostic(output: &Output) -> String {
    format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn sentinel_output(directory: &Path) -> std::io::Result<PathBuf> {
    let path = directory.join("existing-output.html");
    fs::write(&path, b"preserve this output\n")?;
    Ok(path)
}

#[test]
fn disabled_password_export_fails_without_touching_output() -> Result<(), Box<dyn std::error::Error>>
{
    let temp = tempfile::tempdir()?;
    let output_path = sentinel_output(temp.path())?;
    let (output, log_path) =
        run_export(temp.path(), &output_path, &[OsString::from("--passwords")])?;

    assert!(!output.status.success());
    assert!(diagnostic(&output).contains("Non-bookmark export options are disabled"));
    assert_eq!(fs::read(&output_path)?, b"preserve this output\n");
    assert!(!fs::read_to_string(log_path)?.contains("Exporting browser data"));
    Ok(())
}

#[test]
fn missing_update_input_fails_before_export_and_preserves_output(
) -> Result<(), Box<dyn std::error::Error>> {
    let temp = tempfile::tempdir()?;
    let output_path = sentinel_output(temp.path())?;
    let missing_update = temp.path().join("missing-update.html");
    let (output, log_path) = run_export(
        temp.path(),
        &output_path,
        &[
            OsString::from("--update"),
            missing_update.as_os_str().to_owned(),
        ],
    )?;

    assert!(!output.status.success());
    assert!(diagnostic(&output).contains("missing-update.html"));
    assert_eq!(fs::read(&output_path)?, b"preserve this output\n");
    let log = fs::read_to_string(log_path)?;
    assert!(!log.contains("Exporting browser data"));
    Ok(())
}

#[test]
fn malformed_update_input_fails_before_export_and_preserves_output(
) -> Result<(), Box<dyn std::error::Error>> {
    let temp = tempfile::tempdir()?;
    let output_path = sentinel_output(temp.path())?;
    let update = temp.path().join("not-bookmarks.html");
    fs::write(&update, "not a bookmark export")?;
    let (output, log_path) = run_export(
        temp.path(),
        &output_path,
        &[OsString::from("--update"), update.into_os_string()],
    )?;

    assert!(!output.status.success());
    assert!(diagnostic(&output).contains("not a bookmark HTML export"));
    assert_eq!(fs::read(&output_path)?, b"preserve this output\n");
    assert!(!fs::read_to_string(log_path)?.contains("Exporting browser data"));
    Ok(())
}

#[test]
fn existing_validation_copy_fails_before_collecting_and_preserves_both_files(
) -> Result<(), Box<dyn std::error::Error>> {
    let temp = tempfile::tempdir()?;
    let output_path = sentinel_output(temp.path())?;
    let copy = temp.path().join("validation.html");
    fs::write(&copy, b"preserve validation file")?;

    let (output, log_path) = run_export(
        temp.path(),
        &output_path,
        &[
            OsString::from("--validate-copy"),
            copy.clone().into_os_string(),
        ],
    )?;

    assert!(!output.status.success());
    assert!(diagnostic(&output).contains("refusing to overwrite"));
    assert_eq!(fs::read(&copy)?, b"preserve validation file");
    assert_eq!(fs::read(&output_path)?, b"preserve this output\n");
    assert!(!fs::read_to_string(log_path)?.contains("Exporting browser data"));
    Ok(())
}

#[test]
fn zero_validation_concurrency_fails_before_collecting() -> Result<(), Box<dyn std::error::Error>> {
    let temp = tempfile::tempdir()?;
    let output_path = sentinel_output(temp.path())?;
    let copy = temp.path().join("validation.html");

    let (output, log_path) = run_export(
        temp.path(),
        &output_path,
        &[
            OsString::from("--validate-copy"),
            copy.clone().into_os_string(),
            OsString::from("--validate-concurrency"),
            OsString::from("0"),
        ],
    )?;

    assert!(!output.status.success());
    assert!(diagnostic(&output).contains("value must be at least 1"));
    assert!(!copy.exists());
    assert_eq!(fs::read(&output_path)?, b"preserve this output\n");
    assert!(!log_path.exists());
    Ok(())
}
