use adw::prelude::*;
use gtk::{Align, Orientation};
use std::rc::Rc;

use crate::room::{PendingRoom, pending_room};

pub fn present_add_room(
    parent: &adw::ApplicationWindow,
    on_pending: impl Fn(PendingRoom) + 'static,
) {
    let on_pending: Rc<dyn Fn(PendingRoom)> = Rc::new(on_pending);
    let dialog = adw::Dialog::builder()
        .title("Add Room")
        .content_width(500)
        .build();
    let toolbar = adw::ToolbarView::new();
    let header = adw::HeaderBar::new();
    header.set_show_start_title_buttons(false);
    header.set_show_end_title_buttons(false);
    toolbar.add_top_bar(&header);

    let cancel = gtk::Button::with_label("Cancel");
    header.pack_start(&cancel);
    let add = gtk::Button::with_label("Add Room");
    add.add_css_class("suggested-action");
    add.set_sensitive(false);
    header.pack_end(&add);

    let content = gtk::Box::new(Orientation::Vertical, 18);
    content.set_margin_top(24);
    content.set_margin_bottom(24);
    content.set_margin_start(24);
    content.set_margin_end(24);

    let description = gtk::Label::new(Some("Enter the LiveKit server and your room token."));
    description.set_halign(Align::Start);
    description.add_css_class("dim-label");
    content.append(&description);

    let group = adw::PreferencesGroup::new();
    let server = adw::EntryRow::builder()
        .title("Server")
        .text("call.oa.ke")
        .build();
    let token = adw::PasswordEntryRow::builder().title("Token").build();
    group.add(&server);
    group.add(&token);
    content.append(&group);

    let error = gtk::Label::new(None);
    error.set_halign(Align::Start);
    error.set_wrap(true);
    error.add_css_class("error");
    error.set_visible(false);
    content.append(&error);
    toolbar.set_content(Some(&content));
    dialog.set_child(Some(&toolbar));

    let dialog_cancel = dialog.clone();
    cancel.connect_clicked(move |_| {
        dialog_cancel.close();
    });

    let add_for_change = add.clone();
    let server_for_change = server.clone();
    let dialog_for_change = dialog.clone();
    let error_for_change = error.clone();
    let on_pending_for_change = on_pending.clone();
    token.connect_changed(move |entry| {
        add_for_change.set_sensitive(!entry.text().trim().is_empty());
        error_for_change.set_visible(false);
        if let Ok(pending) = pending_room(&server_for_change.text(), &entry.text()) {
            on_pending_for_change(pending);
            dialog_for_change.close();
        }
    });

    let dialog_add = dialog.clone();
    let server_add = server.clone();
    let token_add = token.clone();
    let on_pending_for_add = on_pending.clone();
    add.connect_clicked(
        move |_| match pending_room(&server_add.text(), &token_add.text()) {
            Ok(pending) => {
                on_pending_for_add(pending);
                dialog_add.close();
            }
            Err(problem) => {
                error.set_text(&problem.to_string());
                error.set_visible(true);
            }
        },
    );

    dialog.present(Some(parent));
    token.grab_focus();
}
