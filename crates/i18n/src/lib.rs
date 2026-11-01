//! Minimal UI translation for LumaPrint. Source texts are English (the default language); the
//! tables in [`de`] hold the German translations. `tr!("Text")` returns the text in the current
//! language, `tr!("{count} images imported", count = n)` additionally fills named placeholders.
//!
//! The language is process-wide. Libraries translate when they create a text, so the GUI sets the
//! language once at startup (and when the user changes it); the CLI keeps the English default.

mod de;

use std::collections::HashMap;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU8, Ordering};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
    English,
    German,
}

impl Language {
    pub const ALL: [Self; 2] = [Self::English, Self::German];

    /// Language names are shown in their own language.
    pub const fn label(self) -> &'static str {
        match self {
            Self::English => "English",
            Self::German => "Deutsch",
        }
    }

    /// Stable code for settings files.
    pub const fn code(self) -> &'static str {
        match self {
            Self::English => "en",
            Self::German => "de",
        }
    }

    pub fn from_code(code: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|l| l.code() == code)
    }

    /// From `LC_ALL` / `LC_MESSAGES` / `LANG`: German for `de*`, otherwise English.
    pub fn from_env() -> Self {
        for var in ["LC_ALL", "LC_MESSAGES", "LANG"] {
            if let Ok(value) = std::env::var(var)
                && !value.is_empty()
            {
                return if value.starts_with("de") {
                    Self::German
                } else {
                    Self::English
                };
            }
        }
        Self::English
    }
}

static CURRENT: AtomicU8 = AtomicU8::new(0);

pub fn set_language(language: Language) {
    CURRENT.store(language as u8, Ordering::Relaxed);
}

pub fn language() -> Language {
    if CURRENT.load(Ordering::Relaxed) == Language::German as u8 {
        Language::German
    } else {
        Language::English
    }
}

fn german() -> &'static HashMap<&'static str, &'static str> {
    static MAP: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();
    MAP.get_or_init(|| de::TABLES.iter().flat_map(|t| t.iter().copied()).collect())
}

/// `english` in the current language (English if no translation exists).
pub fn tr(english: &'static str) -> &'static str {
    match language() {
        Language::English => english,
        Language::German => german().get(english).copied().unwrap_or(english),
    }
}

/// Like [`tr`] for texts only known at runtime.
pub fn tr_dyn(english: &str) -> String {
    match language() {
        Language::English => english.to_string(),
        Language::German => german()
            .get(english)
            .map_or_else(|| english.to_string(), |s| (*s).to_string()),
    }
}

/// Replaces `{name}` placeholders in `template`.
pub fn fill(template: &str, args: &[(&str, String)]) -> String {
    let mut out = template.to_string();
    for (name, value) in args {
        out = out.replace(&format!("{{{name}}}"), value);
    }
    out
}

#[macro_export]
macro_rules! tr {
    ($en:literal) => {
        $crate::tr($en)
    };
    ($en:literal, $($name:ident = $value:expr),+ $(,)?) => {
        $crate::fill(
            $crate::tr($en),
            &[$((stringify!($name), ($value).to_string())),+],
        )
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    fn rust_sources(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                rust_sources(&path, out);
            } else if path.extension().is_some_and(|e| e == "rs") {
                out.push(path);
            }
        }
    }

    /// The text of every `tr!("…")` in `src` (escape sequences resolved).
    fn tr_texts(src: &str) -> Vec<String> {
        let mut texts = Vec::new();
        let mut rest = src;
        while let Some(pos) = rest.find("tr!(\"") {
            rest = &rest[pos + 5..];
            let mut text = String::new();
            let mut chars = rest.chars();
            while let Some(c) = chars.next() {
                match c {
                    '"' => break,
                    '\\' => match chars.next() {
                        Some('n') => text.push('\n'),
                        Some('\n') => {
                            // Line continuation: skip the leading whitespace of the next line.
                            let skipped: String =
                                chars.clone().take_while(|c| c.is_whitespace()).collect();
                            for _ in skipped.chars() {
                                chars.next();
                            }
                        }
                        Some(other) => text.push(other),
                        None => break,
                    },
                    _ => text.push(c),
                }
            }
            texts.push(text);
        }
        texts
    }

    /// Every `tr!("…")` text in the workspace sources needs a German translation.
    #[test]
    fn all_ui_texts_are_translated() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let mut files = Vec::new();
        rust_sources(&root.join("crates"), &mut files);
        rust_sources(&root.join("apps"), &mut files);
        files.retain(|f| !f.starts_with(root.join("crates/i18n")));
        assert!(files.len() > 10, "workspace sources not found");
        let mut missing = Vec::new();
        for file in files {
            let src = std::fs::read_to_string(&file).unwrap_or_default();
            for text in tr_texts(&src) {
                if !german().contains_key(text.as_str()) {
                    missing.push(text);
                }
            }
        }
        missing.sort();
        missing.dedup();
        assert!(
            missing.is_empty(),
            "missing German translations: {missing:#?}"
        );
    }

    /// A translation must use exactly the placeholders of the English text.
    #[test]
    fn translations_keep_placeholders() {
        fn names(text: &str) -> Vec<&str> {
            let mut out: Vec<&str> = text
                .split('{')
                .skip(1)
                .filter_map(|part| part.split_once('}').map(|(name, _)| name))
                .collect();
            out.sort_unstable();
            out
        }
        for (en, de) in de::TABLES.iter().flat_map(|t| t.iter()) {
            assert_eq!(names(en), names(de), "{en:?} -> {de:?}");
        }
    }

    /// The same English text must not be translated differently in two places.
    #[test]
    fn translations_are_unambiguous() {
        let mut seen: HashMap<&str, &str> = HashMap::new();
        for (en, de) in de::TABLES.iter().flat_map(|t| t.iter()) {
            if let Some(previous) = seen.insert(en, de) {
                assert_eq!(previous, *de, "{en:?} translated twice");
            }
        }
    }

    #[test]
    fn placeholders_are_filled() {
        assert_eq!(
            fill(
                "Sheet {page} of {total}",
                &[("page", "2".into()), ("total", "5".into())]
            ),
            "Sheet 2 of 5"
        );
    }

    #[test]
    fn line_continuations_are_joined() {
        assert_eq!(tr_texts("tr!(\"a \\\n      b\")"), vec!["a b".to_string()]);
    }
}
