// SPDX-FileCopyrightText: 2022  Emmanuele Bassi
// SPDX-License-Identifier: GPL-3.0-or-later

use std::cell::RefCell;

use glib::clone;
use gst::prelude::*;
use gtk::{gio, glib, prelude::*, subclass::prelude::*};
use log::{debug, warn};

use crate::audio::{Controller, PlaybackState, RepeatMode, Song};

mod imp {
    use glib::{ParamSpec, ParamSpecBoolean, Value};
    use once_cell::sync::Lazy;

    use super::*;

    #[derive(Debug, Default)]
    pub struct WaveformGenerator {
        pub song: RefCell<Option<Song>>,
        pub peaks: RefCell<Option<Vec<(f64, f64)>>>,
        pub pipeline: RefCell<Option<(gst::Element, gst::bus::BusWatchGuard)>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for WaveformGenerator {
        const NAME: &'static str = "WaveformGenerator";
        type Type = super::WaveformGenerator;
    }

    impl ObjectImpl for WaveformGenerator {
        fn dispose(&self) {
            if let Some((pipeline, _bus_watch)) = self.pipeline.take() {
                pipeline.send_event(gst::event::Eos::new());
                match pipeline.set_state(gst::State::Null) {
                    Ok(_) => {}
                    Err(err) => warn!("Unable to set existing pipeline to Null state: {}", err),
                }
            }
        }

        fn properties() -> &'static [ParamSpec] {
            static PROPERTIES: Lazy<Vec<ParamSpec>> =
                Lazy::new(|| vec![ParamSpecBoolean::builder("has-peaks").read_only().build()]);

            PROPERTIES.as_ref()
        }

        // Only registered pspecs reach this: the `_` arm is unreachable.
        fn property(&self, _id: usize, pspec: &ParamSpec) -> Value {
            match pspec.name() {
                "has-peaks" => self.obj().peaks().is_some().to_value(),
                _ => unimplemented!(),
            }
        }
    }
}

glib::wrapper! {
    pub struct WaveformGenerator(ObjectSubclass<imp::WaveformGenerator>);
}

impl Default for WaveformGenerator {
    fn default() -> Self {
        glib::Object::new()
    }
}

impl Controller for WaveformGenerator {
    fn set_playback_state(&self, _playback_state: &PlaybackState) {}

    fn set_song(&self, song: &Song) {
        self.imp().song.replace(Some(song.clone()));
        if song.is_stream() {
            // Decoding a remote stream for peaks would download a whole
            // episode twice — or a live station forever. No waveform.
            self.stop_pipeline();
            self.imp().peaks.replace(None);
            self.notify("has-peaks");
            return;
        }
        self.load_peaks();
    }

    fn set_position(&self, _position: u64, _notify: bool) {}
    fn set_repeat_mode(&self, _mode: RepeatMode) {}
}

impl WaveformGenerator {
    pub fn new() -> Self {
        WaveformGenerator::default()
    }

    pub fn peaks(&self) -> Option<Vec<(f64, f64)>> {
        (*self.imp().peaks.borrow()).as_ref().cloned()
    }

    fn save_peaks(&self) {
        if let Some(peaks) = self.peaks() {
            let song = match self.imp().song.borrow().as_ref() {
                Some(s) => s.clone(),
                None => {
                    self.notify("has-peaks");
                    return;
                }
            };

            if let Some(uuid) = song.uuid() {
                let mut cache = glib::user_cache_dir();
                cache.push("mtune");
                cache.push("waveforms");
                glib::mkdir_with_parents(&cache, 0o755);

                cache.push(format!("{}.json", uuid));

                // A Vec<(f64, f64)> always serializes (NaN becomes null).
                let Ok(j) = serde_json::to_string(&peaks) else {
                    return;
                };
                let file = gio::File::for_path(&cache);
                file.replace_contents_async(
                    j,
                    None,
                    false,
                    gio::FileCreateFlags::NONE,
                    gio::Cancellable::NONE,
                    move |_| {
                        debug!("Waveform cached at: {:?}", &cache);
                    },
                );
            }
        }

        self.notify("has-peaks");
    }

    async fn load_peaks_cache(&self, cache: impl AsRef<std::path::Path>) {
        let file = gio::File::for_path(&cache);
        if let Ok((bytes, _tag)) = file.load_contents_future().await {
            match serde_json::from_slice(&bytes[..]) {
                Ok(p) => {
                    debug!("Loaded cached waveform file");
                    self.imp().peaks.replace(Some(p));
                    self.notify("has-peaks");
                }
                Err(err) => {
                    debug!("Could not parse cached waveform file: {}", err);
                    self.generate_peaks();
                }
            }
        } else {
            debug!("Could not read cached waveform file");
            self.generate_peaks();
        }
    }

    fn load_peaks(&self) {
        let song = match self.imp().song.borrow().as_ref() {
            Some(s) => s.clone(),
            None => return,
        };

        if let Some(uuid) = song.uuid() {
            let mut cache = glib::user_cache_dir();
            cache.push("mtune");
            cache.push("waveforms");
            cache.push(format!("{}.json", uuid));

            let ctx = glib::MainContext::default();
            ctx.spawn_local(clone!(
                #[weak(rename_to = this)]
                self,
                async move { this.load_peaks_cache(&cache).await }
            ));
        }
    }

    fn stop_pipeline(&self) {
        if let Some((pipeline, _bus_watch)) = self.imp().pipeline.take() {
            pipeline.send_event(gst::event::Eos::new());
            if let Err(err) = pipeline.set_state(gst::State::Null) {
                warn!("Unable to set existing pipeline to Null state: {}", err);
            }
        }
    }

    fn generate_peaks(&self) {
        if let Some((pipeline, _bus_watch)) = self.imp().pipeline.take() {
            // Stop any running pipeline, and ensure that we have nothing to
            // report
            self.imp().peaks.replace(None);
            pipeline.send_event(gst::event::Eos::new());
            match pipeline.set_state(gst::State::Null) {
                Ok(_) => {}
                Err(err) => warn!("Unable to set existing pipeline to Null state: {}", err),
            }
        }

        let song = match self.imp().song.borrow().as_ref() {
            Some(s) => s.clone(),
            None => {
                self.imp().peaks.replace(None);
                self.notify("has-peaks");
                return;
            }
        };

        // Reset the peaks vector
        let peaks: Vec<(f64, f64)> = Vec::new();
        self.imp().peaks.replace(Some(peaks));

        let pipeline_str = "uridecodebin name=uridecodebin ! audioconvert ! audio/x-raw,channels=2 ! level name=level interval=250000000 ! fakesink name=faked";
        let pipeline = match gst::parse::launch(pipeline_str) {
            Ok(pipeline) => pipeline,
            Err(err) => {
                warn!("Unable to generate the waveform: {}", err);
                self.imp().peaks.replace(None);
                self.notify("has-peaks");
                return;
            }
        };

        // gst::parse::launch of a `!` chain returns a Pipeline (a Bin)
        // holding both named elements and a bus; anything else is a
        // broken GStreamer install, reported as "no waveform".
        let bin = pipeline.downcast_ref::<gst::Bin>();
        let named = |name: &str| bin.and_then(|b| b.by_name(name));
        let (Some(uridecodebin), Some(fakesink), Some(bus)) =
            (named("uridecodebin"), named("faked"), pipeline.bus())
        else {
            warn!("Unable to generate the waveform: unexpected pipeline");
            self.imp().peaks.replace(None);
            self.notify("has-peaks");
            return;
        };
        uridecodebin.set_property("uri", song.uri());
        fakesink.set_property("qos", false);
        fakesink.set_property("sync", false);

        debug!("Adding bus watch");
        let bus_watch = bus.add_watch_local(clone!(
            #[weak(rename_to = this)]
            self,
            #[weak]
            pipeline,
            #[upgrade_or]
            glib::ControlFlow::Break,
            move |_, msg| {
                use gst::MessageView;

                match msg.view() {
                    MessageView::Eos(..) => {
                        debug!("End of waveform stream");
                        if let Err(e) = pipeline.set_state(gst::State::Null) {
                            warn!("Unable to stop the waveform pipeline: {e}");
                        }
                        // We're done
                        this.imp().pipeline.replace(None);
                        this.save_peaks();
                        return glib::ControlFlow::Break;
                    }
                    MessageView::Error(err) => {
                        warn!("Pipeline error: {:?}", err);
                        if let Err(e) = pipeline.set_state(gst::State::Null) {
                            warn!("Unable to stop the waveform pipeline: {e}");
                        }
                        // We're done
                        this.imp().pipeline.replace(None);
                        this.save_peaks();
                        return glib::ControlFlow::Break;
                    }
                    MessageView::Element(element) => {
                        if let Some(s) = element.structure()
                            && s.has_name("level")
                        {
                            // Two channels (the caps filter), in dB.
                            let peak = |i: usize| {
                                let a = s.get::<&glib::ValueArray>("peak").ok()?;
                                a.get(i)?.get::<f64>().ok()
                            };
                            if let (Some(v1), Some(v2)) = (peak(0), peak(1))
                                && let Some(ref mut peaks) = *this.imp().peaks.borrow_mut()
                            {
                                // Normalize peaks between 0 and 1
                                let peak1 = f64::powf(10.0, v1 / 20.0);
                                let peak2 = f64::powf(10.0, v2 / 20.0);
                                peaks.push((peak1, peak2));
                            }
                        }
                    }
                    _ => (),
                };

                glib::ControlFlow::Continue
            }
        ));
        // Fails only when the bus already has a watch: this pipeline is new.
        let Ok(bus_watch) = bus_watch else {
            warn!("Unable to generate the waveform: bus watch");
            let _ = pipeline.set_state(gst::State::Null);
            self.imp().peaks.replace(None);
            self.notify("has-peaks");
            return;
        };

        match pipeline.set_state(gst::State::Playing) {
            Ok(_) => {
                self.imp().pipeline.replace(Some((pipeline, bus_watch)));
            }
            Err(err) => {
                warn!("Unable to generate the waveform: {}", err);
                if let Err(e) = pipeline.set_state(gst::State::Null) {
                    warn!("Unable to reset the waveform pipeline: {e}");
                }
                self.imp().peaks.replace(None);
                self.notify("has-peaks");
            }
        };
    }
}
