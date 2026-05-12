//! Workspaces module — nine tag pills for margo's 1..=9 tag set.
//!
//! Each pill is a GtkButton whose CSS class tracks the tag's current
//! state on the focused output, mirroring eww's `.0`/`.01..06`/
//! `.011..066` triplet from saimoom's `.scss`:
//!
//!   * `.ws-pill.empty`     → unoccupied (eww `.0`, dim grey)
//!   * `.ws-pill.occupied`  → has windows but not focused (`.01..06`)
//!   * `.ws-pill.focused`   → shown on the focused output (`.011..066`)
//!
//! Clicking a pill spawns `mctl dispatch view <tag>`. State is
//! refreshed twice a second by polling `state::Compositor::current()`
//! — Stage 9 will swap that for an inotify subscription.

use std::cell::RefCell;
use std::rc::Rc;

use gtk::prelude::*;
use gtk::{Box as GtkBox, Button, Orientation};

use crate::state::Compositor;

const TAG_COUNT: u8 = 9;
const POLL_MS: u32 = 500;

/// Spawn the row of nine workspace pills, attach the polling tick
/// and return the container.
pub fn build() -> GtkBox {
    let row = GtkBox::builder()
        .name("workspaces")
        .orientation(Orientation::Horizontal)
        .spacing(4)
        .build();
    row.add_css_class("module");

    let pills: Vec<Button> = (1..=TAG_COUNT)
        .map(|tag| {
            let btn = Button::builder().label(&tag.to_string()).build();
            btn.add_css_class("ws-pill");
            btn.connect_clicked(move |_| dispatch_view(tag));
            row.append(&btn);
            btn
        })
        .collect();

    // Initial paint so the bar shows something the moment it opens,
    // before the first 500 ms tick fires.
    refresh(&pills, &Compositor::current());

    // `last_state` is just a coarse "did this change?" key so we
    // don't thrash the CSS classes 2× a second when the state is
    // stable. We compare the four fields the pills depend on,
    // packed into a single tuple.
    let last: Rc<RefCell<Option<Snapshot>>> = Rc::new(RefCell::new(None));
    glib::timeout_add_local(std::time::Duration::from_millis(POLL_MS as u64), move || {
        let state = Compositor::current();
        let snap = Snapshot::from(&state);
        let mut prev = last.borrow_mut();
        if prev.as_ref() != Some(&snap) {
            refresh(&pills, &state);
            *prev = Some(snap);
        }
        glib::ControlFlow::Continue
    });

    row
}

/// Coarse equality key — `Compositor` carries `Vec<Output>` etc. that
/// don't impl Eq cheaply, so we project the bits the pills look at
/// into a `PartialEq` tuple.
#[derive(PartialEq, Eq)]
struct Snapshot {
    active_output: Option<String>,
    masks: Vec<(String, u32)>,
    windows: [u16; 9],
}

impl From<&Compositor> for Snapshot {
    fn from(c: &Compositor) -> Self {
        Self {
            active_output: c.active_output.clone(),
            masks: c
                .outputs
                .iter()
                .map(|o| (o.name.clone(), o.active_tag_mask))
                .collect(),
            windows: c.tag_window_counts,
        }
    }
}

fn refresh(pills: &[Button], state: &Compositor) {
    for (idx, pill) in pills.iter().enumerate() {
        let tag = (idx + 1) as u8;
        let focused = state.tag_active_on_focused(tag);
        let occupied_anywhere = state
            .outputs
            .iter()
            .any(|o| o.active_tag_mask & (1u32 << (tag - 1)) != 0);
        let has_windows = state.tag_windows(tag) > 0;

        pill.remove_css_class("empty");
        pill.remove_css_class("occupied");
        pill.remove_css_class("focused");
        let class = if focused {
            "focused"
        } else if occupied_anywhere || has_windows {
            "occupied"
        } else {
            "empty"
        };
        pill.add_css_class(class);
    }
}

fn dispatch_view(tag: u8) {
    let _ = std::process::Command::new("mctl")
        .args(["dispatch", "view", &tag.to_string()])
        .spawn();
}
