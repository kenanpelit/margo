//! Detail/preview shown in the launcher's side pane for the selected
//! result. Providers return one from `Provider::preview`; the default
//! impl derives a plain `Text` preview from the item, and providers
//! that benefit (calc, clipboard, apps) override it for richness.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PreviewKind {
    /// Body rendered as ordinary wrapped text.
    Text,
    /// Body rendered monospace (calc result, file path, code).
    Mono,
    /// Body is a colour value; the pane also paints a swatch.
    Color,
}

/// Content for the launcher preview pane.
#[derive(Debug, Clone)]
pub struct LauncherPreview {
    pub title: String,
    pub body: String,
    pub kind: PreviewKind,
    /// `#rrggbb` swatch to paint (only meaningful for `Color`).
    pub swatch: Option<String>,
}

impl LauncherPreview {
    pub fn text(title: impl Into<String>, body: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            body: body.into(),
            kind: PreviewKind::Text,
            swatch: None,
        }
    }
    pub fn mono(title: impl Into<String>, body: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            body: body.into(),
            kind: PreviewKind::Mono,
            swatch: None,
        }
    }
    pub fn color(swatch: impl Into<String>, body: impl Into<String>) -> Self {
        let swatch = swatch.into();
        Self {
            title: swatch.clone(),
            body: body.into(),
            kind: PreviewKind::Color,
            swatch: Some(swatch),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_preview_carries_title_and_body() {
        let p = LauncherPreview::text("Firefox", "Web browser");
        assert_eq!(p.title, "Firefox");
        assert_eq!(p.body, "Web browser");
        assert!(matches!(p.kind, PreviewKind::Text));
    }

    #[test]
    fn color_preview_keeps_the_swatch_hex() {
        let p = LauncherPreview::color("#ff8800", "#ff8800 — copied");
        assert_eq!(p.swatch.as_deref(), Some("#ff8800"));
        assert!(matches!(p.kind, PreviewKind::Color));
    }

    #[test]
    fn mono_preview_is_monospace() {
        let p = LauncherPreview::mono("12 * 8", "96");
        assert!(matches!(p.kind, PreviewKind::Mono));
    }
}
