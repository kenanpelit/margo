//! Catwalk — a CPU-reactive animated cat bar pill.
//!
//! Below `bars.widgets.catwalk.minimum_threshold` CPU busy% the cat idles;
//! above it the cat walks, the frame rate scaling up with load. Click opens
//! the CPU dashboard.
//!
//! Two sprite sets ship bundled (selected by `bars.widgets.catwalk.style`):
//! the original **noctalia** cat (4 walk + 4 idle frames) and **RunCat** —
//! the classic macOS running cat (5 run frames + a sleeping idle pose), via
//! CatWalk by Driglu4it, originally RunCat by Kyome. The SVG frames are
//! rasterised once, at the sprite's device-pixel size, into `gdk::Texture`s;
//! a frame step only swaps the paintable (re-rendered only when the size or
//! the monitor scale changes). The animation stops while the pill is unmapped
//! (bar hidden) or an exclusive-fullscreen window covers its output (margo
//! then renders no layer surfaces there — `MargoService::fullscreen_outputs`),
//! and resumes on map / when the output is uncovered.
//!
//! The walk cadence uses the RunCat easing — `5000/√(cpu+35) − 400` ms per
//! frame, floored at 30 ms — so the cat eases smoothly from a saunter to a
//! full zoom. An optional CPU-% readout (`display`) sits beside the cat,
//! severity-coloured (calm → warn → danger), and the tooltip narrates the
//! cat's mood ("Cat is ZOOMING!! · 94%").

use crate::bars::bar_widgets::sysstat::read_cpu_stat_pub;
use futures::StreamExt;
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{
    BarWidgetsStoreFields, BarsStoreFields, CatStyle, CatwalkConfig, CatwalkConfigStoreFields,
    CatwalkDisplay, ConfigStoreFields,
};
use reactive_graph::prelude::GetUntracked;
use relm4::gtk::gdk_pixbuf::Pixbuf;
use relm4::gtk::prelude::*;
use relm4::gtk::{gdk, gio, glib};
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::time::{Duration, Instant};

const NOCTALIA_IDLE: [(&str, &[u8]); 4] = [
    (
        "idle-0.svg",
        include_bytes!("catwalk_icons/my-idle-0-symbolic.svg"),
    ),
    (
        "idle-1.svg",
        include_bytes!("catwalk_icons/my-idle-1-symbolic.svg"),
    ),
    (
        "idle-2.svg",
        include_bytes!("catwalk_icons/my-idle-2-symbolic.svg"),
    ),
    (
        "idle-3.svg",
        include_bytes!("catwalk_icons/my-idle-3-symbolic.svg"),
    ),
];
const NOCTALIA_ACTIVE: [(&str, &[u8]); 4] = [
    (
        "active-0.svg",
        include_bytes!("catwalk_icons/my-active-0-symbolic.svg"),
    ),
    (
        "active-1.svg",
        include_bytes!("catwalk_icons/my-active-1-symbolic.svg"),
    ),
    (
        "active-2.svg",
        include_bytes!("catwalk_icons/my-active-2-symbolic.svg"),
    ),
    (
        "active-3.svg",
        include_bytes!("catwalk_icons/my-active-3-symbolic.svg"),
    ),
];
const RUNCAT_ACTIVE: [(&str, &[u8]); 5] = [
    (
        "runcat-active-0.svg",
        include_bytes!("catwalk_icons/runcat-active-0.svg"),
    ),
    (
        "runcat-active-1.svg",
        include_bytes!("catwalk_icons/runcat-active-1.svg"),
    ),
    (
        "runcat-active-2.svg",
        include_bytes!("catwalk_icons/runcat-active-2.svg"),
    ),
    (
        "runcat-active-3.svg",
        include_bytes!("catwalk_icons/runcat-active-3.svg"),
    ),
    (
        "runcat-active-4.svg",
        include_bytes!("catwalk_icons/runcat-active-4.svg"),
    ),
];
const RUNCAT_IDLE: [(&str, &[u8]); 1] = [(
    "runcat-idle.svg",
    include_bytes!("catwalk_icons/runcat-idle.svg"),
)];

/// Bundled sprite frames rasterised at one device-pixel size.
struct Frames {
    /// Square edge in device pixels the textures were rendered at.
    px: i32,
    noctalia_idle: Vec<gdk::Texture>,
    noctalia_active: Vec<gdk::Texture>,
    runcat_idle: Vec<gdk::Texture>,
    runcat_active: Vec<gdk::Texture>,
}

pub(crate) struct CatwalkModel {
    /// Rendered lazily on the first tick (needs the widget's scale factor).
    frames: Option<Frames>,
    frame: usize,
    /// A tick is armed. Cleared when the chain stops while unmapped.
    ticking: bool,
    cpu_percent: u32,
    prev_total: u64,
    prev_idle: u64,
    last_sample: Instant,
    /// Last (style, active, frame) painted — only re-set the image on change.
    last_key: Option<(CatStyle, bool, usize)>,
    /// Last CPU% rendered into the label/tooltip — guards needless updates.
    last_pct: Option<u32>,
    /// Last display mode applied (visibility) — guards needless updates.
    last_display: Option<CatwalkDisplay>,
    /// Last sprite size applied — guards needless `set_pixel_size`.
    last_size: Option<u32>,
    /// Outputs margo reports as covered by an exclusive-fullscreen window.
    fullscreen_outputs: Vec<String>,
}

#[derive(Debug)]
pub(crate) enum CatwalkInput {
    Tick,
    /// The pill was mapped again: restart the animation if it stopped.
    Mapped,
    /// margo's set of exclusive-fullscreen outputs changed.
    FullscreenOutputs(Vec<String>),
    Clicked,
}

#[derive(Debug)]
pub(crate) enum CatwalkOutput {
    Clicked,
}

pub(crate) struct CatwalkInit {}

#[relm4::component(pub(crate))]
impl Component for CatwalkModel {
    type CommandOutput = ();
    type Input = CatwalkInput;
    type Output = CatwalkOutput;
    type Init = CatwalkInit;

    view! {
        // Canonical bar-pill anatomy (DESIGN.md §4): outer Box + an inner
        // button carrying `.ok-button-surface .ok-bar-widget`, which is where
        // the standard transparent surface + 14%-primary hover wash come from.
        #[root]
        gtk::Box {
            add_css_class: "catwalk-bar-widget",
            set_hexpand: false,
            set_vexpand: false,

            #[name = "btn"]
            gtk::Button {
                set_css_classes: &["ok-button-surface", "ok-bar-widget"],

                #[name = "content"]
                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 4,
                    set_valign: gtk::Align::Center,

                    #[name = "img"]
                    gtk::Image {
                        set_pixel_size: 22,
                    },
                    #[name = "lbl"]
                    gtk::Label {
                        add_css_class: "catwalk-cpu-label",
                        set_visible: false,
                    },
                },

                connect_clicked[sender] => move |_| {
                    sender.input(CatwalkInput::Clicked);
                },
            },
        }
    }

    fn init(
        _params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let hide_background = config_manager()
            .config()
            .bars()
            .widgets()
            .catwalk()
            .hide_background()
            .get_untracked();

        let (prev_total, prev_idle) = read_cpu_stat_pub();

        // Self-rearming tick: the cadence varies with load (down to the 30 ms
        // RunCat floor), so we schedule each next frame from the handler rather
        // than running a fixed-interval source.
        arm_tick(&sender, 120);

        let model = CatwalkModel {
            frames: None,
            frame: 0,
            ticking: true,
            cpu_percent: 0,
            prev_total,
            prev_idle,
            last_sample: Instant::now(),
            last_key: None,
            last_pct: None,
            last_display: None,
            last_size: None,
            fullscreen_outputs: Vec::new(),
        };

        let widgets = view_output!();

        // hide_background drops the surface fill but keeps `.ok-bar-widget`,
        // so the cat floats yet still gets the standard hover wash. (This one
        // applies on the next widget rebuild, matching the other pills.)
        if hide_background {
            widgets.btn.set_css_classes(&["ok-bar-widget"]);
        }

        let map_sender = sender.clone();
        root.connect_map(move |_| map_sender.input(CatwalkInput::Mapped));

        // Follow margo's exclusive-fullscreen outputs (snapshot on subscribe).
        let input = sender.input_sender().clone();
        sender.command(move |_out, shutdown| async move {
            let mut stream = mshell_services::margo_service().fullscreen_outputs.watch();
            let shutdown_fut = shutdown.wait();
            tokio::pin!(shutdown_fut);
            loop {
                tokio::select! {
                    () = &mut shutdown_fut => return,
                    next = stream.next() => match next {
                        Some(outputs) => {
                            if input.send(CatwalkInput::FullscreenOutputs(outputs)).is_err() {
                                return;
                            }
                        }
                        None => return,
                    },
                }
            }
        });

        ComponentParts { model, widgets }
    }

    fn update_with_view(
        &mut self,
        widgets: &mut Self::Widgets,
        message: Self::Input,
        sender: ComponentSender<Self>,
        root: &Self::Root,
    ) {
        match message {
            CatwalkInput::Clicked => {
                let _ = sender.output(CatwalkOutput::Clicked);
                return;
            }
            CatwalkInput::Mapped => {
                if !self.ticking {
                    self.ticking = true;
                    arm_tick(&sender, 0);
                }
                return;
            }
            CatwalkInput::FullscreenOutputs(outputs) => {
                self.fullscreen_outputs = outputs;
                if !self.ticking
                    && root.is_mapped()
                    && !is_covered(widget_connector(root).as_deref(), &self.fullscreen_outputs)
                {
                    self.ticking = true;
                    arm_tick(&sender, 0);
                }
                return;
            }
            CatwalkInput::Tick => {
                // Nobody can see the cat (bar hidden / output gone, or an
                // exclusive-fullscreen window over this output): let the
                // chain lapse; `Mapped` / `FullscreenOutputs` restart it.
                if !root.is_mapped()
                    || (!self.fullscreen_outputs.is_empty()
                        && is_covered(widget_connector(root).as_deref(), &self.fullscreen_outputs))
                {
                    self.ticking = false;
                    return;
                }

                let cfg: CatwalkConfig = config_manager()
                    .config()
                    .bars()
                    .widgets()
                    .catwalk()
                    .get_untracked();

                // Re-sample CPU at the configured cadence (independent of the
                // animation tick, which can fire ~33×/s under full load).
                let poll = Duration::from_secs(cfg.poll_secs.clamp(1, 10) as u64);
                if self.last_sample.elapsed() >= poll {
                    let (total, idle) = read_cpu_stat_pub();
                    let dt = total.saturating_sub(self.prev_total);
                    let di = idle.saturating_sub(self.prev_idle);
                    if dt > 0 {
                        self.cpu_percent = (dt.saturating_sub(di) * 100 / dt) as u32;
                    }
                    self.prev_total = total;
                    self.prev_idle = idle;
                    self.last_sample = Instant::now();
                }

                let active = self.cpu_percent >= cfg.minimum_threshold;

                // Sprite size (guarded).
                let size = cfg.size.clamp(12, 48);
                if self.last_size != Some(size) {
                    self.last_size = Some(size);
                    widgets.img.set_pixel_size(size as i32);
                }

                // (Re)rasterise the frames when the device-pixel size changes.
                let px = size as i32 * widgets.img.scale_factor().max(1);
                if self.frames.as_ref().is_none_or(|f| f.px != px) {
                    self.frames = Some(Frames::render(px));
                    self.last_key = None;
                }

                // Advance the frame and paint it (only when the picture changes).
                if let Some(frames) = &self.frames {
                    let set = frames.set_for(cfg.style, active);
                    let len = set.len().max(1);
                    self.frame = (self.frame + 1) % len;
                    let key = (cfg.style, active, self.frame);
                    if self.last_key != Some(key) {
                        self.last_key = Some(key);
                        if let Some(texture) = set.get(self.frame % len) {
                            widgets.img.set_paintable(Some(texture));
                        }
                    }
                }

                // CPU-% label text + severity (only when the value changes).
                if self.last_pct != Some(self.cpu_percent) {
                    self.last_pct = Some(self.cpu_percent);
                    let pct = self.cpu_percent;
                    widgets.lbl.set_label(&format!("{pct}%"));
                    let classes: &[&str] = if pct >= 80 {
                        &["catwalk-cpu-label", "danger"]
                    } else if pct >= 50 {
                        &["catwalk-cpu-label", "warn"]
                    } else {
                        &["catwalk-cpu-label"]
                    };
                    widgets.lbl.set_css_classes(classes);
                    widgets.btn.set_tooltip_text(Some(&format!(
                        "{} · {pct}%",
                        mood(pct, cfg.minimum_threshold)
                    )));
                }

                // Display mode → which of {cat, %} are visible (guarded).
                if self.last_display != Some(cfg.display) {
                    self.last_display = Some(cfg.display);
                    let (show_img, show_lbl) = match cfg.display {
                        CatwalkDisplay::Icon => (true, false),
                        CatwalkDisplay::Text => (false, true),
                        CatwalkDisplay::Both => (true, true),
                    };
                    widgets.img.set_visible(show_img);
                    widgets.lbl.set_visible(show_lbl);
                }

                // Schedule the next frame: RunCat easing while active (smooth
                // saunter → zoom), a slow plod while idle.
                arm_tick(&sender, frame_interval_ms(self.cpu_percent, active));
            }
        }

        self.update_view(widgets, sender);
    }
}

/// Connector name of the monitor `widget`'s surface is on (e.g. `DP-1`),
/// `None` before it is realized.
fn widget_connector(widget: &impl IsA<gtk::Widget>) -> Option<String> {
    let surface = widget.native()?.surface()?;
    let monitor = widget.display().monitor_at_surface(&surface)?;
    monitor.connector().map(|c| c.to_string())
}

/// Whether the output `connector` is covered by an exclusive-fullscreen
/// window. An unknown output is never treated as covered, so the cat keeps
/// walking rather than freezing on a lookup miss.
fn is_covered(connector: Option<&str>, fullscreen_outputs: &[String]) -> bool {
    connector.is_some_and(|c| fullscreen_outputs.iter().any(|o| o == c))
}

/// Milliseconds until the next frame: the RunCat easing
/// `5000/√(cpu+35) − 400`, floored at 30 ms, while active; a slow plod idle.
fn frame_interval_ms(cpu_percent: u32, active: bool) -> u64 {
    if active {
        let cpu = f64::from(cpu_percent);
        (5000.0 / (cpu + 35.0).sqrt() - 400.0).ceil().max(30.0) as u64
    } else {
        280
    }
}

impl Frames {
    /// Rasterise every bundled frame to a `px`×`px` texture (aspect kept).
    fn render(px: i32) -> Self {
        let render = |frames: &[(&str, &'static [u8])]| -> Vec<gdk::Texture> {
            frames
                .iter()
                .filter_map(|(name, svg)| {
                    let texture = render_svg(svg, px).map(|p| pixbuf_texture(&p));
                    if texture.is_none() {
                        tracing::warn!("catwalk: cannot render frame {name}");
                    }
                    texture
                })
                .collect()
        };
        Self {
            px,
            noctalia_idle: render(&NOCTALIA_IDLE),
            noctalia_active: render(&NOCTALIA_ACTIVE),
            runcat_idle: render(&RUNCAT_IDLE),
            runcat_active: render(&RUNCAT_ACTIVE),
        }
    }

    fn set_for(&self, style: CatStyle, active: bool) -> &[gdk::Texture] {
        match (style, active) {
            (CatStyle::RunCat, true) => &self.runcat_active,
            (CatStyle::RunCat, false) => &self.runcat_idle,
            (CatStyle::Noctalia, true) => &self.noctalia_active,
            (CatStyle::Noctalia, false) => &self.noctalia_idle,
        }
    }
}

/// CPU-load mood line for the tooltip — bands lifted from the DMS CatWidget.
fn mood(cpu: u32, threshold: u32) -> &'static str {
    if cpu < threshold {
        "Cat is sleeping…"
    } else if cpu < 15 {
        "Cat is strolling"
    } else if cpu < 40 {
        "Cat is walking"
    } else if cpu < 70 {
        "Cat is trotting"
    } else if cpu < 90 {
        "Cat is running!"
    } else {
        "Cat is ZOOMING!!"
    }
}

/// Schedule a single `Tick` after `ms`. The chain re-arms itself from the
/// handler; if the component is gone the send simply fails and the chain ends.
fn arm_tick(sender: &ComponentSender<CatwalkModel>, ms: u64) {
    let sender = sender.clone();
    glib::timeout_add_local_once(Duration::from_millis(ms), move || {
        let _ = sender.input_sender().send(CatwalkInput::Tick);
    });
}

/// Rasterise an SVG to fit a `px`×`px` box (aspect preserved).
fn render_svg(svg: &'static [u8], px: i32) -> Option<Pixbuf> {
    let stream = gio::MemoryInputStream::from_bytes(&glib::Bytes::from_static(svg));
    Pixbuf::from_stream_at_scale(&stream, px, px, true, gio::Cancellable::NONE).ok()
}

/// Wrap a pixbuf's pixels in a texture without a conversion pass.
fn pixbuf_texture(pixbuf: &Pixbuf) -> gdk::Texture {
    let format = if pixbuf.has_alpha() {
        gdk::MemoryFormat::R8g8b8a8
    } else {
        gdk::MemoryFormat::R8g8b8
    };
    gdk::MemoryTexture::new(
        pixbuf.width(),
        pixbuf.height(),
        format,
        &pixbuf.read_pixel_bytes(),
        pixbuf.rowstride() as usize,
    )
    .upcast()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_rasterise_at_the_requested_device_size() {
        for (_, svg) in NOCTALIA_IDLE
            .iter()
            .chain(&NOCTALIA_ACTIVE)
            .chain(&RUNCAT_IDLE)
            .chain(&RUNCAT_ACTIVE)
        {
            let pixbuf = render_svg(svg, 44).expect("bundled SVG renders");
            assert_eq!(pixbuf.width().max(pixbuf.height()), 44);
        }
    }

    #[test]
    fn only_a_known_covered_output_pauses_the_cat() {
        let covered = vec!["DP-1".to_string()];
        assert!(is_covered(Some("DP-1"), &covered));
        assert!(!is_covered(Some("HDMI-A-1"), &covered));
        assert!(!is_covered(None, &covered));
        assert!(!is_covered(Some("DP-1"), &[]));
    }

    #[test]
    fn frame_interval_follows_runcat_easing() {
        assert_eq!(frame_interval_ms(0, false), 280);
        assert_eq!(frame_interval_ms(100, true), 31);
        assert!(frame_interval_ms(10, true) > frame_interval_ms(90, true));
        assert!(frame_interval_ms(100, true) >= 30);
    }
}
