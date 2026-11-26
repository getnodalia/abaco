// SPDX-License-Identifier: MIT OR Apache-2.0
//! The preferences file: `settings.json` in `ABACO_CONFIG_DIR`, or in the platform's config
//! folder. A missing, unreadable or corrupt file gives the defaults; it never stops the app.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Most recent files remembered.
pub const MAX_RECENT: usize = 8;
/// Largest settings file read, in bytes.
const MAX_BYTES: u64 = 256 * 1024;

/// Light, dark or whatever the desktop uses.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Appearance {
    #[default]
    System,
    Light,
    Dark,
}

/// The sections of the side panel.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SidePanel {
    #[default]
    Units,
    Document,
}

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub appearance: Appearance,
    pub high_contrast: bool,
    pub reduced_motion: bool,
    /// Linux only: use the desktop's own window buttons instead of NDL's.
    pub native_window_controls: bool,
    /// `auto`, `es` or `en`.
    pub language: String,
    /// Significant digits for a new document (1 to 17).
    pub significant_digits: usize,
    pub show_navigator: bool,
    pub show_side_panel: bool,
    pub side_panel: SidePanel,
    pub recent: Vec<PathBuf>,
    /// The first launch opens the sample; once it has been shown this is `true`.
    pub sample_shown: bool,
    /// The last document that was open, to come back to.
    pub last_file: Option<PathBuf>,
}

impl Default for Settings {
    fn default() -> Settings {
        Settings {
            appearance: Appearance::System,
            high_contrast: false,
            reduced_motion: false,
            native_window_controls: false,
            language: "auto".to_owned(),
            significant_digits: abaco_doc::DEFAULT_SIGNIFICANT_DIGITS,
            show_navigator: true,
            show_side_panel: true,
            side_panel: SidePanel::Units,
            recent: Vec::new(),
            sample_shown: false,
            last_file: None,
        }
    }
}

impl Settings {
    /// Where the file lives, when there is a home to put it in.
    pub fn file() -> Option<PathBuf> {
        config_dir().map(|d| d.join("settings.json"))
    }

    /// Reads the settings, or the defaults when there are none or they cannot be read.
    pub fn load() -> Settings {
        Settings::file()
            .map(|p| Settings::load_from(&p))
            .unwrap_or_default()
    }

    pub fn load_from(path: &Path) -> Settings {
        let Ok(meta) = std::fs::metadata(path) else {
            return Settings::default();
        };
        if meta.len() > MAX_BYTES {
            return Settings::default();
        }
        let Ok(text) = std::fs::read_to_string(path) else {
            return Settings::default();
        };
        serde_json::from_str::<Settings>(&text)
            .map(Settings::sanitized)
            .unwrap_or_default()
    }

    /// Writes the file atomically (beside it, then renamed over it).
    pub fn save_to(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let text = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, text)?;
        std::fs::rename(&tmp, path).inspect_err(|_| {
            let _ = std::fs::remove_file(&tmp);
        })
    }

    pub fn save(&self) -> std::io::Result<()> {
        match Settings::file() {
            Some(path) => self.save_to(&path),
            None => Ok(()),
        }
    }

    /// Puts every value back in its range: a hand-edited file may hold anything.
    pub fn sanitized(mut self) -> Settings {
        if !(1..=17).contains(&self.significant_digits) {
            self.significant_digits = abaco_doc::DEFAULT_SIGNIFICANT_DIGITS;
        }
        if !matches!(self.language.as_str(), "auto" | "es" | "en") {
            self.language = "auto".to_owned();
        }
        self.recent.truncate(MAX_RECENT);
        self
    }

    /// Moves `path` to the front of the recent files.
    pub fn remember(&mut self, path: &Path) {
        self.recent.retain(|p| p != path);
        self.recent.insert(0, path.to_path_buf());
        self.recent.truncate(MAX_RECENT);
        self.last_file = Some(path.to_path_buf());
    }

    /// Forgets a file that no longer opens.
    pub fn forget(&mut self, path: &Path) {
        self.recent.retain(|p| p != path);
        if self.last_file.as_deref() == Some(path) {
            self.last_file = None;
        }
    }
}

fn config_dir() -> Option<PathBuf> {
    let env = |name: &str| std::env::var_os(name).filter(|v| !v.is_empty());
    if let Some(dir) = env("ABACO_CONFIG_DIR") {
        return Some(PathBuf::from(dir));
    }
    if cfg!(target_os = "windows") {
        return env("APPDATA").map(|d| PathBuf::from(d).join("Abaco"));
    }
    if cfg!(target_os = "macos") {
        return env("HOME").map(|h| PathBuf::from(h).join("Library/Application Support/Abaco"));
    }
    env("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| env("HOME").map(|h| PathBuf::from(h).join(".config")))
        .map(|d| d.join("abaco"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_round_trip_through_a_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested/settings.json");
        let mut s = Settings {
            appearance: Appearance::Dark,
            ..Settings::default()
        };
        s.remember(Path::new("/tmp/a.abaco"));
        s.save_to(&path).unwrap();
        assert_eq!(Settings::load_from(&path), s);
    }

    #[test]
    fn a_missing_or_corrupt_file_gives_the_defaults() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            Settings::load_from(&dir.path().join("none.json")),
            Settings::default()
        );
        let bad = dir.path().join("bad.json");
        std::fs::write(&bad, "{ not json").unwrap();
        assert_eq!(Settings::load_from(&bad), Settings::default());
    }

    #[test]
    fn unknown_fields_and_missing_fields_are_tolerated() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s.json");
        std::fs::write(&path, r#"{"appearance":"dark","future":1}"#).unwrap();
        let s = Settings::load_from(&path);
        assert_eq!(s.appearance, Appearance::Dark);
        assert_eq!(s.significant_digits, 6);
    }

    #[test]
    fn out_of_range_values_are_put_back() {
        let s = Settings {
            significant_digits: 99,
            language: "xx".into(),
            ..Settings::default()
        }
        .sanitized();
        assert_eq!(s.significant_digits, 6);
        assert_eq!(s.language, "auto");
    }

    #[test]
    fn recent_files_are_unique_newest_first_and_capped() {
        let mut s = Settings::default();
        for i in 0..20 {
            s.remember(Path::new(&format!("/tmp/{i}.abaco")));
        }
        s.remember(Path::new("/tmp/5.abaco"));
        assert_eq!(s.recent.len(), MAX_RECENT);
        assert_eq!(s.recent[0], Path::new("/tmp/5.abaco"));
        assert_eq!(
            s.recent
                .iter()
                .filter(|p| *p == Path::new("/tmp/5.abaco"))
                .count(),
            1
        );
        s.forget(Path::new("/tmp/5.abaco"));
        assert!(s.last_file.is_none());
        assert!(!s.recent.contains(&PathBuf::from("/tmp/5.abaco")));
    }
}
