//! `mshellctl qr` — show a QR code to scan from a phone, in the shell's QR
//! menu (opened on the focused output).
//!
//! ```text
//! mshellctl qr "https://example.org"   # encode this text / URL
//! echo hello | mshellctl qr -           # encode stdin
//! mshellctl qr --clipboard              # encode the clipboard's text
//! mshellctl qr --wifi                   # join the Wi-Fi this machine is on
//! mshellctl qr --hotspot                # join this machine's hotspot
//! ```
//!
//! The Wi-Fi modes send no secret over the bus: the shell reads the network
//! password from NetworkManager itself and hands it to `qrencode` on stdin.

use crate::bus::shell;
use clap::Args;
use std::io::Read;

#[derive(Args, Debug)]
#[command(group(clap::ArgGroup::new("source").args(["text", "clipboard", "wifi", "hotspot"]).required(true)))]
pub struct QrArgs {
    /// Text or URL to encode (`-` reads standard input).
    pub text: Option<String>,
    /// Encode the clipboard's text.
    #[arg(long)]
    pub clipboard: bool,
    /// The Wi-Fi network this machine is connected to (the code contains
    /// its password).
    #[arg(long)]
    pub wifi: bool,
    /// This machine's hotspot (the code contains its password).
    #[arg(long)]
    pub hotspot: bool,
}

/// `(mode, text)` for the `Qr` bus method.
pub fn bus_args(
    args: &QrArgs,
    stdin: impl FnOnce() -> std::io::Result<String>,
) -> anyhow::Result<(&'static str, String)> {
    Ok(if args.clipboard {
        ("clipboard", String::new())
    } else if args.wifi {
        ("wifi", String::new())
    } else if args.hotspot {
        ("hotspot", String::new())
    } else {
        match args.text.as_deref() {
            Some("-") => ("text", stdin()?.trim_end_matches(['\n', '\r']).to_string()),
            Some(t) => ("text", t.to_string()),
            None => anyhow::bail!("nothing to encode"),
        }
    })
}

pub async fn execute(args: QrArgs) -> anyhow::Result<()> {
    let (mode, text) = bus_args(&args, || {
        let mut s = String::new();
        std::io::stdin().read_to_string(&mut s)?;
        Ok(s)
    })?;
    shell().await?.qr(mode, &text).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[derive(Parser)]
    struct T {
        #[command(flatten)]
        qr: QrArgs,
    }

    fn parse(argv: &[&str]) -> Result<QrArgs, clap::Error> {
        T::try_parse_from(std::iter::once("qr").chain(argv.iter().copied())).map(|t| t.qr)
    }

    #[test]
    fn modes_map_to_bus_arguments() {
        let none = || -> std::io::Result<String> { panic!("stdin not read") };
        let a = parse(&["https://example.org"]).unwrap();
        assert_eq!(
            bus_args(&a, none).unwrap(),
            ("text", "https://example.org".into())
        );
        let a = parse(&["--wifi"]).unwrap();
        assert_eq!(bus_args(&a, none).unwrap(), ("wifi", String::new()));
        let a = parse(&["--hotspot"]).unwrap();
        assert_eq!(bus_args(&a, none).unwrap(), ("hotspot", String::new()));
        let a = parse(&["--clipboard"]).unwrap();
        assert_eq!(bus_args(&a, none).unwrap(), ("clipboard", String::new()));
        let a = parse(&["-"]).unwrap();
        assert_eq!(
            bus_args(&a, || Ok("from stdin\n".into())).unwrap(),
            ("text", "from stdin".into())
        );
    }

    #[test]
    fn exactly_one_source() {
        assert!(parse(&[]).is_err());
        assert!(parse(&["x", "--wifi"]).is_err());
        assert!(parse(&["--wifi", "--hotspot"]).is_err());
    }
}
