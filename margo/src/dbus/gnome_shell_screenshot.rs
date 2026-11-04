#![allow(dead_code)]
//! `org.gnome.Shell.Screenshot` D-Bus shim.
//!
//! Direct port of niri/src/dbus/gnome_shell_screenshot.rs.
//! Backs xdp-gnome's Screenshot portal. Margo already has a
//! screenshot subprocess (`margo-screenshot`) bound on Print keys;
//! this shim is for the *programmatic* path (browser
//! screenshot APIs, GNOME-aware apps invoking the portal).

use std::collections::HashMap;
use std::path::PathBuf;

use tracing::warn;
use zbus::fdo::{self, RequestNameFlags};
use zbus::zvariant::OwnedValue;
use zbus::{interface, zvariant};

use super::Start;

/// Color picked by an interactive picker session. Margo doesn't
/// implement an in-tree color picker (the user's `screenshot`
/// scripts handle that); this struct matches niri's
/// `niri_ipc::PickedColor` shape so the wire format stays
/// portable.
#[derive(Debug, Clone, Copy)]
pub struct PickedColor {
    /// Linear RGB triple in [0, 1].
    pub rgb: [f64; 3],
}

pub struct Screenshot {
    to_compositor: calloop::channel::Sender<ScreenshotToCompositor>,
}

/// What a screenshot request captures.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShotTarget {
    /// Every output (GNOME's `Screenshot`).
    Everything,
    /// One output by connector name.
    Output(String),
    /// One window by its stable client id (the Introspect window id).
    Toplevel(u64),
    /// A rectangle in `output`'s logical coordinates.
    Region {
        output: String,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
    },
}

pub enum ScreenshotToCompositor {
    /// Each request carries its own reply channel, so two concurrent
    /// callers can never receive each other's picture.
    TakeScreenshot {
        include_cursor: bool,
        target: ShotTarget,
        reply: async_channel::Sender<Option<PathBuf>>,
    },
    PickColor(async_channel::Sender<Option<PickedColor>>),
}

/// The `grim` arguments that capture `target` and nothing more, or `None`
/// when it cannot be captured exactly (unknown output or window, a region
/// that is empty once clipped). `outputs` are `(connector, layout rect)`
/// pairs, the rect being the output's logical area (after transform and
/// scale, see `state::logical_output_rect`); `toplevel_identifier` maps a
/// window id to its ext-foreign-toplevel identifier.
///
/// A region is clipped to its own output and moved into layout
/// coordinates for `grim -g`, which maps layout coordinates to each
/// output's buffer pixels (scale and transform) itself. A window goes
/// through the toplevel capture source (`grim -T`), so whatever overlaps
/// it on screen is not in the picture.
pub fn grim_target_args(
    target: &ShotTarget,
    outputs: &[(String, crate::layout::Rect)],
    toplevel_identifier: impl Fn(u64) -> Option<String>,
) -> Option<Vec<String>> {
    let area = |name: &str| outputs.iter().find(|(n, _)| n == name).map(|(_, r)| *r);
    match target {
        ShotTarget::Everything => Some(Vec::new()),
        ShotTarget::Output(name) => {
            area(name)?;
            Some(vec!["-o".into(), name.clone()])
        }
        ShotTarget::Toplevel(id) => Some(vec!["-T".into(), toplevel_identifier(*id)?]),
        ShotTarget::Region {
            output,
            x,
            y,
            width,
            height,
        } => {
            let out = area(output)?;
            if *width <= 0 || *height <= 0 {
                return None;
            }
            let x0 = (*x).max(0);
            let y0 = (*y).max(0);
            let x1 = x.saturating_add(*width).min(out.width);
            let y1 = y.saturating_add(*height).min(out.height);
            if x1 <= x0 || y1 <= y0 {
                return None;
            }
            Some(vec![
                "-g".into(),
                format!("{},{} {}x{}", out.x + x0, out.y + y0, x1 - x0, y1 - y0),
            ])
        }
    }
}

impl Screenshot {
    async fn take(&self, include_cursor: bool, target: ShotTarget) -> fdo::Result<(bool, PathBuf)> {
        let (reply, rx) = async_channel::bounded(1);
        if let Err(err) = self
            .to_compositor
            .send(ScreenshotToCompositor::TakeScreenshot {
                include_cursor,
                target,
                reply,
            })
        {
            warn!("error sending TakeScreenshot to compositor: {err:?}");
            return Err(fdo::Error::Failed("internal error".to_owned()));
        }

        match rx.recv().await {
            Ok(Some(filename)) => Ok((true, filename)),
            Ok(None) => Err(fdo::Error::Failed("screenshot failed".to_owned())),
            Err(err) => {
                warn!("error receiving ScreenshotResult: {err:?}");
                Err(fdo::Error::Failed("internal error".to_owned()))
            }
        }
    }
}

#[interface(name = "org.gnome.Shell.Screenshot")]
impl Screenshot {
    async fn screenshot(
        &self,
        include_cursor: bool,
        _flash: bool,
        _filename: PathBuf,
    ) -> fdo::Result<(bool, PathBuf)> {
        self.take(include_cursor, ShotTarget::Everything).await
    }

    /// margo extension: one output, by connector name.
    async fn screenshot_output(
        &self,
        connector: String,
        include_cursor: bool,
    ) -> fdo::Result<(bool, PathBuf)> {
        self.take(include_cursor, ShotTarget::Output(connector))
            .await
    }

    /// margo extension: one window, by its Introspect window id.
    async fn screenshot_toplevel(
        &self,
        window_id: u64,
        include_cursor: bool,
    ) -> fdo::Result<(bool, PathBuf)> {
        self.take(include_cursor, ShotTarget::Toplevel(window_id))
            .await
    }

    /// margo extension: a rectangle in `output`'s logical coordinates,
    /// clipped to that output.
    #[allow(clippy::too_many_arguments)]
    async fn screenshot_region(
        &self,
        output: String,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        include_cursor: bool,
    ) -> fdo::Result<(bool, PathBuf)> {
        let target = ShotTarget::Region {
            output,
            x,
            y,
            width,
            height,
        };
        self.take(include_cursor, target).await
    }

    async fn pick_color(&self) -> fdo::Result<HashMap<String, OwnedValue>> {
        let (tx, rx) = async_channel::bounded(1);
        if let Err(err) = self
            .to_compositor
            .send(ScreenshotToCompositor::PickColor(tx))
        {
            warn!("error sending PickColor to compositor: {err:?}");
            return Err(fdo::Error::Failed("internal error".to_owned()));
        }

        let color = match rx.recv().await {
            Ok(Some(color)) => color,
            Ok(None) => {
                return Err(fdo::Error::Failed("no color picked".to_owned()));
            }
            Err(err) => {
                warn!("error receiving PickedColor: {err:?}");
                return Err(fdo::Error::Failed("internal error".to_owned()));
            }
        };

        let mut result = HashMap::new();
        let [r, g, b] = color.rgb;
        result.insert(
            "color".to_string(),
            zvariant::OwnedValue::try_from(zvariant::Value::from((r, g, b))).unwrap(),
        );

        Ok(result)
    }
}

impl Screenshot {
    pub fn new(to_compositor: calloop::channel::Sender<ScreenshotToCompositor>) -> Self {
        Self { to_compositor }
    }
}

impl Start for Screenshot {
    fn start(self) -> anyhow::Result<zbus::blocking::Connection> {
        let conn = zbus::blocking::Connection::session()?;
        let flags = RequestNameFlags::AllowReplacement
            | RequestNameFlags::ReplaceExisting
            | RequestNameFlags::DoNotQueue;

        conn.object_server()
            .at("/org/gnome/Shell/Screenshot", self)?;
        conn.request_name_with_flags("org.gnome.Shell.Screenshot", flags)?;

        Ok(conn)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::Rect;
    use smithay::output::{Mode, Output, PhysicalProperties, Scale, Subpixel};
    use smithay::utils::Transform;

    fn rect(x: i32, y: i32, width: i32, height: i32) -> Rect {
        Rect {
            x,
            y,
            width,
            height,
        }
    }

    fn outputs() -> Vec<(String, Rect)> {
        vec![
            ("DP-1".into(), rect(0, 0, 2560, 1440)),
            ("HDMI-A-1".into(), rect(2560, 0, 1920, 1080)),
        ]
    }

    fn no_toplevel(_: u64) -> Option<String> {
        None
    }

    fn args(v: &[&str]) -> Option<Vec<String>> {
        Some(v.iter().map(|s| (*s).to_string()).collect())
    }

    #[test]
    fn the_whole_screen_takes_no_target_arguments() {
        assert_eq!(
            grim_target_args(&ShotTarget::Everything, &outputs(), no_toplevel),
            args(&[])
        );
    }

    #[test]
    fn an_output_is_captured_by_name_only_if_it_exists() {
        assert_eq!(
            grim_target_args(
                &ShotTarget::Output("HDMI-A-1".into()),
                &outputs(),
                no_toplevel
            ),
            args(&["-o", "HDMI-A-1"])
        );
        assert_eq!(
            grim_target_args(&ShotTarget::Output("gone".into()), &outputs(), no_toplevel),
            None
        );
    }

    #[test]
    fn a_window_is_captured_through_its_toplevel_handle() {
        let ident = |id: u64| (id == 7).then(|| "abc123".to_string());
        assert_eq!(
            grim_target_args(&ShotTarget::Toplevel(7), &outputs(), ident),
            args(&["-T", "abc123"])
        );
        // An unknown window (closed meanwhile) fails, it never widens.
        assert_eq!(
            grim_target_args(&ShotTarget::Toplevel(8), &outputs(), ident),
            None
        );
    }

    fn region(output: &str, x: i32, y: i32, width: i32, height: i32) -> ShotTarget {
        ShotTarget::Region {
            output: output.into(),
            x,
            y,
            width,
            height,
        }
    }

    #[test]
    fn a_region_moves_from_output_to_layout_coordinates() {
        assert_eq!(
            grim_target_args(
                &region("HDMI-A-1", 10, 20, 300, 200),
                &outputs(),
                no_toplevel
            ),
            args(&["-g", "2570,20 300x200"])
        );
    }

    #[test]
    fn a_region_is_clipped_to_its_own_output() {
        // Spills right into nothing and up off the top.
        assert_eq!(
            grim_target_args(
                &region("HDMI-A-1", 1800, -50, 500, 150),
                &outputs(),
                no_toplevel
            ),
            args(&["-g", "4360,0 120x100"])
        );
        // On DP-1, spilling right would reach HDMI-A-1: clipped at DP-1's edge.
        assert_eq!(
            grim_target_args(&region("DP-1", 2500, 0, 200, 100), &outputs(), no_toplevel),
            args(&["-g", "2500,0 60x100"])
        );
    }

    #[test]
    fn an_empty_or_unknown_region_fails_instead_of_capturing_more() {
        for t in [
            region("DP-1", 0, 0, 0, 100),
            region("DP-1", 0, 0, 100, -1),
            region("DP-1", 3000, 0, 100, 100),
            region("gone", 0, 0, 100, 100),
        ] {
            assert_eq!(grim_target_args(&t, &outputs(), no_toplevel), None, "{t:?}");
        }
    }

    #[test]
    fn a_region_on_a_rotated_scaled_output_uses_its_logical_size() {
        // A 3840x2160 panel turned 90 degrees at scale 2 is 1080x1920 in
        // layout space. grim maps layout coordinates to buffer pixels per
        // output (scale and transform), so the region is clipped in layout
        // space: against 1080 wide and 1920 tall, not the mode's 3840x2160.
        let output = Output::new(
            "eDP-1".into(),
            PhysicalProperties {
                size: (0, 0).into(),
                subpixel: Subpixel::Unknown,
                make: "test".into(),
                model: "test".into(),
                serial_number: String::new(),
            },
        );
        let mode = Mode {
            size: (3840, 2160).into(),
            refresh: 60_000,
        };
        output.change_current_state(
            Some(mode),
            Some(Transform::_90),
            Some(Scale::Fractional(2.0)),
            None,
        );
        let area = crate::state::logical_output_rect(&output, 100, 0);
        assert_eq!((area.width, area.height), (1080, 1920));
        let outs = vec![("eDP-1".to_string(), area)];
        assert_eq!(
            grim_target_args(&region("eDP-1", 1000, 1800, 500, 500), &outs, no_toplevel),
            args(&["-g", "1100,1800 80x120"])
        );
    }
}
