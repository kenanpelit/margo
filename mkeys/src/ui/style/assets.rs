use core::str;

use rust_embed::Embed;

#[derive(Embed)]
#[folder = "assets/style/"]
pub struct StyleAssets;

impl StyleAssets {
    pub fn get_default_style_file() -> String {
        // Self-hosted asset — always present.
        let css_file = StyleAssets::get("default-style.css").unwrap();
        String::from(str::from_utf8(css_file.data.as_ref()).unwrap())
    }
}
