use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct KeyDefinition {
    pub top_legend: Option<String>,
    pub bottom_legend: Option<String>,
    pub scan_code: u16,
    pub width: Option<f32>,
}

pub enum KeyType {
    Mod,
    Lock,
    Normal,
}

impl KeyDefinition {
    pub fn key_type(&self) -> KeyType {
        if self.is_mod_key() {
            KeyType::Mod
        } else if self.is_lock_key() {
            KeyType::Lock
        } else {
            KeyType::Normal
        }
    }

    fn is_mod_key(&self) -> bool {
        let k = evdev::KeyCode::new(self.scan_code);
        k == evdev::KeyCode::KEY_LEFTCTRL
            || k == evdev::KeyCode::KEY_RIGHTCTRL
            || k == evdev::KeyCode::KEY_LEFTMETA
            || k == evdev::KeyCode::KEY_RIGHTMETA
            || k == evdev::KeyCode::KEY_LEFTSHIFT
            || k == evdev::KeyCode::KEY_RIGHTSHIFT
            || k == evdev::KeyCode::KEY_LEFTALT
            || k == evdev::KeyCode::KEY_RIGHTALT
    }

    fn is_lock_key(&self) -> bool {
        let k = evdev::KeyCode::new(self.scan_code);
        k == evdev::KeyCode::KEY_CAPSLOCK
            || k == evdev::KeyCode::KEY_NUMLOCK
            || k == evdev::KeyCode::KEY_SCROLLLOCK
    }
}

pub type Layout = Vec<Vec<KeyDefinition>>;

#[derive(Serialize, Deserialize, Debug)]
pub struct LayoutDefinition {
    pub layout: Layout,
    #[serde(skip_deserializing)]
    pub width: f32,
    #[serde(skip_deserializing)]
    pub height: i32,
}

impl LayoutDefinition {
    /// Parse a layout TOML and compute its geometry. Returns `Err` on garbage
    /// instead of panicking, so callers can fall back to a bundled layout.
    pub fn from_toml(toml_str: &str) -> Result<Self, toml::de::Error> {
        let mut def = toml::from_str::<LayoutDefinition>(toml_str)?;
        def.height = def.layout.len() as i32;
        def.width = def
            .layout
            .iter()
            .map(|row| row.iter().map(|k| k.width.unwrap_or(1.0)).sum::<f32>())
            .fold(0.0_f32, f32::max);
        Ok(def)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_layout_and_computes_geometry() {
        // Two rows; the second row is wider (1 + 2 = 3).
        let toml = r#"
            layout = [
                [ { top_legend = "Q", scan_code = 16 } ],
                [
                    { top_legend = "A", scan_code = 30 },
                    { top_legend = "Wide", scan_code = 57, width = 2.0 },
                ],
            ]
        "#;
        let def = LayoutDefinition::from_toml(toml).expect("valid layout");
        assert_eq!(def.height, 2);
        assert_eq!(def.width, 3.0);
        assert_eq!(def.layout[0][0].scan_code, 16);
        assert_eq!(def.layout[1][1].width, Some(2.0));
    }

    #[test]
    fn rejects_garbage_without_panicking() {
        assert!(LayoutDefinition::from_toml("not = [valid").is_err());
    }
}
