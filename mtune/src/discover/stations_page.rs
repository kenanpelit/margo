// SPDX-License-Identifier: GPL-3.0-or-later

//! My Stations: starred directory stations and custom stream URLs. Playing
//! one queues the whole list, so next/previous (and the Tune pill) step
//! through your stations.

use std::rc::Rc;

use adw::prelude::*;

use super::{
    Ctx, play,
    rows::{self, States},
    shared::{self, Change},
};
use crate::{
    i18n::i18n,
    radio::{Station, store::CustomError},
};

pub fn build(ctx: &Rc<Ctx>) -> gtk::Widget {
    let list = rows::boxed_list();
    let add = gtk::Button::builder()
        .child(
            &adw::ButtonContent::builder()
                .icon_name("list-add-symbolic")
                .label(i18n("Add Stream URL"))
                .build(),
        )
        .halign(gtk::Align::Center)
        .css_classes(["pill"])
        .build();
    let column = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .margin_top(12)
        .margin_bottom(18)
        .margin_start(12)
        .margin_end(12)
        .build();
    column.append(&list);
    column.append(&add);
    let scroller = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .child(
            &adw::Clamp::builder()
                .maximum_size(720)
                .child(&column)
                .build(),
        )
        .build();
    let states = Rc::new(States::new(&scroller));

    let c = ctx.clone();
    add.connect_clicked(move |b| edit_dialog(&c, b, None));
    // The empty state's button also adds.
    let c = ctx.clone();
    let anchor = add.clone();
    states.on_retry(move || edit_dialog(&c, &anchor, None));

    let render = {
        let ctx = ctx.clone();
        let list = list.clone();
        let states = states.clone();
        Rc::new(move || fill(&ctx, &list, &states))
    };
    render();
    let r = render.clone();
    ctx.listen(move |change| {
        if change == Change::Stations {
            r();
        }
    });
    states.stack.clone().upcast()
}

fn fill(ctx: &Rc<Ctx>, list: &gtk::ListBox, states: &States) {
    rows::clear(list);
    let stations = shared::radio().data.borrow().stations.clone();
    if stations.is_empty() {
        states.status(
            "starred-symbolic",
            &i18n("No Stations Yet"),
            &i18n("Star stations in Radio, or add a stream by its URL"),
            Some(&i18n("Add Stream URL")),
        );
        return;
    }
    for (index, st) in stations.iter().enumerate() {
        let mut suffixes: Vec<gtk::Widget> = Vec::new();
        if st.custom {
            let edit = rows::icon_button("document-edit-symbolic", &i18n("Edit Stream"));
            let c = ctx.clone();
            let station = st.clone();
            edit.connect_clicked(move |b| edit_dialog(&c, b, Some(&station)));
            suffixes.push(edit.upcast());
        } else {
            let star = rows::star_button(true);
            let key = st.key();
            star.connect_toggled(move |b| {
                if !b.is_active() {
                    shared::radio().data.borrow_mut().remove(&key);
                    shared::radio_changed();
                }
            });
            suffixes.push(star.upcast());
        }
        let c = ctx.clone();
        let all = stations.clone();
        list.append(&rows::station_row(st, &suffixes, move || {
            play::play_stations(&c, &all, index)
        }));
    }
    states.show_content();
}

/// Add (`existing` = None) or edit/remove a custom stream.
fn edit_dialog(ctx: &Rc<Ctx>, anchor: &impl IsA<gtk::Widget>, existing: Option<&Station>) {
    let name = adw::EntryRow::builder()
        .title(i18n("Name (optional)"))
        .build();
    let url = adw::EntryRow::builder()
        .title(i18n("Stream URL"))
        .input_purpose(gtk::InputPurpose::Url)
        .activates_default(true)
        .build();
    if let Some(st) = existing {
        name.set_text(&st.name);
        url.set_text(&st.url);
    }
    let fields = rows::boxed_list();
    fields.append(&name);
    fields.append(&url);

    let dialog = adw::AlertDialog::builder()
        .heading(if existing.is_some() {
            i18n("Edit Stream")
        } else {
            i18n("Add Stream")
        })
        .body(i18n("An http(s) Icecast, Shoutcast or HLS stream address"))
        .extra_child(&fields)
        .default_response("save")
        .close_response("cancel")
        .build();
    dialog.add_responses(&[("cancel", &i18n("_Cancel"))]);
    if existing.is_some() {
        dialog.add_response("remove", &i18n("_Remove"));
        dialog.set_response_appearance("remove", adw::ResponseAppearance::Destructive);
    }
    dialog.add_response("save", &i18n("_Save"));
    dialog.set_response_appearance("save", adw::ResponseAppearance::Suggested);

    let ctx = ctx.clone();
    let key = existing.map(Station::key);
    dialog.connect_response(None, move |_, response| {
        let lib = shared::radio();
        let result = match (response, key.as_deref()) {
            ("save", None) => lib
                .data
                .borrow_mut()
                .add_custom(&name.text(), &url.text())
                .map(|_| ()),
            ("save", Some(k)) => lib
                .data
                .borrow_mut()
                .edit_custom(k, &name.text(), &url.text()),
            ("remove", Some(k)) => {
                lib.data.borrow_mut().remove(k);
                Ok(())
            }
            _ => return,
        };
        match result {
            Ok(()) => {
                shared::radio_changed();
            }
            Err(CustomError::BadUrl) => ctx.toast(&i18n("That is not an http(s) stream URL")),
            Err(CustomError::Duplicate) => {
                ctx.toast(&i18n("That stream is already in My Stations"))
            }
        }
    });
    dialog.present(Some(anchor));
}
