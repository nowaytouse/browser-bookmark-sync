use serde::Deserialize;
use std::sync::LazyLock;

const ENGLISH: &str = include_str!("../../assets/locales/en.json");
const SIMPLIFIED_CHINESE: &str = include_str!("../../assets/locales/zh-Hans.json");

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct EntryCopy {
    pub title: Box<str>,
    pub export_html: Box<str>,
    pub settings: Box<str>,
    pub import: Box<str>,
    pub html_file: Box<str>,
    pub import_browsers: Box<str>,
    pub open_batch: Box<str>,
    pub welcome_title: Box<str>,
    pub welcome_description: Box<str>,
    pub choose_source: Box<str>,
    pub import_html: Box<str>,
    pub drop_hint: Box<str>,
    pub source_report_hint: Box<str>,
    pub export_scope_hint: Box<str>,
    pub try_sample: Box<str>,
    pub sample_hint: Box<str>,
    pub sample_loaded: Box<str>,
    pub workflow: Box<str>,
}

pub(super) fn entry_copy(zh: bool) -> anyhow::Result<&'static EntryCopy> {
    static EN: LazyLock<serde_json::Result<EntryCopy>> =
        LazyLock::new(|| serde_json::from_str(ENGLISH));
    static ZH: LazyLock<serde_json::Result<EntryCopy>> =
        LazyLock::new(|| serde_json::from_str(SIMPLIFIED_CHINESE));
    let (language, catalog) = if zh { ("zh-Hans", &*ZH) } else { ("en", &*EN) };
    catalog
        .as_ref()
        .map_err(|error| anyhow::anyhow!("Invalid bundled {language} locale: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entry_catalogs_have_matching_nonempty_labels() {
        for (zh, text) in [(false, ENGLISH), (true, SIMPLIFIED_CHINESE)] {
            let entries = serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(text)
                .expect("valid locale object");
            assert!(entries
                .values()
                .all(|value| value.as_str().is_some_and(|label| !label.trim().is_empty())));
            assert_eq!(
                entry_copy(zh).unwrap().title.as_ref(),
                entries["title"].as_str().unwrap()
            );
        }
        assert_ne!(
            entry_copy(false).unwrap().title,
            entry_copy(true).unwrap().title
        );
    }
}
