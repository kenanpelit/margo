//! QR codes the user scans from a phone: a Wi-Fi network (the standard
//! `WIFI:` payload) or any text / URL. Encoded by the `qrencode` CLI into a
//! module map held in memory ([`QrMatrix`]) — the payload goes to qrencode
//! on **stdin**, never on an argv (a Wi-Fi payload carries the network
//! password, and every local user can read `/proc/<pid>/cmdline`), and no
//! image is ever written to disk. The UI draws the modules itself (crisp at
//! any scale) and builds the clipboard image from [`QrMatrix::to_gray`].
//!
//! Shared by the Network menu's "Show QR" (current Wi-Fi / hotspot) and the
//! QR menu (`mshellctl qr`).

use std::process::Stdio;
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use zeroize::Zeroizing;

/// Longest payload accepted, in bytes. A version-40 code holds ~2.3 kB at
/// level M, but anything past ~1 kB packs modules too small for a phone
/// camera to read off a laptop screen.
pub const MAX_PAYLOAD: usize = 1000;
const TIMEOUT: Duration = Duration::from_secs(10);

/// How a Wi-Fi network authenticates, as the QR payload names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WifiAuth {
    /// No password (open or OWE "Enhanced Open").
    Open,
    /// WEP key.
    Wep,
    /// WPA/WPA2 personal, or WPA2/WPA3 transition.
    Wpa,
    /// WPA3-personal only (SAE): `T:WPA` plus the `R:1` transition-disable
    /// flag of the WPA3 spec §7.1 — not the non-standard `T:SAE`.
    Wpa3Only,
}

impl WifiAuth {
    /// From NetworkManager's `802-11-wireless-security.key-mgmt` (empty =
    /// no security section = open). `None` for enterprise (802.1X): a QR
    /// code cannot carry its identity / certificates.
    pub fn from_key_mgmt(key_mgmt: &str, has_wep_key: bool) -> Option<Self> {
        match key_mgmt.trim() {
            "" | "owe" => Some(Self::Open),
            "none" if has_wep_key => Some(Self::Wep),
            "none" => Some(Self::Open),
            "wpa-psk" => Some(Self::Wpa),
            "sae" => Some(Self::Wpa3Only),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Open => "Open",
            Self::Wep => "WEP",
            Self::Wpa => "WPA/WPA2",
            Self::Wpa3Only => "WPA3",
        }
    }
}

/// Backslash-escape the characters the `WIFI:` grammar reserves
/// (`\ ; , : "`), as ZXing and the phone scanners expect.
pub fn escape_wifi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if matches!(c, '\\' | ';' | ',' | ':' | '"') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// The `WIFI:` payload a phone camera joins from:
/// `WIFI:T:WPA;S:<ssid>;P:<password>;;` (plus `R:1` for WPA3-only,
/// `H:true` for a hidden network). Zeroized on drop — it holds the key.
pub fn wifi_payload(ssid: &str, auth: WifiAuth, password: &str, hidden: bool) -> Zeroizing<String> {
    let mut p = Zeroizing::new(String::from("WIFI:"));
    match auth {
        WifiAuth::Open => p.push_str("T:nopass;"),
        WifiAuth::Wep => p.push_str("T:WEP;"),
        WifiAuth::Wpa => p.push_str("T:WPA;"),
        WifiAuth::Wpa3Only => p.push_str("T:WPA;R:1;"),
    }
    p.push_str("S:");
    p.push_str(&escape_wifi(ssid));
    p.push(';');
    if auth != WifiAuth::Open {
        p.push_str("P:");
        p.push_str(&Zeroizing::new(escape_wifi(password)));
        p.push(';');
    }
    if hidden {
        p.push_str("H:true;");
    }
    p.push(';');
    p
}

/// Why a payload can't become a QR code, worded for the UI.
pub fn check_payload(payload: &str) -> Result<(), String> {
    if payload.trim().is_empty() {
        return Err("Nothing to encode".to_string());
    }
    if payload.len() > MAX_PAYLOAD {
        return Err(format!(
            "Too long for a scannable QR code ({} of at most {MAX_PAYLOAD} bytes)",
            payload.len()
        ));
    }
    Ok(())
}

/// A rendered QR code: `size × size` modules, row-major, `1` = dark.
/// Zeroized on drop (a Wi-Fi code encodes the key).
#[derive(Clone, PartialEq, Eq)]
pub struct QrMatrix {
    size: usize,
    modules: Zeroizing<Vec<u8>>,
}

impl std::fmt::Debug for QrMatrix {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "QrMatrix({0}x{0})", self.size)
    }
}

impl QrMatrix {
    /// Modules per side (21 for version 1, +4 per version).
    pub fn size(&self) -> usize {
        self.size
    }

    /// Whether the module at column `x`, row `y` is dark.
    pub fn dark(&self, x: usize, y: usize) -> bool {
        x < self.size && y < self.size && self.modules[y * self.size + x] == 1
    }

    /// An 8-bit grey image (0 = dark, 255 = light) of the code with a
    /// `quiet`-module light border, `module_px` pixels per module:
    /// `(side_px, pixels)`. Feeds the clipboard PNG and the tests' decoder.
    pub fn to_gray(&self, module_px: usize, quiet: usize) -> (usize, Zeroizing<Vec<u8>>) {
        let module_px = module_px.max(1);
        let side = (self.size + 2 * quiet) * module_px;
        let mut px = Zeroizing::new(vec![255u8; side * side]);
        for y in 0..side {
            let my = (y / module_px).checked_sub(quiet);
            for x in 0..side {
                let mx = (x / module_px).checked_sub(quiet);
                if let (Some(mx), Some(my)) = (mx, my)
                    && self.dark(mx, my)
                {
                    px[y * side + x] = 0;
                }
            }
        }
        (side, px)
    }
}

/// Parse `qrencode -t ASCII -m 0`: one text row per module row, each
/// module two characters (`##` dark, two spaces light).
pub fn parse_ascii(out: &str) -> Result<QrMatrix, String> {
    let rows: Vec<&str> = out.lines().filter(|l| !l.is_empty()).collect();
    let size = rows.len();
    if size < 21 {
        return Err("qrencode produced no QR code".to_string());
    }
    let mut modules = Zeroizing::new(Vec::with_capacity(size * size));
    for row in &rows {
        let bytes = row.as_bytes();
        if bytes.len() != size * 2 {
            return Err("qrencode produced a malformed QR code".to_string());
        }
        modules.extend(bytes.chunks(2).map(|m| u8::from(m[0] == b'#')));
    }
    Ok(QrMatrix { size, modules })
}

/// The fixed qrencode argv: an ASCII module map to stdout, no quiet zone
/// (the UI draws its own), level M. The payload comes from stdin.
pub fn qrencode_args() -> [&'static str; 6] {
    ["-t", "ASCII", "-m", "0", "-l", "M"]
}

/// Encode `payload` (on qrencode's stdin only).
pub async fn render(payload: &str) -> Result<QrMatrix, String> {
    render_with("qrencode", payload).await
}

/// [`render`] against an explicit binary (tests point it at fakes).
pub async fn render_with(bin: &str, payload: &str) -> Result<QrMatrix, String> {
    check_payload(payload)?;
    if !margo_fs::path::on_path(bin) && !std::path::Path::new(bin).is_file() {
        return Err("qrencode is not installed (package qrencode)".to_string());
    }
    let mut child = tokio::process::Command::new(bin)
        .args(qrencode_args())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| format!("qrencode: {e}"))?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| "qrencode: no stdin".to_string())?;
    let bytes = Zeroizing::new(payload.as_bytes().to_vec());
    let write = async move {
        let r = stdin.write_all(&bytes).await;
        drop(stdin); // EOF: qrencode encodes what it read.
        r
    };
    let run = async {
        let (w, out) = tokio::join!(write, child.wait_with_output());
        w.map_err(|e| format!("qrencode: {e}"))?;
        out.map_err(|e| format!("qrencode: {e}"))
    };
    let out = tokio::time::timeout(TIMEOUT, run)
        .await
        .map_err(|_| "qrencode timed out".to_string())??;
    let stdout = Zeroizing::new(out.stdout);
    if !out.status.success() {
        // qrencode's stderr names the problem ("Input data too large"); it
        // never echoes the input.
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return Err(if err.is_empty() {
            format!("qrencode exited with {}", out.status)
        } else {
            err
        });
    }
    parse_ascii(&String::from_utf8_lossy(&stdout))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wifi_payloads_follow_the_spec() {
        assert_eq!(
            *wifi_payload("Ken_5", WifiAuth::Wpa, "hunter22", false),
            "WIFI:T:WPA;S:Ken_5;P:hunter22;;"
        );
        assert_eq!(
            *wifi_payload("Cafe", WifiAuth::Open, "ignored", false),
            "WIFI:T:nopass;S:Cafe;;"
        );
        assert_eq!(
            *wifi_payload("Lab", WifiAuth::Wpa3Only, "k", true),
            "WIFI:T:WPA;R:1;S:Lab;P:k;H:true;;"
        );
        assert_eq!(
            *wifi_payload("Old", WifiAuth::Wep, "abcde", false),
            "WIFI:T:WEP;S:Old;P:abcde;;"
        );
    }

    #[test]
    fn reserved_characters_are_backslash_escaped() {
        assert_eq!(escape_wifi(r#"a;b,c:d"e\f"#), r#"a\;b\,c\:d\"e\\f"#);
        assert_eq!(
            *wifi_payload("My;Net", WifiAuth::Wpa, "p:a,ss", false),
            r"WIFI:T:WPA;S:My\;Net;P:p\:a\,ss;;"
        );
        // Non-ASCII passes through untouched (UTF-8 bytes).
        assert_eq!(escape_wifi("çağrı"), "çağrı");
    }

    #[test]
    fn key_mgmt_maps_to_the_payload_type() {
        assert_eq!(
            WifiAuth::from_key_mgmt("wpa-psk", false),
            Some(WifiAuth::Wpa)
        );
        assert_eq!(
            WifiAuth::from_key_mgmt("sae", false),
            Some(WifiAuth::Wpa3Only)
        );
        assert_eq!(WifiAuth::from_key_mgmt("", false), Some(WifiAuth::Open));
        assert_eq!(WifiAuth::from_key_mgmt("owe", false), Some(WifiAuth::Open));
        assert_eq!(WifiAuth::from_key_mgmt("none", true), Some(WifiAuth::Wep));
        assert_eq!(WifiAuth::from_key_mgmt("none", false), Some(WifiAuth::Open));
        assert_eq!(WifiAuth::from_key_mgmt("wpa-eap", false), None);
    }

    #[test]
    fn payload_limits() {
        assert!(check_payload("https://example.org").is_ok());
        assert!(check_payload("   ").is_err());
        assert!(check_payload(&"x".repeat(MAX_PAYLOAD)).is_ok());
        assert!(check_payload(&"x".repeat(MAX_PAYLOAD + 1)).is_err());
    }

    #[test]
    fn the_payload_is_never_an_argv_element() {
        let args = qrencode_args();
        // No trailing STRING argument (qrencode then reads stdin), and the
        // output is text on stdout: nothing on disk.
        assert_eq!(args, ["-t", "ASCII", "-m", "0", "-l", "M"]);
    }

    /// `printf hi | qrencode -t ASCII -m 0 -l M`, captured on the owner's
    /// machine (qrencode 4.1.1).
    const HI: &str = include_str!("testdata/qrencode_hi.txt");

    #[test]
    fn parses_the_ascii_module_map() {
        let m = parse_ascii(HI).unwrap();
        assert_eq!(m.size(), 21);
        // Finder pattern corners: dark ring, light gap, dark core.
        assert!(m.dark(0, 0) && m.dark(6, 0) && m.dark(0, 6) && m.dark(6, 6));
        assert!(!m.dark(1, 1) && m.dark(2, 2) && m.dark(4, 4));
        assert!(m.dark(20, 0) && m.dark(0, 20));
        assert!(!m.dark(21, 0), "outside the code is light");
        assert!(parse_ascii("").is_err());
        assert!(parse_ascii(&HI.replacen("##", "#", 1)).is_err());
    }

    #[test]
    fn gray_image_has_the_quiet_zone() {
        let m = parse_ascii(HI).unwrap();
        let (side, px) = m.to_gray(3, 2);
        assert_eq!(side, (21 + 4) * 3);
        assert_eq!(px.len(), side * side);
        assert_eq!(px[0], 255, "quiet zone is light");
        assert_eq!(px[(2 * 3) * side + 2 * 3], 0, "first module is dark");
    }

    /// The real tools: encode with qrencode, draw the module map as a PGM,
    /// decode it with zbarimg (both installed on the owner's machine):
    /// byte-exact round trip.
    #[tokio::test]
    async fn round_trips_through_qrencode_and_zbarimg() {
        if !margo_fs::path::on_path("qrencode") || !margo_fs::path::on_path("zbarimg") {
            eprintln!("skipped: qrencode / zbarimg not installed");
            return;
        }
        for payload in [
            "https://example.org/a?b=c",
            r"WIFI:T:WPA;S:My\;Net;P:p\:a ss;;",
            "çağrı ünlü",
        ] {
            let m = render(payload).await.expect("qrencode renders");
            let (side, px) = m.to_gray(6, 4);
            let mut pgm = format!("P5 {side} {side} 255\n").into_bytes();
            pgm.extend_from_slice(&px);
            let mut zbar = tokio::process::Command::new("zbarimg")
                .args(["-q", "--raw", "-Sbinary", "-"])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .spawn()
                .unwrap();
            let mut stdin = zbar.stdin.take().unwrap();
            stdin.write_all(&pgm).await.unwrap();
            drop(stdin);
            let out = zbar.wait_with_output().await.unwrap();
            assert_eq!(String::from_utf8_lossy(&out.stdout), payload);
        }
        // Too big for any QR version: an error, not a panic.
        let huge = "x".repeat(MAX_PAYLOAD);
        assert!(render(&format!("{huge}{huge}{huge}")).await.is_err());
    }

    #[tokio::test]
    async fn a_missing_or_failing_qrencode_is_an_error() {
        assert!(
            render_with("/nonexistent/qrencode", "x")
                .await
                .unwrap_err()
                .contains("not installed")
        );
        // A fake qrencode that swallows its input and prints garbage.
        let dir = tempfile::tempdir().unwrap();
        let fake = dir.path().join("qrencode");
        std::fs::write(&fake, "#!/bin/sh\ncat >/dev/null\necho nope\n").unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        let err = render_with(fake.to_str().unwrap(), "x").await.unwrap_err();
        assert!(err.contains("no QR code"), "{err}");
    }
}
