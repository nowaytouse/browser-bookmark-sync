//! Download history extraction
//!
//! Supports Chromium and Firefox browsers.

use anyhow::Result;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Download entry from browser
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Download {
    pub url: String,
    pub filename: String,
    pub target_path: String,
    pub total_bytes: i64,
    pub start_time: i64,
    pub end_time: i64,
    pub state: String,
    pub browser: String,
}

/// Extract downloads from Chromium-based browser
pub fn extract_chromium_downloads(db_path: &Path, browser: &str) -> Result<Vec<Download>> {
    let conn = Connection::open(db_path)?;

    let mut stmt = conn.prepare(
        "SELECT tab_url, target_path, total_bytes, start_time, end_time, state 
         FROM downloads",
    )?;

    let mut downloads = Vec::new();

    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, i64>(2)?,
            row.get::<_, i64>(3)?,
            row.get::<_, i64>(4).unwrap_or(0),
            row.get::<_, i32>(5)?,
        ))
    })?;

    for row in rows {
        let (url, target_path, total_bytes, start_time, end_time, state) = row?;

        let filename = Path::new(&target_path)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();

        let state_str = match state {
            0 => "In Progress",
            1 => "Complete",
            2 => "Cancelled",
            3 => "Interrupted",
            _ => "Unknown",
        };

        downloads.push(Download {
            url,
            filename,
            target_path,
            total_bytes,
            start_time,
            end_time,
            state: state_str.to_string(),
            browser: browser.to_string(),
        });
    }

    downloads.sort_by(|a, b| b.start_time.cmp(&a.start_time));

    Ok(downloads)
}

/// Extract downloads from Firefox
#[allow(dead_code)]
pub fn extract_firefox_downloads(db_path: &Path) -> Result<Vec<Download>> {
    let conn = Connection::open(db_path)?;

    // Firefox stores downloads in places.sqlite with moz_annos
    let mut stmt = conn.prepare(
        "SELECT p.url, a.content
         FROM moz_places p
         JOIN moz_annos a ON p.id = a.place_id
         WHERE a.anno_attribute_id IN (SELECT id FROM moz_anno_attributes WHERE name = 'downloads/destinationFileURI')"
    )?;

    let mut downloads = Vec::new();

    let rows = stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    });

    if let Ok(rows) = rows {
        for (url, dest_uri) in rows.flatten() {
            let filename = dest_uri
                .replace("file://", "")
                .split('/')
                .next_back()
                .unwrap_or("")
                .to_string();

            downloads.push(Download {
                url,
                filename: filename.clone(),
                target_path: dest_uri,
                total_bytes: 0,
                start_time: 0,
                end_time: 0,
                state: "Complete".to_string(),
                browser: "Firefox".to_string(),
            });
        }
    }

    Ok(downloads)
}

/// Export downloads to CSV format
pub fn export_to_csv(downloads: &[Download], output_path: &Path) -> Result<()> {
    use std::io::Write;

    let mut file = std::fs::File::create(output_path)?;

    writeln!(file, "url,filename,size_bytes,start_time,state,browser")?;

    for dl in downloads {
        writeln!(
            file,
            "\"{}\",\"{}\",{},{},\"{}\",\"{}\"",
            dl.url.replace('"', "\"\""),
            dl.filename.replace('"', "\"\""),
            dl.total_bytes,
            dl.start_time,
            dl.state,
            dl.browser
        )?;
    }

    Ok(())
}
