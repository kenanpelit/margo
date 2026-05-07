//! Quick parser sanity check for a margo config file.
//!
//! Usage:
//!     cargo run -p margo-config --example check_config -- ~/.config/margo/config.conf
//!     cargo run -p margo-config --example check_config        # → ~/.config/margo/config.conf
//!
//! Reports rule counts and surfaces any rules that exercise the
//! niri-style additions (exclude clauses, min/max size, open_focused,
//! block_out_from_screencast) so the user can confirm their config
//! picked them up correctly without launching the compositor.

use std::path::PathBuf;

fn main() -> std::process::ExitCode {
    let arg = std::env::args().nth(1);
    let path: PathBuf = match arg {
        Some(p) => PathBuf::from(p),
        None => {
            let home = std::env::var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or_default();
            home.join(".config/margo/config.conf")
        }
    };

    if !path.exists() {
        eprintln!("config not found: {}", path.display());
        return std::process::ExitCode::FAILURE;
    }

    match margo_config::parse_config(Some(path.as_path())) {
        Ok(cfg) => {
            println!(
                "OK  {}  rules: {} window, {} layer, {} key bindings, {} gesture bindings",
                path.display(),
                cfg.window_rules.len(),
                cfg.layer_rules.len(),
                cfg.key_bindings.len(),
                cfg.gesture_bindings.len(),
            );

            let mut header_printed = false;
            for (i, r) in cfg.window_rules.iter().enumerate() {
                let touches_new = r.exclude_id.is_some()
                    || r.exclude_title.is_some()
                    || r.min_width > 0
                    || r.max_width > 0
                    || r.min_height > 0
                    || r.max_height > 0
                    || r.open_focused.is_some()
                    || r.block_out_from_screencast.is_some();
                if !touches_new {
                    continue;
                }
                if !header_printed {
                    println!("\nrules using niri-style features:");
                    header_printed = true;
                }
                let label = r
                    .id
                    .as_deref()
                    .or(r.title.as_deref())
                    .unwrap_or("(no match)");
                let label = if label.len() > 60 {
                    format!("{}…", &label[..59])
                } else {
                    label.to_string()
                };
                println!(
                    "  #{i:<3} match={label}\n        \
                     exclude_id={:?} exclude_title={:?}\n        \
                     min=({}×{}) max=({}×{}) open_focused={:?} blockout={:?}",
                    r.exclude_id,
                    r.exclude_title,
                    r.min_width,
                    r.min_height,
                    r.max_width,
                    r.max_height,
                    r.open_focused,
                    r.block_out_from_screencast,
                );
            }
            std::process::ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("PARSE ERROR in {}: {e:?}", path.display());
            std::process::ExitCode::FAILURE
        }
    }
}
