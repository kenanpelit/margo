pub(crate) mod qr_menu_widget;

use std::sync::Mutex;

/// What `mshellctl qr …` asks the QR menu to show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QrRequest {
    /// Encode this text / URL.
    Text(String),
    /// Encode the clipboard's text.
    Clipboard,
    /// The Wi-Fi network this machine is on (the code holds its password).
    Wifi,
    /// The hotspot (the code holds its password).
    Hotspot,
}

/// Validate the bus arguments of `Qr(mode, text)`: `text` with a payload,
/// or `clipboard` / `wifi` / `hotspot` with none. The message is the
/// D-Bus `InvalidArgs` text `mshellctl` prints.
pub fn parse_qr_request(mode: &str, text: &str) -> Result<QrRequest, String> {
    match mode {
        "text" => {
            mshell_utils::qr::check_payload(text)?;
            Ok(QrRequest::Text(text.to_string()))
        }
        "clipboard" | "wifi" | "hotspot" if !text.is_empty() => {
            Err(format!("qr mode `{mode}` takes no text"))
        }
        "clipboard" => Ok(QrRequest::Clipboard),
        "wifi" => Ok(QrRequest::Wifi),
        "hotspot" => Ok(QrRequest::Hotspot),
        other => Err(format!(
            "unknown qr mode `{other}` (text | clipboard | wifi | hotspot)"
        )),
    }
}

/// The request waiting for the QR menu's next reveal. The IPC handler
/// stores it, then opens the menu; the menu takes it when it appears.
static PENDING: Mutex<Option<QrRequest>> = Mutex::new(None);

/// Queue `req` for the QR menu (called before the menu is opened).
pub fn request(req: QrRequest) {
    if let Ok(mut slot) = PENDING.lock() {
        *slot = Some(req);
    }
}

pub(crate) fn take_request() -> Option<QrRequest> {
    PENDING.lock().ok().and_then(|mut slot| slot.take())
}

/// The QR code of the Wi-Fi network this machine is on (`hotspot: false`)
/// or of its hotspot, with the caption to show under it. The key goes
/// NetworkManager → zeroized payload → qrencode's stdin, nowhere else.
pub(crate) async fn wifi_code(
    hotspot: bool,
) -> Result<(mshell_utils::qr::QrMatrix, String), String> {
    use mshell_utils::{nm_wifi, qr};
    let share = if hotspot {
        nm_wifi::hotspot().await?
    } else {
        nm_wifi::active_wifi().await?
    };
    let matrix = qr::render(&share.payload()).await?;
    let secret_note = if share.auth == qr::WifiAuth::Open {
        "Open network: no password"
    } else {
        "The code contains the network password"
    };
    Ok((
        matrix,
        format!(
            "Scan with a phone camera to join “{}” ({})\n{secret_note}",
            share.ssid,
            share.auth.label()
        ),
    ))
}

/// The clipboard's text (`wl-paste`, so it works without keyboard focus).
pub(crate) async fn clipboard_text() -> Result<String, String> {
    if !margo_fs::path::on_path("wl-paste") {
        return Err("wl-paste is not installed (package wl-clipboard)".to_string());
    }
    let out = tokio::time::timeout(
        std::time::Duration::from_secs(3),
        tokio::process::Command::new("wl-paste")
            .args(["--no-newline", "--type", "text"])
            .kill_on_drop(true)
            .output(),
    )
    .await
    .map_err(|_| "Reading the clipboard timed out".to_string())?
    .map_err(|e| format!("wl-paste: {e}"))?;
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    if !out.status.success() || text.trim().is_empty() {
        return Err("The clipboard holds no text".to_string());
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bus_arguments_are_validated() {
        assert_eq!(
            parse_qr_request("text", "https://example.org"),
            Ok(QrRequest::Text("https://example.org".into()))
        );
        assert_eq!(parse_qr_request("wifi", ""), Ok(QrRequest::Wifi));
        assert_eq!(parse_qr_request("hotspot", ""), Ok(QrRequest::Hotspot));
        assert_eq!(parse_qr_request("clipboard", ""), Ok(QrRequest::Clipboard));
        assert!(parse_qr_request("text", "").is_err());
        assert!(parse_qr_request("text", &"x".repeat(5000)).is_err());
        assert!(parse_qr_request("wifi", "extra").is_err());
        assert!(parse_qr_request("png", "").is_err());
    }

    #[test]
    fn a_request_is_taken_once() {
        request(QrRequest::Wifi);
        request(QrRequest::Text("later wins".into()));
        assert_eq!(take_request(), Some(QrRequest::Text("later wins".into())));
        assert_eq!(take_request(), None);
    }
}
