//! `mshellctl theme bundle list|apply`: the saved theme bundles
//! (Settings → Theme → Bundles), answered on the GTK main thread.

use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{ConfigStoreFields, Theme};
use reactive_graph::traits::GetUntracked;

/// `None` lists the bundles; `Some(name)` applies that bundle (its scheme,
/// matugen settings and wallpaper), the same path as a bundle card.
pub(super) fn reply(name: Option<String>) -> Result<String, String> {
    match name {
        None => Ok(list(&config_manager().config().theme().get_untracked())),
        Some(name) => mshell_cache::theme_bundle::apply_theme_bundle(&name)
            .map(|()| String::new())
            .map_err(|e| e.to_string()),
    }
}

/// One `name<TAB>scheme<TAB>wallpaper-count[<TAB>active]` line per bundle.
fn list(theme: &Theme) -> String {
    theme
        .bundles
        .iter()
        .map(|b| {
            let mut line = format!("{}\t{}\t{}", b.name, b.theme.ident(), b.wallpapers.len());
            if b.name == theme.active_bundle {
                line.push_str("\tactive");
            }
            line
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use mshell_config::schema::config::ThemeBundle;
    use mshell_config::schema::themes::Themes;

    #[test]
    fn list_marks_the_active_bundle() {
        let mut theme = Theme::default();
        assert_eq!(list(&theme), "");
        theme.bundles = vec![
            ThemeBundle {
                name: "Day".into(),
                theme: Themes::Wallpaper,
                wallpapers: vec!["/a".into(), "/b".into()],
                ..ThemeBundle::default()
            },
            ThemeBundle {
                name: "Night".into(),
                theme: Themes::TokyoNight,
                ..ThemeBundle::default()
            },
        ];
        theme.active_bundle = "Night".into();
        let night = Themes::TokyoNight.ident();
        let day = Themes::Wallpaper.ident();
        assert_eq!(
            list(&theme),
            format!("Day\t{day}\t2\nNight\t{night}\t0\tactive")
        );
    }
}
