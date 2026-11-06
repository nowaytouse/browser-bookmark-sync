use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};

const SENTINEL: &[u8] = b"preserve this output\n";

fn run_merge(directory: &Path, result_dir: &Path, output_path: &Path) -> std::io::Result<Output> {
    Command::new(env!("CARGO_BIN_EXE_browser-bookmark-sync"))
        .current_dir(directory)
        .arg("--log-file")
        .arg(directory.join("cli.log"))
        .arg("merge-results")
        .arg("--dir")
        .arg(result_dir)
        .arg("--output")
        .arg(output_path)
        .output()
}

fn sentinel_output(directory: &Path) -> std::io::Result<PathBuf> {
    let path = directory.join("existing-output.html");
    fs::write(&path, SENTINEL)?;
    Ok(path)
}

fn valid_html() -> &'static [u8] {
    b"<!DOCTYPE NETSCAPE-Bookmark-file-1>\n<DL><p>\n<DT><A HREF=\"https://example.com/\">Example</A>\n</DL><p>\n"
}

#[test]
fn empty_result_directory_fails_and_preserves_existing_output(
) -> Result<(), Box<dyn std::error::Error>> {
    let temp = tempfile::tempdir()?;
    let result_dir = temp.path().join("results");
    fs::create_dir(&result_dir)?;
    let output_path = sentinel_output(temp.path())?;

    let output = run_merge(temp.path(), &result_dir, &output_path)?;

    assert!(!output.status.success());
    assert_eq!(fs::read(&output_path)?, SENTINEL);
    Ok(())
}

#[test]
fn unreadable_result_partition_fails_and_preserves_existing_output(
) -> Result<(), Box<dyn std::error::Error>> {
    let temp = tempfile::tempdir()?;
    let result_dir = temp.path().join("results");
    fs::create_dir(&result_dir)?;
    fs::write(result_dir.join("valid.html"), valid_html())?;
    fs::write(result_dir.join("invalid.html"), [0xff, 0xfe])?;
    let output_path = sentinel_output(temp.path())?;

    let output = run_merge(temp.path(), &result_dir, &output_path)?;

    assert!(!output.status.success());
    assert_eq!(fs::read(&output_path)?, SENTINEL);
    Ok(())
}

#[test]
fn output_equal_to_result_partition_fails_without_modifying_input(
) -> Result<(), Box<dyn std::error::Error>> {
    let temp = tempfile::tempdir()?;
    let result_dir = temp.path().join("results");
    fs::create_dir(&result_dir)?;
    let valid_path = result_dir.join("valid.html");
    fs::write(&valid_path, valid_html())?;
    let original = fs::read(&valid_path)?;

    let output = run_merge(temp.path(), &result_dir, &valid_path)?;

    assert!(!output.status.success());
    assert_eq!(fs::read(&valid_path)?, original);
    Ok(())
}

#[cfg(unix)]
#[test]
fn symlink_output_to_result_partition_is_rejected_and_preserved(
) -> Result<(), Box<dyn std::error::Error>> {
    use std::os::unix::fs::symlink;

    let temp = tempfile::tempdir()?;
    let result_dir = temp.path().join("results");
    fs::create_dir(&result_dir)?;
    let valid_path = result_dir.join("valid.html");
    fs::write(&valid_path, valid_html())?;
    let original = fs::read(&valid_path)?;
    let output_path = temp.path().join("output-link.html");
    symlink(&valid_path, &output_path)?;

    let output = run_merge(temp.path(), &result_dir, &output_path)?;

    assert!(!output.status.success());
    assert!(fs::symlink_metadata(&output_path)?.file_type().is_symlink());
    assert_eq!(fs::read_link(&output_path)?, valid_path);
    assert_eq!(fs::read(&valid_path)?, original);
    Ok(())
}

#[test]
fn valid_partial_result_directory_merges_bookmarks() -> Result<(), Box<dyn std::error::Error>> {
    let temp = tempfile::tempdir()?;
    let result_dir = temp.path().join("results");
    fs::create_dir(&result_dir)?;
    fs::write(result_dir.join("valid.html"), valid_html())?;
    let output_path = temp.path().join("merged.html");

    let output = run_merge(temp.path(), &result_dir, &output_path)?;

    assert!(output.status.success());
    let merged = fs::read_to_string(&output_path)?;
    assert!(merged.contains("https://example.com/"));
    assert!(merged.contains("Example"));
    Ok(())
}

#[cfg(unix)]
#[test]
fn broken_partition_symlink_fails_instead_of_being_treated_as_absent(
) -> Result<(), Box<dyn std::error::Error>> {
    use std::os::unix::fs::symlink;

    let temp = tempfile::tempdir()?;
    let result_dir = temp.path().join("results");
    fs::create_dir(&result_dir)?;
    fs::write(result_dir.join("valid.html"), valid_html())?;
    symlink(
        result_dir.join("missing.html"),
        result_dir.join("invalid.html"),
    )?;
    let output_path = sentinel_output(temp.path())?;

    let output = run_merge(temp.path(), &result_dir, &output_path)?;

    assert!(!output.status.success());
    assert_eq!(fs::read(&output_path)?, SENTINEL);
    Ok(())
}
