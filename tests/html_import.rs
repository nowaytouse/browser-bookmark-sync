use browser_bookmark_sync::{
    browsers::Bookmark,
    sync::{export_bookmarks_to_html, import_bookmarks_from_html},
    url_checker::collect_urls_from_bookmarks,
};

#[test]
fn ordinary_text_is_not_an_empty_bookmark_export() -> Result<(), Box<dyn std::error::Error>> {
    let temp = tempfile::tempdir()?;
    let input = temp.path().join("input.html");
    std::fs::write(&input, "this is not a bookmark export")?;

    assert!(import_bookmarks_from_html(&input.to_string_lossy()).is_err());
    Ok(())
}

#[test]
fn partially_parsed_html_is_rejected_instead_of_losing_links(
) -> Result<(), Box<dyn std::error::Error>> {
    let temp = tempfile::tempdir()?;
    let input = temp.path().join("input.html");
    std::fs::write(
        &input,
        "<DL>\n<DT><A HREF=\"https://example.com/one\">One</A><DT><A HREF=\"https://example.com/two\">Two</A>\n</DL>",
    )?;

    assert!(import_bookmarks_from_html(&input.to_string_lossy()).is_err());
    Ok(())
}

#[test]
fn unsupported_url_entities_are_rejected_instead_of_changing_the_url(
) -> Result<(), Box<dyn std::error::Error>> {
    let temp = tempfile::tempdir()?;
    let input = temp.path().join("input.html");
    std::fs::write(
        &input,
        "<DL>\n<DT><A HREF=\"https://example.com/?x=1&#38;y=2\">Example</A>\n</DL>",
    )?;

    assert!(import_bookmarks_from_html(&input.to_string_lossy()).is_err());
    Ok(())
}

#[test]
fn unicode_attributes_do_not_shift_url_byte_offsets() -> Result<(), Box<dyn std::error::Error>> {
    let temp = tempfile::tempdir()?;
    let input = temp.path().join("input.html");
    std::fs::write(
        &input,
        "<DL>\n<DT><A DATA-NOTE=\"İ\" HREF=\"https://example.com/\">Example</A>\n</DL>",
    )?;

    let bookmarks = import_bookmarks_from_html(&input.to_string_lossy())?;
    assert_eq!(
        collect_urls_from_bookmarks(&bookmarks),
        ["https://example.com/"]
    );
    Ok(())
}

#[test]
fn oversized_bookmark_dates_do_not_panic() -> Result<(), Box<dyn std::error::Error>> {
    let temp = tempfile::tempdir()?;
    let input = temp.path().join("input.html");
    std::fs::write(
        &input,
        "<DL>\n<DT><A HREF=\"https://example.com/\" ADD_DATE=\"9223372036854775807\">Example</A>\n</DL>",
    )?;

    let bookmarks = import_bookmarks_from_html(&input.to_string_lossy())?;
    assert_eq!(
        collect_urls_from_bookmarks(&bookmarks),
        ["https://example.com/"]
    );
    assert_eq!(bookmarks[0].date_added, None);
    Ok(())
}

#[test]
fn unsupported_folder_title_entities_are_rejected_instead_of_changed(
) -> Result<(), Box<dyn std::error::Error>> {
    let temp = tempfile::tempdir()?;
    let input = temp.path().join("input.html");
    std::fs::write(
        &input,
        "<DL>\n<DT><H3>Fish &copy; Docs</H3>\n<DL>\n</DL>\n</DL>",
    )?;

    assert!(import_bookmarks_from_html(&input.to_string_lossy()).is_err());
    Ok(())
}

#[test]
fn explicitly_empty_bookmark_export_is_accepted() -> Result<(), Box<dyn std::error::Error>> {
    let temp = tempfile::tempdir()?;
    let input = temp.path().join("input.html");
    std::fs::write(
        &input,
        "<!DOCTYPE NETSCAPE-Bookmark-file-1>\n<DL><p>\n</DL><p>\n",
    )?;

    assert!(import_bookmarks_from_html(&input.to_string_lossy())?.is_empty());
    Ok(())
}

#[test]
fn generated_html_round_trip_preserves_nested_bookmarks() -> Result<(), Box<dyn std::error::Error>>
{
    let temp = tempfile::tempdir()?;
    let output = temp.path().join("round-trip.html");
    let leaf = Bookmark {
        id: "leaf".into(),
        title: "中文 & reading".into(),
        url: Some("https://example.com/?a=1&b=2".into()),
        folder: false,
        children: vec![],
        date_added: Some(1700000000000),
        date_modified: None,
    };
    let folder = Bookmark {
        id: "folder".into(),
        title: "阅读 & 工作".into(),
        url: None,
        folder: true,
        children: vec![leaf],
        date_added: Some(1700000000000),
        date_modified: None,
    };
    export_bookmarks_to_html(&[folder], &output.to_string_lossy())?;

    let bookmarks = import_bookmarks_from_html(&output.to_string_lossy())?;
    assert_eq!(bookmarks.len(), 1);
    assert!(bookmarks[0].folder);
    assert_eq!(bookmarks[0].title, "阅读 & 工作");
    assert_eq!(bookmarks[0].children[0].title, "中文 & reading");
    assert_eq!(
        collect_urls_from_bookmarks(&bookmarks),
        ["https://example.com/?a=1&b=2"]
    );
    Ok(())
}
