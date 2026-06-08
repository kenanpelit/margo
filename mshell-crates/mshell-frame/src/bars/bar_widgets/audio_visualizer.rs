//! AudioVisualizer — cava-backed spectrum bar pill.
//!
//! Port of the noctalia `audio_visualizer` widget. Rather than open a
//! raw PipeWire monitor-capture stream + run our own FFT, we drive the
//! battle-tested `cava` CLI in `raw`/`ascii` mode and render the bar
//! heights it streams to stdout. This matches how waybar's cava module
//! works and degrades gracefully: if `cava` isn't installed (or has no
//! audio source) the pill simply shows flat bars.
//!
//! Render-only, no menu. A single long-running command task spawns
//! cava with a generated config and pushes each frame's bar values
//! onto the model; the bars are pre-built `gtk::Box`es whose height is
//! updated per frame.

use relm4::gtk::Orientation;
use relm4::gtk::prelude::{BoxExt, OrientableExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, BufReader};

/// Number of spectrum bars.
const BARS: usize = 12;
/// Per-bar pixel width.
const BAR_WIDTH: i32 = 3;
/// Tallest a bar can grow to (pill height budget).
const BAR_MAX_PX: f64 = 18.0;
/// Shortest a bar ever shows. Kept tall enough that the strip is
/// clearly visible at rest (silence / no cava) instead of collapsing
/// into an invisible 1-2px sliver.
const BAR_MIN_PX: i32 = 6;
/// cava `ascii_max_range` — the value scale of each frame sample.
const CAVA_RANGE: f64 = 100.0;

pub(crate) struct AudioVisualizerModel {
    /// Pre-built bar boxes, updated in place each frame.
    bars: Vec<gtk::Box>,
    _orientation: Orientation,
}

impl std::fmt::Debug for AudioVisualizerModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AudioVisualizerModel").finish()
    }
}

#[derive(Debug)]
pub(crate) enum AudioVisualizerInput {}

#[derive(Debug)]
pub(crate) enum AudioVisualizerOutput {}

pub(crate) struct AudioVisualizerInit {
    pub(crate) orientation: Orientation,
}

#[derive(Debug)]
pub(crate) enum AudioVisualizerCommandOutput {
    /// One frame of bar values (0..=CAVA_RANGE), one per bar.
    Frame(Vec<u16>),
}

#[relm4::component(pub)]
impl Component for AudioVisualizerModel {
    type CommandOutput = AudioVisualizerCommandOutput;
    type Input = AudioVisualizerInput;
    type Output = AudioVisualizerOutput;
    type Init = AudioVisualizerInit;

    view! {
        #[root]
        #[name = "root"]
        gtk::Box {
            add_css_class: "audio-visualizer-bar-widget",
            add_css_class: "ok-bar-widget",
            set_hexpand: model._orientation == Orientation::Vertical,
            set_vexpand: model._orientation == Orientation::Horizontal,
            set_halign: gtk::Align::Center,
            set_valign: gtk::Align::Center,

            #[name = "strip"]
            gtk::Box {
                add_css_class: "audio-visualizer-strip",
                set_orientation: Orientation::Horizontal,
                set_spacing: 2,
                set_halign: gtk::Align::Center,
                set_valign: gtk::Align::Center,
                // Reserve a stable height so the pill always occupies
                // real space (bars grow from a baseline within it).
                set_height_request: BAR_MAX_PX as i32,
            },
        }
    }

    fn init(
        params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        // Long-running cava reader. Spawns cava with a generated raw
        // config and streams each frame's bar values back.
        sender.command(|out, shutdown| async move {
            let shutdown_fut = shutdown.wait();
            tokio::pin!(shutdown_fut);
            tokio::select! {
                () = &mut shutdown_fut => {}
                _ = run_cava(out) => {}
            }
        });

        let model = AudioVisualizerModel {
            bars: Vec::with_capacity(BARS),
            _orientation: params.orientation,
        };

        let widgets = view_output!();

        // Build the bar boxes once and stash handles for per-frame
        // height updates.
        let mut bars = Vec::with_capacity(BARS);
        for _ in 0..BARS {
            let bar = gtk::Box::new(Orientation::Vertical, 0);
            bar.add_css_class("audio-visualizer-bar");
            // Grow from a shared bottom baseline like a real equalizer.
            bar.set_valign(gtk::Align::End);
            bar.set_size_request(BAR_WIDTH, BAR_MIN_PX);
            widgets.strip.append(&bar);
            bars.push(bar);
        }

        let mut model = model;
        model.bars = bars;
        ComponentParts { model, widgets }
    }

    fn update_cmd(
        &mut self,
        message: Self::CommandOutput,
        _sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            AudioVisualizerCommandOutput::Frame(values) => {
                for (bar, v) in self.bars.iter().zip(values.iter()) {
                    let frac = (*v as f64 / CAVA_RANGE).clamp(0.0, 1.0);
                    let px = (frac * BAR_MAX_PX).round() as i32;
                    bar.set_size_request(BAR_WIDTH, px.max(BAR_MIN_PX));
                }
            }
        }
    }
}

/// Generate the cava config and stream raw ascii frames until cava
/// exits (or the spawn fails — e.g. cava not installed).
async fn run_cava(out: relm4::Sender<AudioVisualizerCommandOutput>) {
    let cfg_path = match write_cava_config() {
        Some(p) => p,
        None => return,
    };

    let mut child = match tokio::process::Command::new("cava")
        .arg("-p")
        .arg(&cfg_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            tracing::info!(error = %e, "audio-visualizer: cava unavailable, pill stays flat");
            return;
        }
    };

    let Some(stdout) = child.stdout.take() else {
        return;
    };
    let mut lines = BufReader::new(stdout).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        let values: Vec<u16> = line
            .split(';')
            .filter_map(|t| t.trim().parse::<u16>().ok())
            .take(BARS)
            .collect();
        if !values.is_empty() {
            let _ = out.send(AudioVisualizerCommandOutput::Frame(values));
        }
    }
    let _ = child.kill().await;
}

/// Write a throwaway cava config tuned for raw ascii output and return
/// its path. Lives under the runtime dir so it's cleaned on logout.
fn write_cava_config() -> Option<std::path::PathBuf> {
    let dir = std::env::var_os("XDG_RUNTIME_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("mshell");
    if std::fs::create_dir_all(&dir).is_err() {
        return None;
    }
    let path = dir.join("cava-visualizer.conf");
    let cfg = format!(
        "[general]\n\
         mode = normal\n\
         framerate = 30\n\
         bars = {BARS}\n\
         \n\
         [output]\n\
         method = raw\n\
         raw_target = /dev/stdout\n\
         data_format = ascii\n\
         ascii_max_range = {max}\n",
        max = CAVA_RANGE as u32,
    );
    std::fs::write(&path, cfg).ok()?;
    Some(path)
}
