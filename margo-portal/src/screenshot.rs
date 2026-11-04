//! `org.freedesktop.impl.portal.Screenshot` backend.
//!
//! Fulfilled by driving margo's own `org.gnome.Shell.Screenshot` shim
//! (compositor screencopy → PNG + native colour pick), same pattern as
//! the ScreenCast backend. This lets `margo-portals.conf` route
//! `Screenshot=margo` and drop the GNOME backend entirely.
//!
//! Consent. xdg-desktop-portal (screenshot.c, checked against 1.22.1)
//! asks the user itself, through the Access dialog and the permission
//! store, only for **non-interactive** requests, and then marks them
//! `permission_store_checked`. An `interactive` request reaches the
//! backend unchecked: the backend is expected to show its own UI. So this
//! backend asks for every request that is interactive or that nobody
//! checked (an older frontend, or a caller on the backend's bus name),
//! using the same Access dialog (`org.freedesktop.impl.portal.Access`
//! on the gtk backend, which `margo-portals.conf` routes by default). A
//! declined dialog is RESPONSE_CANCELLED; no dialog at all is an error,
//! never a silent capture.
//!
//! What gets captured. A non-interactive request gets the whole screen,
//! which the dialog says. An interactive one, after the dialog, opens
//! mshell's chooser (the same one the ScreenCast backend uses: Monitor /
//! Area / a window) and captures exactly the pick through the shim's
//! margo extensions (`ScreenshotOutput` / `ScreenshotToplevel` /
//! `ScreenshotRegion`). Closing the chooser is RESPONSE_CANCELLED; a
//! chooser or capture failure is RESPONSE_ERROR, never a wider shot.

use crate::mutter::ShellScreenshotProxy;
use crate::picker::{self, Selection};
use std::collections::HashMap;
use tracing::{info, warn};
use zbus::zvariant::{ObjectPath, OwnedObjectPath, OwnedValue, Value};
use zbus::{Connection, interface, proxy};

const RESPONSE_SUCCESS: u32 = 0;
const RESPONSE_CANCELLED: u32 = 1;
const RESPONSE_ERROR: u32 = 2;

/// `org.freedesktop.impl.portal.Access` — the deny/grant dialog the
/// frontend itself uses for screenshot permission. Served by
/// xdg-desktop-portal-gtk, a margo runtime dependency.
#[proxy(
    interface = "org.freedesktop.impl.portal.Access",
    default_service = "org.freedesktop.impl.portal.desktop.gtk",
    default_path = "/org/freedesktop/portal/desktop"
)]
trait Access {
    #[allow(clippy::too_many_arguments)]
    fn access_dialog(
        &self,
        handle: &ObjectPath<'_>,
        app_id: &str,
        parent_window: &str,
        title: &str,
        subtitle: &str,
        body: &str,
        options: HashMap<&str, Value<'_>>,
    ) -> zbus::Result<(u32, HashMap<String, OwnedValue>)>;
}

fn bool_option(options: &HashMap<String, OwnedValue>, key: &str) -> Option<bool> {
    options.get(key).and_then(|v| bool::try_from(v).ok())
}

/// Must the user confirm this request here? Yes when it is interactive (the
/// frontend never asks for those) or when the frontend did not say it
/// checked the screenshot permission.
fn needs_consent(options: &HashMap<String, OwnedValue>) -> bool {
    bool_option(options, "interactive").unwrap_or(false)
        || !bool_option(options, "permission_store_checked").unwrap_or(false)
}

/// How a request is served.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RequestPlan {
    /// Show the Access dialog first (see [`needs_consent`]).
    ask_consent: bool,
    /// Let the user choose the output, window or area (interactive);
    /// otherwise capture the whole screen.
    choose: bool,
}

fn request_plan(options: &HashMap<String, OwnedValue>) -> RequestPlan {
    RequestPlan {
        ask_consent: needs_consent(options),
        choose: bool_option(options, "interactive").unwrap_or(false),
    }
}

/// What the chooser's answer means: the target to capture, or the response
/// to return without capturing. An empty region is an error: it must never
/// turn into a bigger capture.
fn capture_target(choice: anyhow::Result<Option<Selection>>) -> Result<Selection, u32> {
    match choice {
        Ok(None) => Err(RESPONSE_CANCELLED),
        Err(e) => {
            warn!(error = %e, "screenshot: chooser failed");
            Err(RESPONSE_ERROR)
        }
        Ok(Some(Selection::Region { width, height, .. })) if width <= 0 || height <= 0 => {
            warn!(width, height, "screenshot: empty region picked");
            Err(RESPONSE_ERROR)
        }
        Ok(Some(selection)) => Ok(selection),
    }
}

/// The dialog body: what the app will get.
fn consent_body(interactive: bool) -> &'static str {
    if interactive {
        "You will choose a screen, a window or an area next. Only that is shared."
    } else {
        "It will get a picture of your whole screen, every window included."
    }
}

/// The portal response for a shim call's result.
fn shot_outcome(result: zbus::Result<(bool, String)>) -> (u32, HashMap<String, OwnedValue>) {
    match result {
        Ok((true, used)) => {
            let mut results = HashMap::new();
            results.insert("uri".into(), owned_str(format!("file://{used}")));
            info!(%used, "screenshot: captured");
            (RESPONSE_SUCCESS, results)
        }
        Ok((false, _)) => {
            warn!("screenshot: shim reported failure");
            (RESPONSE_ERROR, HashMap::new())
        }
        Err(e) => {
            warn!(error = %e, "screenshot: shim call failed");
            (RESPONSE_ERROR, HashMap::new())
        }
    }
}

/// What the Access dialog's answer means: `None` to go ahead, else the
/// response to return without capturing. Only an explicit grant (0)
/// proceeds; a failed dialog is an error, not a yes.
fn consent_outcome(dialog: Result<u32, ()>) -> Option<u32> {
    match dialog {
        Ok(0) => None,
        Ok(_) => Some(RESPONSE_CANCELLED),
        Err(()) => Some(RESPONSE_ERROR),
    }
}

/// The dialog title. `app_id` is empty for unsandboxed callers.
fn consent_title(app_id: &str) -> String {
    if app_id.is_empty() {
        "An app wants to take a screenshot".to_string()
    } else {
        format!("{app_id} wants to take a screenshot")
    }
}

pub struct ScreenshotBackend {
    conn: Connection,
}

impl ScreenshotBackend {
    pub fn new(conn: Connection) -> Self {
        Self { conn }
    }

    /// Show the Access dialog for this request. `Ok(response)` is the
    /// dialog's answer; `Err` means no dialog could be shown.
    async fn ask_consent(
        &self,
        handle: &OwnedObjectPath,
        app_id: &str,
        parent_window: &str,
        modal: bool,
        interactive: bool,
    ) -> Result<u32, ()> {
        let access = AccessProxy::new(&self.conn).await.map_err(|e| {
            warn!(error = %e, "screenshot: Access proxy failed; refusing");
        })?;
        let options = HashMap::from([
            ("deny_label", Value::from("Cancel")),
            ("grant_label", Value::from("Take Screenshot")),
            ("icon", Value::from("applets-screenshooter-symbolic")),
            ("modal", Value::from(modal)),
        ]);
        access
            .access_dialog(
                handle,
                app_id,
                parent_window,
                &consent_title(app_id),
                consent_body(interactive),
                "",
                options,
            )
            .await
            .map(|(response, _)| response)
            .map_err(|e| warn!(error = %e, "screenshot: Access dialog failed; refusing"))
    }
}

#[interface(name = "org.freedesktop.impl.portal.Screenshot")]
impl ScreenshotBackend {
    #[zbus(property, name = "version")]
    async fn version(&self) -> u32 {
        2
    }

    async fn screenshot(
        &self,
        handle: OwnedObjectPath,
        app_id: String,
        parent_window: String,
        options: HashMap<String, OwnedValue>,
    ) -> (u32, HashMap<String, OwnedValue>) {
        let plan = request_plan(&options);
        if plan.ask_consent {
            let modal = bool_option(&options, "modal").unwrap_or(true);
            let answer = self
                .ask_consent(&handle, &app_id, &parent_window, modal, plan.choose)
                .await;
            if let Some(response) = consent_outcome(answer) {
                info!(%app_id, response, "screenshot: not taken (no consent)");
                return (response, HashMap::new());
            }
        }
        let target = if plan.choose {
            match capture_target(picker::choose(&self.conn).await) {
                Ok(target) => Some(target),
                Err(response) => {
                    info!(%app_id, response, "screenshot: nothing chosen");
                    return (response, HashMap::new());
                }
            }
        } else {
            None
        };
        let shim = match ShellScreenshotProxy::new(&self.conn).await {
            Ok(s) => s,
            Err(e) => {
                warn!(error = %e, "screenshot: Shell.Screenshot proxy failed");
                return (RESPONSE_ERROR, HashMap::new());
            }
        };
        // The compositor picks the output path; the passed filename is a
        // hint and the used path is what comes back.
        let result = match target {
            None => {
                let runtime = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".into());
                let hint = format!("{runtime}/margo-portal-shot.png");
                shim.screenshot(false, false, &hint).await
            }
            Some(Selection::Monitor { connector }) => {
                shim.screenshot_output(&connector, false).await
            }
            Some(Selection::Window { id }) => shim.screenshot_toplevel(id, false).await,
            Some(Selection::Region {
                output,
                x,
                y,
                width,
                height,
            }) => {
                shim.screenshot_region(&output, x, y, width, height, false)
                    .await
            }
        };
        shot_outcome(result)
    }

    async fn pick_color(
        &self,
        _handle: OwnedObjectPath,
        _app_id: String,
        _parent_window: String,
        _options: HashMap<String, OwnedValue>,
    ) -> (u32, HashMap<String, OwnedValue>) {
        let shim = match ShellScreenshotProxy::new(&self.conn).await {
            Ok(s) => s,
            Err(e) => {
                warn!(error = %e, "pick_color: proxy failed");
                return (RESPONSE_ERROR, HashMap::new());
            }
        };
        match shim.pick_color().await {
            Ok(map) => {
                // Shim returns `{ "color": (ddd) }`; pass it straight on.
                let mut results = HashMap::new();
                if let Some(c) = map.get("color").and_then(|v| v.try_clone().ok()) {
                    results.insert("color".into(), c);
                }
                (RESPONSE_SUCCESS, results)
            }
            Err(e) => {
                warn!(error = %e, "pick_color: shim call failed");
                (RESPONSE_ERROR, HashMap::new())
            }
        }
    }
}

fn owned_str(s: String) -> OwnedValue {
    Value::from(s).try_to_owned().expect("String → OwnedValue")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts(pairs: &[(&str, bool)]) -> HashMap<String, OwnedValue> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), OwnedValue::from(*v)))
            .collect()
    }

    #[test]
    fn an_interactive_request_always_asks_the_user() {
        // xdg-desktop-portal shows its own Access dialog only for
        // non-interactive requests; interactive ones reach us unchecked.
        assert!(needs_consent(&opts(&[("interactive", true)])));
        assert!(needs_consent(&opts(&[
            ("interactive", true),
            ("permission_store_checked", true)
        ])));
    }

    #[test]
    fn a_non_interactive_request_the_frontend_already_cleared_does_not_ask_twice() {
        assert!(!needs_consent(&opts(&[
            ("interactive", false),
            ("permission_store_checked", true)
        ])));
        assert!(!needs_consent(&opts(&[("permission_store_checked", true)])));
    }

    #[test]
    fn a_request_nobody_checked_asks_the_user() {
        // An older frontend, or a caller that reached the backend directly.
        assert!(needs_consent(&opts(&[])));
        assert!(needs_consent(&opts(&[("permission_store_checked", false)])));
    }

    #[test]
    fn only_an_explicit_grant_proceeds() {
        assert_eq!(consent_outcome(Ok(0)), None);
        assert_eq!(consent_outcome(Ok(1)), Some(RESPONSE_CANCELLED));
        assert_eq!(consent_outcome(Ok(2)), Some(RESPONSE_CANCELLED));
        assert_eq!(consent_outcome(Err(())), Some(RESPONSE_ERROR));
    }

    #[test]
    fn the_dialog_names_the_app_or_says_an_app() {
        assert!(consent_title("org.example.Chat").contains("org.example.Chat"));
        assert!(consent_title("").contains("An app"));
    }
    #[test]
    fn an_interactive_request_lets_the_user_choose_after_consent() {
        let plan = request_plan(&opts(&[("interactive", true)]));
        assert!(plan.ask_consent && plan.choose);
        let plan = request_plan(&opts(&[
            ("interactive", true),
            ("permission_store_checked", true),
        ]));
        assert!(plan.ask_consent && plan.choose);
    }

    #[test]
    fn a_non_interactive_request_takes_the_whole_screen_without_a_chooser() {
        let plan = request_plan(&opts(&[("permission_store_checked", true)]));
        assert!(!plan.ask_consent && !plan.choose);
        // Unchecked: still asked, still whole screen.
        let plan = request_plan(&opts(&[]));
        assert!(plan.ask_consent && !plan.choose);
    }

    #[test]
    fn closing_the_chooser_cancels_and_a_broken_chooser_is_an_error() {
        assert_eq!(capture_target(Ok(None)), Err(RESPONSE_CANCELLED));
        assert_eq!(
            capture_target(Err(anyhow::anyhow!("shell gone"))),
            Err(RESPONSE_ERROR)
        );
    }

    #[test]
    fn a_picked_target_is_captured_exactly() {
        let region = Selection::Region {
            output: "DP-1".into(),
            x: 5,
            y: 6,
            width: 70,
            height: 80,
        };
        assert_eq!(capture_target(Ok(Some(region.clone()))), Ok(region));
        let win = Selection::Window { id: 9 };
        assert_eq!(capture_target(Ok(Some(win.clone()))), Ok(win));
    }

    #[test]
    fn an_empty_region_is_an_error_never_a_wider_shot() {
        for (w, h) in [(0, 10), (10, 0), (-3, 10)] {
            let region = Selection::Region {
                output: "DP-1".into(),
                x: 0,
                y: 0,
                width: w,
                height: h,
            };
            assert_eq!(capture_target(Ok(Some(region))), Err(RESPONSE_ERROR));
        }
    }

    #[test]
    fn the_dialog_says_what_will_be_shared() {
        assert!(consent_body(false).contains("whole screen"));
        assert!(!consent_body(true).contains("whole screen"));
        assert!(consent_body(true).contains("choose"));
    }

    #[test]
    fn only_a_successful_shot_returns_a_uri() {
        let (code, res) = shot_outcome(Ok((true, "/run/user/1/a.png".into())));
        assert_eq!(code, RESPONSE_SUCCESS);
        assert_eq!(
            res.get("uri").and_then(|v| <&str>::try_from(v).ok()),
            Some("file:///run/user/1/a.png")
        );
        let (code, res) = shot_outcome(Ok((false, String::new())));
        assert_eq!((code, res.is_empty()), (RESPONSE_ERROR, true));
        let (code, res) = shot_outcome(Err(zbus::Error::Failure("x".into())));
        assert_eq!((code, res.is_empty()), (RESPONSE_ERROR, true));
    }
}
