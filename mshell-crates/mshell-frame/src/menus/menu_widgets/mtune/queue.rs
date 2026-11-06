use super::*;

/// Whether `title`/`artist` should show under `query` (case-insensitive
/// substring on either field; a blank query matches everything).
fn queue_row_matches(title: &str, artist: &str, query: &str) -> bool {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return true;
    }
    title.to_lowercase().contains(&q) || artist.to_lowercase().contains(&q)
}

/// Rebuild the queue row list from `m.queue_entries`, filtered by
/// `m.queue_filter`. Called after every refresh and every filter
/// keystroke — the list is small enough (a folder-first personal
/// library queue, not a virtualized thousand-row history) that a full
/// rebuild is simpler and cheap enough, same call shape as the
/// existing saved-playlist rows.
pub(super) fn rebuild_queue_rows(
    widgets: &MtuneMenuWidgetModelWidgets,
    m: &MtuneMenuWidgetModel,
    sender: &ComponentSender<MtuneMenuWidgetModel>,
) {
    while let Some(c) = widgets.queue_rows.first_child() {
        widgets.queue_rows.remove(&c);
    }

    for (i, (title, artist, duration)) in m.queue_entries.iter().enumerate() {
        if !queue_row_matches(title, artist, &m.queue_filter) {
            continue;
        }
        let idx = i as u32;
        let is_current = !m.is_live && m.current_index >= 0 && m.current_index as usize == i;

        let row = gtk::Box::builder()
            .css_classes(["mtune-queue-row"])
            .orientation(gtk::Orientation::Horizontal)
            .spacing(8)
            .build();
        if is_current {
            row.add_css_class("mtune-queue-row-current");
        }

        let num = gtk::Label::new(Some(&(i + 1).to_string()));
        num.add_css_class("mtune-queue-row-num");

        let text = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let title_label = gtk::Label::new(Some(title));
        title_label.set_xalign(0.0);
        title_label.set_ellipsize(pango::EllipsizeMode::End);
        title_label.add_css_class("mtune-queue-row-title");
        let artist_label = gtk::Label::new(Some(artist));
        artist_label.set_xalign(0.0);
        artist_label.set_ellipsize(pango::EllipsizeMode::End);
        artist_label.add_css_class("mtune-queue-row-artist");
        text.append(&title_label);
        text.append(&artist_label);
        text.set_hexpand(true);

        let dur = gtk::Label::new(Some(&format_duration(Duration::from_secs(*duration))));
        dur.add_css_class("mtune-queue-row-duration");

        let remove_btn = gtk::Button::builder()
            .css_classes(["mtune-queue-row-remove"])
            .icon_name("window-close-symbolic")
            .tooltip_text("Remove from queue")
            .valign(gtk::Align::Center)
            .build();
        let s = sender.clone();
        remove_btn.connect_clicked(move |_| s.input(MtuneMenuInput::RemoveQueueIndex(idx)));

        let content = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        content.append(&num);
        content.append(&text);
        content.append(&dur);
        let play = gtk::Button::builder()
            .child(&content)
            .hexpand(true)
            .css_classes(["ok-button-flat"])
            .build();
        mshell_common::a11y::name(&play, &format!("Play {title}"));
        mshell_common::a11y::name(&remove_btn, "Remove from queue");
        let s = sender.clone();
        play.connect_clicked(move |_| s.input(MtuneMenuInput::PlayQueueIndex(idx)));
        row.append(&play);
        row.append(&remove_btn);
        widgets.queue_rows.append(&row);
    }
}

pub(super) fn keyboard_rows(container: &gtk::Box) {
    let keys = gtk::EventControllerKey::new();
    let weak = container.downgrade();
    keys.connect_key_pressed(move |_, key, _, _| {
        let forward = match key {
            gtk::gdk::Key::Down => true,
            gtk::gdk::Key::Up => false,
            _ => return gtk::glib::Propagation::Proceed,
        };
        let Some(container) = weak.upgrade() else {
            return gtk::glib::Propagation::Proceed;
        };
        let Some(focus) = container.root().and_then(|root| root.focus()) else {
            return gtk::glib::Propagation::Proceed;
        };
        let mut row = container.first_child();
        while let Some(current) = row {
            if focus.is_ancestor(&current) || focus == current {
                let next = if forward {
                    current.next_sibling()
                } else {
                    current.prev_sibling()
                };
                if let Some(next) = next {
                    next.child_focus(gtk::DirectionType::TabForward);
                }
                return gtk::glib::Propagation::Stop;
            }
            row = current.next_sibling();
        }
        gtk::glib::Propagation::Proceed
    });
    container.add_controller(keys);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_on_title_or_artist_case_insensitively() {
        assert!(queue_row_matches("Get Lucky", "Daft Punk", "lucky"));
        assert!(queue_row_matches("Get Lucky", "Daft Punk", "DAFT"));
        assert!(!queue_row_matches("Get Lucky", "Daft Punk", "acoustic"));
    }

    #[test]
    fn blank_query_matches_everything() {
        assert!(queue_row_matches("Anything", "Anyone", ""));
        assert!(queue_row_matches("Anything", "Anyone", "   "));
    }
}
