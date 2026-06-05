use rust_embed::Embed;

#[derive(Embed)]
#[folder = "assets/layouts/"]
pub struct LayoutAssets;

impl LayoutAssets {
    /// Embedded layout TOML by name (`"en"`, `"tr"`); falls back to `en`.
    pub fn by_name(name: &str) -> String {
        let file = format!("{name}.toml");
        let data = LayoutAssets::get(&file)
            .or_else(|| LayoutAssets::get("en.toml"))
            .expect("en.toml is bundled");
        String::from_utf8(data.data.into_owned()).expect("layout is valid utf-8")
    }
}
