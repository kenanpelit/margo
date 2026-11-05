//! Turn the built-in keyboard off while an external one is in use.
//!
//! Native port of the internal-keyboard community plugin. The plugin
//! needs root, a udev rule and `sudo udevadm` to make libinput ignore
//! the device; margo owns the libinput context, so it just sets the
//! device's send-events mode to `DISABLED` (every libinput device
//! supports that mode) and back. Nothing outside the compositor changes:
//! the kernel side is untouched, so the TTY consoles keep the keyboard,
//! and the state ends with the session.
//!
//! Which device: `internal_keyboard = <name>` (the device name as a
//! window-rule style regex, or its exact `eventN` sysname); unset, the
//! keyboard on the i8042 (PS/2) bus, where virtually every laptop's
//! built-in keyboard lives. Only "typing" keyboards count — devices with
//! letter keys, not the power button, lid switch, ThinkPad hotkeys, a
//! speaker's or headset's media keys, or the TrackPoint that shares the
//! i8042 bus.
//!
//! Safety: margo never leaves you without a keyboard. Turning the
//! internal keyboard off is refused unless another typing keyboard is
//! connected, and unplugging the last external keyboard turns the
//! internal one back on. With `internal_keyboard_auto = 1` it also goes
//! off by itself when an external keyboard is plugged in (on the plug
//! edge, so switching it back on by hand sticks until the next plug).

use smithay::reexports::input::{Device, DeviceCapability, SendEventsMode};

/// `BUS_I8042` from `linux/input.h`: the PS/2 controller laptop
/// keyboards sit on.
pub const BUS_I8042: u32 = 0x11;
/// `KEY_A` / `KEY_SPACE` from `linux/input-event-codes.h`.
const KEY_A: u32 = 30;
const KEY_SPACE: u32 = 57;

/// What the decision logic needs to know about one input device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyboardInfo {
    pub name: String,
    pub sysname: String,
    pub bustype: u32,
    /// A keyboard you can type on (has letter keys and a space bar).
    pub typing: bool,
}

impl KeyboardInfo {
    /// Probe a libinput device once, when it is added: its key set via
    /// libinput, its bus type from sysfs (libinput only exposes it from
    /// 1.26, newer than margo requires). Cached in `KeyboardState`.
    pub fn of(device: &Device) -> Self {
        let typing = device.has_capability(DeviceCapability::Keyboard)
            && device.keyboard_has_key(KEY_A) == Ok(true)
            && device.keyboard_has_key(KEY_SPACE) == Ok(true);
        let sysname = device.sysname().to_string();
        Self {
            name: device.name().into_owned(),
            bustype: read_bustype_in(std::path::Path::new("/sys/class/input"), &sysname),
            sysname,
            typing,
        }
    }
}

/// `<class_dir>/<sysname>/device/id/bustype` (hex, e.g. `0011`); 0 when
/// unreadable.
pub fn read_bustype_in(class_dir: &std::path::Path, sysname: &str) -> u32 {
    if sysname.is_empty() || sysname.contains('/') {
        return 0;
    }
    std::fs::read_to_string(class_dir.join(sysname).join("device/id/bustype"))
        .ok()
        .and_then(|s| u32::from_str_radix(s.trim(), 16).ok())
        .unwrap_or(0)
}

/// Is `dev` the keyboard `internal_keyboard = pattern` names (or, with an
/// empty pattern, the i8042 one)?
pub fn is_internal(dev: &KeyboardInfo, pattern: &str) -> bool {
    if !dev.typing {
        return false;
    }
    let pattern = pattern.trim();
    if pattern.is_empty() {
        return dev.bustype == BUS_I8042;
    }
    dev.sysname == pattern || crate::state::matches_rule_text(pattern, dev.name.trim())
}

/// The connected keyboards, split the way the toggle needs them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Presence {
    /// Name of the internal keyboard, if one is connected.
    pub internal: Option<String>,
    /// Another typing keyboard is connected.
    pub external: bool,
}

pub fn presence<'a>(
    devices: impl IntoIterator<Item = &'a KeyboardInfo>,
    pattern: &str,
) -> Presence {
    let mut p = Presence::default();
    for d in devices {
        if is_internal(d, pattern) {
            p.internal.get_or_insert_with(|| d.name.trim().to_string());
        } else if d.typing {
            p.external = true;
        }
    }
    p
}

/// What the user asked for (`internal_keyboard on|off|toggle`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Request {
    On,
    Off,
    Toggle,
}

impl Request {
    pub fn parse(arg: Option<&str>) -> Option<Self> {
        match arg.map(|s| s.trim().to_ascii_lowercase()).as_deref() {
            None | Some("") | Some("toggle") => Some(Self::Toggle),
            Some("on") | Some("enable") => Some(Self::On),
            Some("off") | Some("disable") => Some(Self::Off),
            _ => None,
        }
    }
}

/// Result of a request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// The keyboard is now off (`true`) / on (`false`).
    Changed(bool),
    /// Already in the requested state.
    Unchanged,
    /// Refused: no internal keyboard found.
    NoInternal,
    /// Refused: turning it off would leave no keyboard.
    NoOtherKeyboard,
}

/// Decide a user request against the current state.
pub fn request(off: bool, req: Request, p: &Presence) -> Outcome {
    let want_off = match req {
        Request::On => false,
        Request::Off => true,
        Request::Toggle => !off,
    };
    if want_off == off {
        return Outcome::Unchanged;
    }
    if want_off {
        if p.internal.is_none() {
            return Outcome::NoInternal;
        }
        if !p.external {
            return Outcome::NoOtherKeyboard;
        }
    }
    Outcome::Changed(want_off)
}

/// The new off-state after a device was plugged in or removed, if it
/// changes: the safety re-enable when no other keyboard is left, and the
/// auto-disable on the plug edge of an external keyboard.
///
/// `settled` is false right after a device was *added*: libinput re-adds
/// every device one by one on a VT switch back, so "no other keyboard"
/// may only mean "not re-added yet". The safety re-enable then waits for
/// the settle check ([`MargoState`]'s timer) or a removal, which are
/// settled.
pub fn after_hotplug(
    off: bool,
    auto: bool,
    before: &Presence,
    now: &Presence,
    settled: bool,
) -> Option<bool> {
    if off && (!now.external || now.internal.is_none()) {
        // Never leave the session without a keyboard.
        return settled.then_some(false);
    }
    if auto && !off && now.internal.is_some() && now.external && !before.external {
        return Some(true);
    }
    None
}

/// Send-events mode override for `dev`: `DISABLED` when it is the
/// internal keyboard and that is off, else `None` (keep the configured
/// mode).
pub fn mode_override(dev: &KeyboardInfo, pattern: &str, off: bool) -> Option<SendEventsMode> {
    (off && is_internal(dev, pattern)).then_some(SendEventsMode::DISABLED)
}

use crate::state::MargoState;
use smithay::reexports::input::Device as LibinputDevice;

impl MargoState {
    /// The connected keyboards as the toggle sees them.
    pub(crate) fn internal_keyboard_presence(&self) -> Presence {
        presence(self.known_keyboards(), &self.config.internal_keyboard)
    }

    /// The cached infos of the devices libinput has now. Joined on the
    /// sysname, so the list libinput clears on a VT switch away needs no
    /// matching bookkeeping here.
    fn known_keyboards(&self) -> impl Iterator<Item = &KeyboardInfo> {
        self.input_keyboard.keyboard_infos.iter().filter(|info| {
            self.libinput_devices
                .iter()
                .any(|d| d.sysname() == info.sysname)
        })
    }

    fn cached_info(&self, device: &LibinputDevice) -> Option<&KeyboardInfo> {
        let sysname = device.sysname();
        self.input_keyboard
            .keyboard_infos
            .iter()
            .find(|info| info.sysname == sysname)
    }

    /// Push the current off-state to the matching device(s): `DISABLED`
    /// for the internal keyboard while off, its configured mode otherwise.
    fn apply_internal_keyboard(&mut self) {
        let off = self.input_keyboard.internal_keyboard_off;
        let internal: Vec<(String, Option<SendEventsMode>)> = self
            .known_keyboards()
            .filter(|info| is_internal(info, &self.config.internal_keyboard))
            .map(|info| {
                let mode = mode_override(info, &self.config.internal_keyboard, off);
                (info.sysname.clone(), mode)
            })
            .collect();
        for device in &mut self.libinput_devices {
            if let Some((_, mode)) = internal.iter().find(|(s, _)| s == device.sysname()) {
                super::apply_device(device, &self.config, *mode);
            }
        }
    }

    /// Set the off-state, apply it and publish it (`watch state` →
    /// the shell's toast and Control Center tile).
    fn set_internal_keyboard_off(&mut self, off: bool) {
        self.input_keyboard.internal_keyboard_off = off;
        self.apply_internal_keyboard();
        tracing::info!(off, "internal keyboard");
        self.mark_state_dirty();
    }

    /// Action `internal_keyboard [on|off|toggle]`. A refusal (no internal
    /// keyboard, or no other keyboard to type on) is told with a
    /// notification, since the state does not change.
    pub fn internal_keyboard_request(&mut self, req: Request) {
        let p = self.internal_keyboard_presence();
        match request(self.input_keyboard.internal_keyboard_off, req, &p) {
            Outcome::Changed(off) => self.set_internal_keyboard_off(off),
            Outcome::Unchanged => {}
            Outcome::NoInternal => notify_refusal(
                "No built-in keyboard found. Set `internal_keyboard = <device name>` in config.conf.",
            ),
            Outcome::NoOtherKeyboard => notify_refusal(
                "Connect another keyboard first: the built-in one is the only keyboard.",
            ),
        }
    }

    /// A libinput device appeared: configure it (with the internal
    /// keyboard's off-state), track it, sync its LEDs, then run the
    /// hotplug rules (auto-disable on an external keyboard's arrival).
    pub(crate) fn libinput_device_added(&mut self, device: &mut LibinputDevice) {
        let before = self.internal_keyboard_presence();
        let info = KeyboardInfo::of(device);
        let mode = mode_override(
            &info,
            &self.config.internal_keyboard,
            self.input_keyboard.internal_keyboard_off,
        );
        super::apply_device(device, &self.config, mode);
        self.libinput_devices.retain(|known| known != device);
        self.libinput_devices.push(device.clone());
        let infos = &mut self.input_keyboard.keyboard_infos;
        infos.retain(|known| known.sysname != info.sysname);
        infos.push(info);
        // A newly plugged keyboard starts with its LEDs off.
        if let Some(keyboard) = self.seat.get_keyboard() {
            self.sync_keyboard_leds(keyboard.led_state());
        }
        self.internal_keyboard_hotplug(&before, false);
    }

    /// A libinput device went away: forget it, then run the hotplug rules
    /// (the internal keyboard comes back on when no other is left).
    pub(crate) fn libinput_device_removed(&mut self, device: &LibinputDevice) {
        let before = self.internal_keyboard_presence();
        if let Some(info) = self.cached_info(device).map(|i| i.sysname.clone()) {
            self.input_keyboard
                .keyboard_infos
                .retain(|known| known.sysname != info);
        }
        self.libinput_devices.retain(|known| known != device);
        self.internal_keyboard_hotplug(&before, true);
    }

    fn internal_keyboard_hotplug(&mut self, before: &Presence, settled: bool) {
        let now = self.internal_keyboard_presence();
        if &now != before {
            self.mark_state_dirty();
        }
        if let Some(off) = after_hotplug(
            self.input_keyboard.internal_keyboard_off,
            self.config.internal_keyboard_auto,
            before,
            &now,
            settled,
        ) {
            self.set_internal_keyboard_off(off);
        }
        if !settled && self.input_keyboard.internal_keyboard_off {
            self.arm_internal_keyboard_settle_check();
        }
    }

    /// A device was added while the internal keyboard is off: check again
    /// once the burst of additions is over, and turn it back on if no
    /// other keyboard came up (unplugged while on another VT).
    fn arm_internal_keyboard_settle_check(&mut self) {
        if self.input_keyboard.internal_keyboard_settle_armed {
            return;
        }
        self.input_keyboard.internal_keyboard_settle_armed = true;
        let timer = calloop::timer::Timer::from_duration(std::time::Duration::from_secs(2));
        let armed = self.loop_handle.insert_source(timer, |_, _, state| {
            state.input_keyboard.internal_keyboard_settle_armed = false;
            let p = state.internal_keyboard_presence();
            let off = state.input_keyboard.internal_keyboard_off;
            if let Some(off) = after_hotplug(off, false, &p, &p, true) {
                state.set_internal_keyboard_off(off);
            }
            calloop::timer::TimeoutAction::Drop
        });
        if armed.is_err() {
            // No timer: decide now rather than risk staying keyboardless.
            self.input_keyboard.internal_keyboard_settle_armed = false;
            let p = self.internal_keyboard_presence();
            if let Some(off) = after_hotplug(true, false, &p, &p, true) {
                self.set_internal_keyboard_off(off);
            }
        }
    }

    /// After `reload_config` re-applied every device: re-assert the
    /// off-state (a changed `internal_keyboard` pattern may name another
    /// device, or none), and honour `internal_keyboard_auto` switched on
    /// while an external keyboard is already connected.
    pub(crate) fn internal_keyboard_after_reload(&mut self, auto_was: bool) {
        let p = self.internal_keyboard_presence();
        let off = self.input_keyboard.internal_keyboard_off;
        let none = Presence::default();
        let before = if auto_was { &p } else { &none };
        let next = after_hotplug(off, self.config.internal_keyboard_auto, before, &p, true);
        match next {
            Some(off) => self.set_internal_keyboard_off(off),
            None => self.apply_internal_keyboard(),
        }
    }

    /// `internal_keyboard` object of the state snapshot.
    pub(crate) fn internal_keyboard_json(&self) -> serde_json::Value {
        let p = self.internal_keyboard_presence();
        serde_json::json!({
            "name": p.internal,
            "disabled": self.input_keyboard.internal_keyboard_off,
            "external": p.external,
            "auto": self.config.internal_keyboard_auto,
        })
    }
}

/// Desktop notification for a refused toggle (the compositor's usual
/// path, as `session_save` uses; nothing changes, so no state toast).
fn notify_refusal(body: &str) {
    tracing::info!("internal keyboard toggle refused: {body}");
    // Tests drive the action headless; never pop a notification there.
    #[cfg(not(test))]
    let _ = crate::utils::spawn([
        "notify-send",
        "-a",
        "margo",
        "-i",
        "input-keyboard-symbolic",
        "-t",
        "3000",
        "Built-in keyboard stays on",
        body,
    ]);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `/proc/bus/input/devices` from the owner's ThinkPad (Bluetooth MACs
    /// zeroed): what libinput would enumerate, keyboards and not.
    const PROC_INPUT: &str = include_str!("testdata/proc-bus-input-devices.txt");

    /// Parse the fixture into [`KeyboardInfo`]s: `I: Bus=`, `N: Name=`,
    /// `S: Sysfs=` and the `B: KEY=` bitmap (hex words, most significant
    /// first, 64 bits each).
    fn parse(text: &str) -> Vec<KeyboardInfo> {
        let mut out = Vec::new();
        for block in text.split("\n\n").filter(|b| !b.trim().is_empty()) {
            let mut info = KeyboardInfo {
                name: String::new(),
                sysname: String::new(),
                bustype: 0,
                typing: false,
            };
            for line in block.lines() {
                if let Some(rest) = line.strip_prefix("I: Bus=") {
                    info.bustype = u32::from_str_radix(&rest[..4], 16).unwrap();
                } else if let Some(rest) = line.strip_prefix("N: Name=") {
                    info.name = rest.trim_matches('"').to_string();
                } else if let Some(rest) = line.strip_prefix("H: Handlers=") {
                    if let Some(ev) = rest.split_whitespace().find(|h| h.starts_with("event")) {
                        info.sysname = ev.to_string();
                    }
                } else if let Some(rest) = line.strip_prefix("B: KEY=") {
                    let words: Vec<u64> = rest
                        .split_whitespace()
                        .map(|w| u64::from_str_radix(w, 16).unwrap())
                        .collect();
                    let bit = |k: u32| {
                        let (word, off) = ((k / 64) as usize, k % 64);
                        words.len() > word && words[words.len() - 1 - word] & (1 << off) != 0
                    };
                    info.typing = bit(KEY_A) && bit(KEY_SPACE);
                }
            }
            out.push(info);
        }
        out
    }

    fn usb_keyboard() -> KeyboardInfo {
        KeyboardInfo {
            name: "Keychron K3 Pro".into(),
            sysname: "event23".into(),
            bustype: 0x03,
            typing: true,
        }
    }

    #[test]
    fn the_thinkpad_fixture_has_one_typing_keyboard_the_i8042_one() {
        let devices = parse(PROC_INPUT);
        assert!(devices.len() > 15, "parsed {} devices", devices.len());
        let typing: Vec<&str> = devices
            .iter()
            .filter(|d| d.typing)
            .map(|d| d.name.as_str())
            .collect();
        assert_eq!(typing, ["AT Translated Set 2 keyboard"]);
        // The TrackPoint shares the i8042 bus but is no keyboard; the
        // speaker / earbuds / hotkeys have keys but no letters.
        let p = presence(&devices, "");
        assert_eq!(p.internal.as_deref(), Some("AT Translated Set 2 keyboard"));
        assert!(!p.external, "media-key devices are not external keyboards");
    }

    #[test]
    fn a_pattern_picks_the_keyboard_by_name_or_sysname() {
        let devices = parse(PROC_INPUT);
        let at = devices.iter().find(|d| d.typing).unwrap();
        assert!(is_internal(at, "^AT Translated"));
        assert!(is_internal(at, &at.sysname));
        assert!(!is_internal(at, "Keychron"));
        assert!(is_internal(&usb_keyboard(), "Keychron"));
        // A non-typing device never matches, whatever the pattern.
        let lid = devices.iter().find(|d| d.name == "Lid Switch").unwrap();
        assert!(!is_internal(lid, "Lid"));
    }

    #[test]
    fn off_is_refused_without_another_keyboard() {
        let devices = parse(PROC_INPUT);
        let alone = presence(&devices, "");
        assert_eq!(
            request(false, Request::Toggle, &alone),
            Outcome::NoOtherKeyboard
        );
        assert_eq!(
            request(false, Request::Off, &alone),
            Outcome::NoOtherKeyboard
        );
        let none = presence(&devices, "Keychron");
        assert_eq!(request(false, Request::Off, &none), Outcome::NoInternal);
        // Turning it on is always allowed.
        assert_eq!(request(true, Request::On, &alone), Outcome::Changed(false));
    }

    #[test]
    fn requests_with_an_external_keyboard() {
        let mut devices = parse(PROC_INPUT);
        devices.push(usb_keyboard());
        let p = presence(&devices, "");
        assert!(p.external);
        assert_eq!(request(false, Request::Toggle, &p), Outcome::Changed(true));
        assert_eq!(request(true, Request::Toggle, &p), Outcome::Changed(false));
        assert_eq!(request(true, Request::Off, &p), Outcome::Unchanged);
        assert_eq!(request(false, Request::On, &p), Outcome::Unchanged);
    }

    #[test]
    fn unplugging_the_external_keyboard_turns_the_internal_one_back_on() {
        let base = parse(PROC_INPUT);
        let mut with = base.clone();
        with.push(usb_keyboard());
        let (before, now) = (presence(&with, ""), presence(&base, ""));
        assert_eq!(after_hotplug(true, false, &before, &now, true), Some(false));
        assert_eq!(after_hotplug(true, true, &before, &now, true), Some(false));
        assert_eq!(after_hotplug(false, true, &before, &now, true), None);
    }

    #[test]
    fn the_safety_re_enable_waits_while_devices_are_being_re_added() {
        // VT switch back: libinput re-adds the internal keyboard before
        // the external one. Unsettled → keep it off; settled with still no
        // external keyboard → on.
        let base = parse(PROC_INPUT);
        let alone = presence(&base, "");
        let empty = Presence::default();
        assert_eq!(after_hotplug(true, false, &empty, &alone, false), None);
        assert_eq!(
            after_hotplug(true, false, &alone, &alone, true),
            Some(false)
        );
    }

    #[test]
    fn auto_mode_turns_it_off_on_the_plug_edge_only() {
        let base = parse(PROC_INPUT);
        let mut with = base.clone();
        with.push(usb_keyboard());
        let (none, some) = (presence(&base, ""), presence(&with, ""));
        assert_eq!(after_hotplug(false, true, &none, &some, false), Some(true));
        assert_eq!(
            after_hotplug(false, false, &none, &some, false),
            None,
            "auto off"
        );
        // Switched back on by hand while the external one stays: a second
        // keyboard-ish device appearing doesn't flip it again.
        assert_eq!(after_hotplug(false, true, &some, &some, false), None);
    }

    #[test]
    fn bustype_is_read_from_sysfs() {
        let dir = std::env::temp_dir().join(format!("margo-ikb-{}", std::process::id()));
        let id = dir.join("event3/device/id");
        std::fs::create_dir_all(&id).unwrap();
        std::fs::write(id.join("bustype"), "0011\n").unwrap();
        assert_eq!(read_bustype_in(&dir, "event3"), BUS_I8042);
        assert_eq!(read_bustype_in(&dir, "event9"), 0, "missing");
        assert_eq!(read_bustype_in(&dir, "../event3"), 0, "no path tricks");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn request_args_parse() {
        assert_eq!(Request::parse(None), Some(Request::Toggle));
        assert_eq!(Request::parse(Some(" OFF ")), Some(Request::Off));
        assert_eq!(Request::parse(Some("enable")), Some(Request::On));
        assert_eq!(Request::parse(Some("sideways")), None);
    }

    #[test]
    fn only_the_internal_keyboard_gets_disabled() {
        let devices = parse(PROC_INPUT);
        let at = devices.iter().find(|d| d.typing).unwrap();
        assert_eq!(mode_override(at, "", true), Some(SendEventsMode::DISABLED));
        assert_eq!(mode_override(at, "", false), None);
        assert_eq!(mode_override(&usb_keyboard(), "", true), None);
    }
}
