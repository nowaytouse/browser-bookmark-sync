//! Local display preferences, deliberately separate from bookmark data and paths.

use std::env;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tempfile::NamedTempFile;

/// Smallest supported text multiplier.
pub const MIN_TEXT_SCALE: f32 = 0.9;
/// Largest supported text multiplier.
pub const MAX_TEXT_SCALE: f32 = 1.3;

const MAX_PREFERENCES_BYTES: u64 = 16 * 1024;
const APP_DIRECTORY: &str = "browser-bookmark-sync";
const PREFERENCES_FILE: &str = "native-review.json";

/// Preferred native window color scheme.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThemeChoice {
    /// Follow the operating system color scheme.
    #[default]
    System,
    /// Use a light color scheme.
    Light,
    /// Use a dark color scheme.
    Dark,
}

/// Spacing preference for the native review window.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DisplayDensity {
    /// Keep generous spacing and larger row targets.
    #[default]
    Comfortable,
    /// Fit more rows into the same window.
    Compact,
}

/// Language used for native window labels.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LanguageChoice {
    /// Follow the operating system's preferred language.
    #[default]
    System,
    /// Use English labels.
    English,
    /// Use Simplified Chinese labels.
    SimplifiedChinese,
}

impl LanguageChoice {
    /// Resolve this selection without repeatedly querying operating-system settings.
    pub fn prefers_chinese(self) -> bool {
        match self {
            Self::System => system_prefers_chinese(),
            Self::English => false,
            Self::SimplifiedChinese => true,
        }
    }
}

/// Accent palette used independently of the light or dark color scheme.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccentChoice {
    /// Blue accent palette.
    #[default]
    Blue,
    /// Teal accent palette.
    Teal,
    /// Neutral black or white accent palette.
    Monochrome,
}

/// Persisted display settings only; never add bookmark contents or source paths.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct NativePreferences {
    /// Native window color scheme.
    pub theme: ThemeChoice,
    /// Native window spacing.
    pub density: DisplayDensity,
    /// Language used for native window labels.
    pub language: LanguageChoice,
    /// Accent palette used in either color scheme.
    pub accent: AccentChoice,
    /// Text multiplier, normalized when loading and saving.
    pub text_scale: f32,
    /// Whether to show the selected bookmark's details panel.
    pub show_details: bool,
}

impl Default for NativePreferences {
    fn default() -> Self {
        Self {
            theme: ThemeChoice::System,
            density: DisplayDensity::Comfortable,
            language: LanguageChoice::System,
            accent: AccentChoice::Blue,
            text_scale: 1.0,
            show_details: true,
        }
    }
}

impl NativePreferences {
    /// Clamp finite scales to the supported range and reset non-finite scales.
    pub fn normalized(mut self) -> Self {
        self.text_scale = if self.text_scale.is_finite() {
            self.text_scale.clamp(MIN_TEXT_SCALE, MAX_TEXT_SCALE)
        } else {
            Self::default().text_scale
        };
        self
    }
}

/// A preferences failure that callers should surface without stopping review.
#[derive(Debug, thiserror::Error)]
pub enum PreferencesError {
    /// No suitable absolute per-user configuration directory is available.
    #[error("The local display preferences directory is unavailable")]
    ConfigurationDirectoryUnavailable,
    /// An operating-system file operation failed.
    #[error("Could not {operation} display preferences: {source}")]
    Io {
        /// File operation that failed.
        operation: &'static str,
        /// Original operating-system failure.
        #[source]
        source: io::Error,
    },
    /// Settings are not valid JSON or do not match the display-only schema.
    #[error("Display preferences are invalid; default settings are in use: {0}")]
    InvalidConfiguration(#[source] serde_json::Error),
    /// A settings file unexpectedly exceeds the small display-only limit.
    #[error("Display preferences are too large; default settings are in use")]
    TooLarge,
}

/// Preferences plus a recoverable diagnostic; missing files have no diagnostic.
#[derive(Debug)]
#[must_use = "surface the error when default preferences replace unreadable settings"]
pub struct PreferencesLoad {
    /// Loaded settings, or defaults when the file cannot be read or parsed.
    pub preferences: NativePreferences,
    /// An explicit error to display or log; never silently discard this field.
    pub error: Option<PreferencesError>,
}

/// A local display-settings file, independent of all bookmark storage.
#[derive(Debug, Clone)]
pub struct PreferencesStore {
    path: PathBuf,
}

impl PreferencesStore {
    /// Resolve the operating system's per-user local configuration location.
    pub fn local() -> Result<Self, PreferencesError> {
        #[cfg(target_os = "macos")]
        let directory =
            absolute_environment_path("HOME").map(|home| home.join("Library/Application Support"));

        #[cfg(target_os = "windows")]
        let directory = absolute_environment_path("LOCALAPPDATA")
            .or_else(|| absolute_environment_path("APPDATA"));

        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        let directory = absolute_environment_path("XDG_CONFIG_HOME")
            .or_else(|| absolute_environment_path("HOME").map(|home| home.join(".config")));

        let directory = directory.ok_or(PreferencesError::ConfigurationDirectoryUnavailable)?;
        Ok(Self {
            path: directory.join(APP_DIRECTORY).join(PREFERENCES_FILE),
        })
    }

    /// The display-settings file location; this path is not stored in its JSON.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Load settings, with clean defaults for absence and an explicit error otherwise.
    pub fn load(&self) -> PreferencesLoad {
        match self.read() {
            Ok(preferences) => PreferencesLoad {
                preferences,
                error: None,
            },
            Err(error) => PreferencesLoad {
                preferences: NativePreferences::default(),
                error: Some(error),
            },
        }
    }

    fn read(&self) -> Result<NativePreferences, PreferencesError> {
        let file = match File::open(&self.path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Ok(NativePreferences::default());
            }
            Err(source) => {
                return Err(PreferencesError::Io {
                    operation: "read",
                    source,
                });
            }
        };
        let mut bytes = Vec::new();
        file.take(MAX_PREFERENCES_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|source| PreferencesError::Io {
                operation: "read",
                source,
            })?;
        if bytes.len() as u64 > MAX_PREFERENCES_BYTES {
            return Err(PreferencesError::TooLarge);
        }
        serde_json::from_slice::<NativePreferences>(&bytes)
            .map(NativePreferences::normalized)
            .map_err(PreferencesError::InvalidConfiguration)
    }

    /// Atomically replace normalized settings using a same-directory temporary file.
    /// On Unix the app directory and settings file are private (0700 and 0600).
    pub fn save(&self, preferences: &NativePreferences) -> Result<(), PreferencesError> {
        let bytes = serde_json::to_vec_pretty(&preferences.normalized())
            .map_err(PreferencesError::InvalidConfiguration)?;
        let parent = self
            .path
            .parent()
            .ok_or(PreferencesError::ConfigurationDirectoryUnavailable)?;
        self.write_atomic(parent, &bytes)
            .map_err(|source| PreferencesError::Io {
                operation: "save",
                source,
            })
    }

    fn write_atomic(&self, parent: &Path, bytes: &[u8]) -> io::Result<()> {
        fs::create_dir_all(parent)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(parent, fs::Permissions::from_mode(0o700))?;
        }
        let mut temporary = NamedTempFile::new_in(parent)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            temporary
                .as_file()
                .set_permissions(fs::Permissions::from_mode(0o600))?;
        }
        temporary.write_all(bytes)?;
        temporary.write_all(b"\n")?;
        temporary.as_file().sync_all()?;
        temporary.persist(&self.path).map_err(|error| error.error)?;
        Ok(())
    }
}

fn absolute_environment_path(variable: &str) -> Option<PathBuf> {
    env::var_os(variable)
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
}

/// Detect the preferred system language once per application launch.
/// Explicit language selections do not depend on this platform-specific fallback.
pub fn system_prefers_chinese() -> bool {
    static CHINESE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *CHINESE.get_or_init(|| {
        #[cfg(target_os = "macos")]
        if let Some(chinese) = std::process::Command::new("/usr/bin/defaults")
            .args(["read", "-g", "AppleLanguages"])
            .output()
            .ok()
            .filter(|output| {
                output.status.success() && output.stdout.len() <= MAX_PREFERENCES_BYTES as usize
            })
            .and_then(|output| {
                std::str::from_utf8(&output.stdout)
                    .ok()
                    .and_then(first_apple_language_prefers_chinese)
            })
        {
            return chinese;
        }

        ["LC_ALL", "LC_MESSAGES", "LANG"]
            .iter()
            .find_map(|variable| env::var(variable).ok().filter(|value| !value.is_empty()))
            .is_some_and(|locale| locale.to_ascii_lowercase().starts_with("zh"))
    })
}

#[cfg(any(target_os = "macos", test))]
fn first_apple_language_prefers_chinese(languages: &str) -> Option<bool> {
    languages
        .trim()
        .trim_start_matches('(')
        .lines()
        .map(|line| line.trim().trim_end_matches(',').trim_matches('"'))
        .find(|language| !language.is_empty() && *language != ")")
        .map(|language| language.to_ascii_lowercase().starts_with("zh"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_settings_use_defaults_without_an_error() {
        let directory = tempfile::tempdir().unwrap();
        let store = PreferencesStore {
            path: directory.path().join("missing.json"),
        };
        let loaded = store.load();
        assert_eq!(loaded.preferences, NativePreferences::default());
        assert!(loaded.error.is_none());
    }

    #[test]
    fn display_settings_round_trip_and_replace_previous_settings() {
        let directory = tempfile::tempdir().unwrap();
        let store = PreferencesStore {
            path: directory.path().join("app/preferences.json"),
        };
        store.save(&NativePreferences::default()).unwrap();
        let preferences = NativePreferences {
            theme: ThemeChoice::Dark,
            density: DisplayDensity::Compact,
            language: LanguageChoice::SimplifiedChinese,
            accent: AccentChoice::Teal,
            text_scale: 1.2,
            show_details: false,
        };
        store.save(&preferences).unwrap();
        let loaded = store.load();
        assert_eq!(loaded.preferences, preferences);
        assert!(loaded.error.is_none());
    }

    #[test]
    fn older_display_settings_default_new_language_and_accent_fields() {
        let preferences = serde_json::from_str::<NativePreferences>(
            r#"{"theme":"dark","density":"compact","text_scale":1.1,"show_details":false}"#,
        )
        .unwrap();
        assert_eq!(preferences.theme, ThemeChoice::Dark);
        assert_eq!(preferences.language, LanguageChoice::System);
        assert_eq!(preferences.accent, AccentChoice::Blue);
    }

    #[test]
    fn explicit_language_and_monochrome_accent_round_trip() {
        let preferences = NativePreferences {
            language: LanguageChoice::English,
            accent: AccentChoice::Monochrome,
            ..NativePreferences::default()
        };
        let json = serde_json::to_string(&preferences).unwrap();
        assert_eq!(
            serde_json::from_str::<NativePreferences>(&json).unwrap(),
            preferences
        );
        assert!(!LanguageChoice::English.prefers_chinese());
        assert!(LanguageChoice::SimplifiedChinese.prefers_chinese());
    }

    #[test]
    fn apple_language_detection_uses_first_preferred_language_only() {
        assert_eq!(
            first_apple_language_prefers_chinese("(\n    \"zh-Hans-CN\",\n    \"en-US\"\n)"),
            Some(true)
        );
        assert_eq!(
            first_apple_language_prefers_chinese("(\n    \"en-US\",\n    \"zh-Hans-CN\"\n)"),
            Some(false)
        );
        assert_eq!(first_apple_language_prefers_chinese("(\n)"), None);
    }

    #[test]
    fn malformed_settings_keep_defaults_and_report_the_error() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("preferences.json");
        fs::write(&path, b"{not valid json}").unwrap();
        let loaded = PreferencesStore { path }.load();
        assert_eq!(loaded.preferences, NativePreferences::default());
        assert!(matches!(
            loaded.error,
            Some(PreferencesError::InvalidConfiguration(_))
        ));
    }

    #[test]
    fn load_and_save_clamp_scales_to_supported_bounds() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("preferences.json");
        fs::write(&path, br#"{"text_scale": 0.1}"#).unwrap();
        let store = PreferencesStore { path };
        assert_eq!(store.load().preferences.text_scale, MIN_TEXT_SCALE);
        store
            .save(&NativePreferences {
                text_scale: 9.0,
                ..NativePreferences::default()
            })
            .unwrap();
        assert_eq!(store.load().preferences.text_scale, MAX_TEXT_SCALE);
        assert_eq!(
            NativePreferences {
                text_scale: f32::NAN,
                ..NativePreferences::default()
            }
            .normalized()
            .text_scale,
            1.0
        );
    }

    #[test]
    fn settings_schema_rejects_non_display_fields() {
        assert!(serde_json::from_str::<NativePreferences>(
            r#"{"bookmark_path":"must-not-be-persisted"}"#
        )
        .is_err());
    }

    #[cfg(unix)]
    #[test]
    fn saved_settings_and_app_directory_have_private_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let directory = tempfile::tempdir().unwrap();
        let store = PreferencesStore {
            path: directory.path().join("app/preferences.json"),
        };
        store.save(&NativePreferences::default()).unwrap();
        assert_eq!(
            fs::metadata(store.path()).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            fs::metadata(store.path().parent().unwrap())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
    }
}
