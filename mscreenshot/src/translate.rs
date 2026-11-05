//! `mscreenshot ocr --translate` — OCR → translation, through the same
//! engine and settings as `mshellctl translate` (Settings → Translate:
//! provider, target language, secondary language for the auto-flip, the
//! DeepL key in the keyring).
//!
//! The recognised text only leaves the machine after the same privacy
//! screen the translate trigger uses ([`mshell_translate::looks_sensitive`]);
//! a token-shaped capture is copied untranslated instead.

use mshell_translate::{TranslateConfig, config, looks_sensitive, translate_auto};

/// What the OCR + translate run ends with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Translated: `(text, from, to)`; `from` is the provider's detected
    /// source language when it reported one.
    Translated {
        text: String,
        from: Option<String>,
        to: String,
    },
    /// Not sent (privacy screen) or the provider failed — the original text
    /// is what gets copied; `reason` says why.
    Kept { reason: String },
}

impl Outcome {
    /// Clipboard text for this outcome given the recognised `original`.
    pub fn clipboard<'a>(&'a self, original: &'a str) -> &'a str {
        match self {
            Outcome::Translated { text, .. } => text,
            Outcome::Kept { .. } => original,
        }
    }

    /// Notification title.
    pub fn title(&self) -> String {
        match self {
            Outcome::Translated { from, to, .. } => match from {
                Some(f) => format!("Translated ({f} → {to})"),
                None => format!("Translated (→ {to})"),
            },
            Outcome::Kept { reason } => format!("Text copied — not translated: {reason}"),
        }
    }
}

/// Apply a `--to LANG` override to the stored config.
pub fn with_target(mut cfg: TranslateConfig, to: Option<&str>) -> TranslateConfig {
    if let Some(lang) = to.map(str::trim).filter(|l| !l.is_empty()) {
        cfg.target_lang = lang.to_lowercase();
    }
    cfg
}

/// Translate recognised text (blocking network call; this is a CLI).
pub fn run(text: &str, to: Option<&str>) -> Outcome {
    if let Some(reason) = looks_sensitive(text) {
        return Outcome::Kept {
            reason: reason.to_string(),
        };
    }
    let settings = config::load();
    let cfg = with_target(config::resolved(), to);
    // An explicit `--to` means exactly that language: no auto-flip.
    let secondary = if to.is_some() {
        String::new()
    } else {
        settings.secondary_lang
    };
    match translate_auto(&cfg, &secondary, text) {
        Ok((res, used)) => Outcome::Translated {
            text: res.translated,
            from: res.detected_source,
            to: used,
        },
        Err(e) => Outcome::Kept { reason: e },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> TranslateConfig {
        TranslateConfig {
            provider: mshell_translate::Provider::Google,
            target_lang: "tr".into(),
            source_lang: None,
            deepl_api_key: zeroize::Zeroizing::new(String::new()),
        }
    }

    #[test]
    fn target_override() {
        assert_eq!(with_target(cfg(), Some(" EN ")).target_lang, "en");
        assert_eq!(with_target(cfg(), Some("")).target_lang, "tr");
        assert_eq!(with_target(cfg(), None).target_lang, "tr");
    }

    #[test]
    fn secrets_never_leave_the_machine() {
        // Screened before any config / network access.
        let out = run("ghp_0123456789abcdefghijklmnopqrstuvwxyz", None);
        assert!(matches!(out, Outcome::Kept { .. }));
        assert_eq!(out.clipboard("orig"), "orig");
    }

    #[test]
    fn titles() {
        let t = Outcome::Translated {
            text: "merhaba".into(),
            from: Some("en".into()),
            to: "tr".into(),
        };
        assert_eq!(t.title(), "Translated (en → tr)");
        assert_eq!(t.clipboard("hello"), "merhaba");
        let k = Outcome::Kept {
            reason: "looks like a JWT".into(),
        };
        assert!(k.title().ends_with("looks like a JWT"));
    }
}
