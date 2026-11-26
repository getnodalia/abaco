// SPDX-License-Identifier: MIT OR Apache-2.0
//! Interface text in Spanish and English.
//!
//! The source is written in English, through [`tr`]; Spanish comes from [`SPANISH`]. A test reads
//! every `tr("…")` in `src/` and every command label and fails when one has no Spanish entry, so a
//! new string cannot ship untranslated. Text with a value in it uses `{}` placeholders filled by
//! [`trf`].

use std::cell::Cell;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lang {
    Es,
    En,
}

impl Lang {
    /// The system language when it is Spanish, English otherwise. macOS and Windows keep the
    /// language in their own settings (an app opened from the Dock has no `LANG`), so ask them;
    /// Linux and the BSDs say it in the environment.
    pub fn system() -> Lang {
        if system_tag().is_some_and(|tag| is_spanish(&tag)) {
            Lang::Es
        } else {
            Lang::En
        }
    }

    /// The language a settings value names: `es`, `en`, or `auto` for the system's.
    pub fn from_setting(value: &str) -> Lang {
        match value {
            "es" => Lang::Es,
            "en" => Lang::En,
            _ => Lang::system(),
        }
    }
}

fn system_tag() -> Option<String> {
    let platform = || sys_locale::get_locale().filter(|tag| !tag.is_empty() && tag.len() <= 64);
    if cfg!(any(target_os = "macos", target_os = "windows")) {
        return platform();
    }
    ["LC_ALL", "LC_MESSAGES", "LANG", "LANGUAGE"]
        .iter()
        .filter_map(|v| std::env::var(v).ok())
        .find(|v| !v.is_empty())
        .or_else(platform)
}

/// Whether a locale tag (`es`, `es-ES`, `es_MX.UTF-8`, `es-419`) names Spanish.
fn is_spanish(tag: &str) -> bool {
    let tag = tag.trim().to_ascii_lowercase();
    tag.strip_prefix("es")
        .is_some_and(|rest| rest.is_empty() || rest.starts_with(['-', '_', '.', '@']))
}

thread_local! {
    // Per thread: the window and everything that draws share the main thread, and tests, each on
    // a thread of its own, cannot change one another's language.
    static CURRENT: Cell<Lang> = const { Cell::new(Lang::Es) };
}

/// Sets the language of the calling thread (the window's).
pub fn set(lang: Lang) {
    CURRENT.with(|c| c.set(lang));
}

pub fn current() -> Lang {
    CURRENT.with(Cell::get)
}

/// The text in the current language.
pub fn tr(english: &'static str) -> &'static str {
    translate(english)
}

/// [`tr`] for a text that is not a literal (a unit's name, a dimension): the English text itself
/// when the table has nothing for it.
pub fn translate(english: &str) -> &str {
    if current() == Lang::En {
        return english;
    }
    SPANISH
        .binary_search_by(|(en, _)| (*en).cmp(english))
        .ok()
        .and_then(|i| SPANISH.get(i))
        .map_or(english, |(_, es)| es)
}

/// The Spanish text for `english`, `None` when the table has no row for it.
pub fn translate_known(english: &str) -> Option<&'static str> {
    SPANISH
        .binary_search_by(|(en, _)| (*en).cmp(english))
        .ok()
        .and_then(|i| SPANISH.get(i))
        .map(|(_, es)| *es)
}

/// [`tr`] with each `{}` replaced, in order, by `args`.
pub fn trf(english: &'static str, args: &[&str]) -> String {
    let mut out = String::new();
    let mut rest = tr(english);
    for arg in args {
        match rest.split_once("{}") {
            Some((before, after)) => {
                out.push_str(before);
                out.push_str(arg);
                rest = after;
            }
            None => break,
        }
    }
    out.push_str(rest);
    out
}

/// English → Spanish, sorted by the English text (the lookup is a binary search).
pub use crate::spanish::SPANISH;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_is_sorted_and_has_no_duplicates() {
        for pair in SPANISH.windows(2) {
            assert!(pair[0].0 < pair[1].0, "{:?} is out of order", pair[1].0);
        }
    }

    #[test]
    fn placeholders_match_between_languages() {
        for (en, es) in SPANISH {
            assert_eq!(
                en.matches("{}").count(),
                es.matches("{}").count(),
                "{en}: placeholders differ"
            );
        }
    }

    #[test]
    fn trf_fills_placeholders_in_order() {
        set(Lang::En);
        assert_eq!(trf("Line {}, column {}", &["3", "9"]), "Line 3, column 9");
        set(Lang::Es);
    }

    #[test]
    fn spanish_tags() {
        for tag in ["es", "es-ES", "es_MX.UTF-8", "es-419", "ES"] {
            assert!(is_spanish(tag), "{tag}");
        }
        for tag in ["en", "en_US", "eu", "est", ""] {
            assert!(!is_spanish(tag), "{tag}");
        }
    }

    #[test]
    fn every_interface_string_has_spanish() {
        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut missing = Vec::new();
        let mut stack = vec![src];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).expect("read src") {
                let path = entry.expect("entry").path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                if path.file_name().is_some_and(|n| n == "i18n.rs") {
                    continue;
                }
                let text = std::fs::read_to_string(&path).expect("read file");
                for marker in ["tr(", "trf("] {
                    for (at, _) in text.match_indices(marker) {
                        let before = text[..at].chars().next_back();
                        if before.is_some_and(|c| c.is_alphanumeric() || c == '_') {
                            continue;
                        }
                        let rest = text[at + marker.len()..].trim_start();
                        let Some(piece) = rest.strip_prefix('"') else {
                            continue;
                        };
                        let Some(end) = piece.find('"') else {
                            continue;
                        };
                        let english = &piece[..end];
                        if SPANISH
                            .binary_search_by(|(en, _)| en.cmp(&english))
                            .is_err()
                        {
                            missing.push(format!("{}: {english}", path.display()));
                        }
                    }
                }
            }
        }
        missing.sort();
        missing.dedup();
        assert!(
            missing.is_empty(),
            "strings without Spanish:\n{}",
            missing.join("\n")
        );
    }

    fn has_spanish(english: &str) -> bool {
        SPANISH
            .binary_search_by(|(en, _)| (*en).cmp(english))
            .is_ok()
    }

    #[test]
    fn units_constants_dimensions_and_commands_have_spanish() {
        let mut missing: Vec<String> = Vec::new();
        for u in crate::catalog::units() {
            for text in [u.name, u.category.as_str()] {
                if !has_spanish(text) {
                    missing.push(text.to_owned());
                }
            }
        }
        for c in crate::catalog::constants_table(6) {
            if !has_spanish(c.description) {
                missing.push(c.description.to_owned());
            }
        }
        missing.sort();
        missing.dedup();
        assert!(missing.is_empty(), "without Spanish: {missing:?}");
    }

    #[test]
    fn the_samples_comments_have_spanish() {
        for comment in [
            "length",
            "load at the free end",
            "Young's modulus of steel",
            "second moment of area",
            "deflection at the free end",
            "peak bending stress",
            "resistances in ohms",
            "loop voltages in volts",
            "loop currents",
            "a one metre pendulum",
            "as long as the beam",
        ] {
            assert!(has_spanish(comment), "{comment}");
        }
    }
}
