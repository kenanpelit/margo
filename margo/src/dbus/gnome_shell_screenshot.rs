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
    from_compositor: async_channel::Receiver<CompositorToScreenshot>,
}

pub enum ScreenshotToCompositor {
    TakeScreenshot { include_cursor: bool },
    PickColor(async_channel::Sender<Option<PickedColor>>),
}

pub enum CompositorToScreenshot {
    ScreenshotResult(Option<PathBuf>),
}

#[interface(name = "org.gnome.Shell.Screenshot")]
impl Screenshot {
    async fn screenshot(
        &self,
        include_cursor: bool,
        _flash: bool,
        _filename: PathBuf,
    ) -> fdo::Result<(bool, PathBuf)> {
        if let Err(err) = self
            .to_compositor
            .send(ScreenshotToCompositor::TakeScreenshot { include_cursor })
        {
            warn!("error sending TakeScreenshot to compositor: {err:?}");
            return Err(fdo::Error::Failed("internal error".to_owned()));
        }

        let filename = match self.from_compositor.recv().await {
            Ok(CompositorToScreenshot::ScreenshotResult(Some(filename))) => filename,
            Ok(CompositorToScreenshot::ScreenshotResult(None)) => {
                return Err(fdo::Error::Failed("internal error".to_owned()));
            }
            Err(err) => {
                warn!("error receiving ScreenshotResult: {err:?}");
                return Err(fdo::Error::Failed("internal error".to_owned()));
            }
        };

        Ok((true, filename))
    }

    async fn pick_color(&self) -> fdo::Result<HashMap<String, OwnedValue>> {
        let (tx, rx) = async_channel::bounded(1);
        if let Err(err) = self.to_compositor.send(ScreenshotToCompositor::PickColor(tx)) {
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
    pub fn new(
        to_compositor: calloop::channel::Sender<ScreenshotToCompositor>,
        from_compositor: async_channel::Receiver<CompositorToScreenshot>,
    ) -> Self {
        Self {
            to_compositor,
            from_compositor,
        }
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
