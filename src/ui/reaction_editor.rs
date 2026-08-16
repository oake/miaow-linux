use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use adw::prelude::*;
use gtk::prelude::Cast;
use gtk::{Align, Orientation};

use crate::reaction::{
    PreparedReactionFile, ReactionImportError, ReactionStore, RecentlyHeardReaction,
    normalize_name, prepare,
};

pub fn present_reaction_editor(
    parent: &adw::ApplicationWindow,
    number: u8,
    store: Rc<RefCell<ReactionStore>>,
    recent: Option<RecentlyHeardReaction>,
    on_changed: impl Fn() + 'static,
    on_preview: impl Fn(std::path::PathBuf) + 'static,
    on_closed: impl Fn() + 'static,
) {
    let on_changed = Rc::new(on_changed);
    let on_preview = Rc::new(on_preview);
    let existing = store.borrow().prepared_file(number);
    let slot = store.borrow().slot(number).cloned();
    let was_assigned = slot.as_ref().is_some_and(|slot| slot.is_assigned());
    let original_hash = slot.as_ref().and_then(|slot| slot.hash.clone());
    let original_name = slot.as_ref().and_then(|slot| slot.name.clone());
    let original_emoji = slot.as_ref().and_then(|slot| slot.emoji.clone());
    let prepared = Rc::new(RefCell::new(existing));
    let encoding = Rc::new(Cell::new(false));

    let dialog = adw::Dialog::builder()
        .title(format!("Reaction {number}"))
        .content_width(390)
        .build();
    let toolbar = adw::ToolbarView::new();
    let header = adw::HeaderBar::new();
    header.set_show_start_title_buttons(false);
    header.set_show_end_title_buttons(false);
    toolbar.add_top_bar(&header);

    let cancel = gtk::Button::with_label("Cancel");
    header.pack_start(&cancel);
    let assign = gtk::Button::with_label(if was_assigned { "Replace" } else { "Assign" });
    assign.add_css_class("suggested-action");
    header.pack_end(&assign);

    let content = gtk::Box::new(Orientation::Vertical, 18);
    content.set_margin_top(22);
    content.set_margin_bottom(22);
    content.set_margin_start(22);
    content.set_margin_end(22);

    let title_row = gtk::Box::new(Orientation::Horizontal, 10);
    let title = gtk::Label::new(Some(&format!("Reaction {number}")));
    title.add_css_class("title-2");
    title.set_halign(Align::Start);
    title.set_hexpand(true);
    title_row.append(&title);

    let name_entry = gtk::Entry::builder()
        .placeholder_text("Name")
        .hexpand(true)
        .text(original_name.as_deref().unwrap_or_default())
        .build();
    let emoji_button = gtk::Button::builder()
        .label(original_emoji.as_deref().unwrap_or("📣"))
        .width_request(48)
        .height_request(42)
        .tooltip_text("Choose emoji")
        .build();

    if let Some(recent) = recent {
        let recent_button = gtk::Button::builder()
            .label(&recent.emoji)
            .tooltip_text(format!("Use recently heard reaction: {}", recent.name))
            .build();
        recent_button.add_css_class("pill");
        recent_button.update_property(&[gtk::accessible::Property::Label(&format!(
            "Use recently heard reaction {}",
            recent.name
        ))]);
        let name_entry = name_entry.clone();
        let emoji_button = emoji_button.clone();
        let prepared = prepared.clone();
        recent_button.connect_clicked(move |_| {
            let Some(path) = crate::reaction::cached_file_path(&recent.hash) else {
                return;
            };
            if let Some(previous) = prepared.borrow_mut().take() {
                discard_temporary(previous);
            }
            name_entry.set_text(&recent.name);
            emoji_button.set_label(&recent.emoji);
            *prepared.borrow_mut() = Some(PreparedReactionFile {
                path,
                hash: recent.hash.clone(),
                is_temporary: false,
            });
        });
        title_row.append(&recent_button);
    }
    content.append(&title_row);

    let fields = gtk::Box::new(Orientation::Horizontal, 10);
    fields.append(&name_entry);
    fields.append(&emoji_button);
    content.append(&fields);

    let chooser = gtk::EmojiChooser::new();
    chooser.set_parent(&emoji_button);
    let emoji_button_for_picker = emoji_button.clone();
    chooser.connect_emoji_picked(move |chooser, picked| {
        emoji_button_for_picker.set_label(picked);
        chooser.popdown();
    });
    let chooser_for_click = chooser.clone();
    emoji_button.connect_clicked(move |_| chooser_for_click.popup());

    let actions = gtk::Box::new(Orientation::Horizontal, 8);
    let browse = gtk::Button::with_label("Browse…");
    let preview = gtk::Button::from_icon_name("media-playback-start-symbolic");
    preview.set_tooltip_text(Some("Preview reaction"));
    let spinner = gtk::Spinner::new();
    spinner.set_visible(false);
    actions.append(&browse);
    actions.append(&preview);
    actions.append(&spinner);
    content.append(&actions);

    let error = gtk::Label::new(None);
    error.set_halign(Align::Start);
    error.set_wrap(true);
    error.add_css_class("error");
    error.set_visible(false);
    content.append(&error);

    if was_assigned {
        let footer = gtk::Box::new(Orientation::Horizontal, 8);
        let remove = gtk::Button::with_label("Remove");
        remove.add_css_class("destructive-action");
        let store_for_remove = store.clone();
        let prepared_for_remove = prepared.clone();
        let dialog_for_remove = dialog.clone();
        let on_changed_for_remove = on_changed.clone();
        remove.connect_clicked(move |_| {
            if let Some(file) = prepared_for_remove.borrow_mut().take() {
                discard_temporary(file);
            }
            store_for_remove.borrow_mut().remove(number);
            on_changed_for_remove();
            dialog_for_remove.close();
        });
        footer.append(&remove);
        content.append(&footer);
    }

    toolbar.set_content(Some(&content));
    dialog.set_child(Some(&toolbar));

    let update_actions = {
        let assign = assign.clone();
        let preview = preview.clone();
        let name_entry = name_entry.clone();
        let emoji_button = emoji_button.clone();
        let prepared = prepared.clone();
        let encoding = encoding.clone();
        let original_hash = original_hash.clone();
        let original_name = original_name.clone();
        let original_emoji = original_emoji.clone();
        move || {
            let file = prepared.borrow();
            let can_use_file = file.is_some() && !encoding.get();
            preview.set_sensitive(can_use_file);
            let trimmed = normalize_name(&name_entry.text());
            let current_emoji = emoji_button.label().unwrap_or_default();
            let can_assign = can_use_file
                && !trimmed.is_empty()
                && (!was_assigned
                    || file.as_ref().map(|file| file.hash.as_str()) != original_hash.as_deref()
                    || trimmed != original_name.clone().unwrap_or_default()
                    || current_emoji.as_str() != original_emoji.as_deref().unwrap_or_default());
            assign.set_sensitive(can_assign);
        }
    };
    let update_actions = Rc::new(update_actions);
    update_actions();
    if let Some(button) = title_row
        .last_child()
        .and_then(|child| child.downcast::<gtk::Button>().ok())
    {
        let update = update_actions.clone();
        button.connect_clicked(move |_| update());
    }

    let update_for_name = update_actions.clone();
    name_entry.connect_changed(move |entry| {
        let text = entry.text();
        if text.chars().count() > 12 {
            let trimmed: String = text.chars().take(12).collect();
            entry.set_text(&trimmed);
            entry.set_position(-1);
        }
        update_for_name();
    });
    let update_for_emoji = update_actions.clone();
    emoji_button.connect_label_notify(move |_| update_for_emoji());

    let prepared_for_preview = prepared.clone();
    preview.connect_clicked(move |_| {
        if let Some(file) = prepared_for_preview.borrow().as_ref() {
            on_preview(file.path.clone());
        }
    });

    let assign_store = store.clone();
    let assign_name = name_entry.clone();
    let assign_emoji = emoji_button.clone();
    let assign_prepared = prepared.clone();
    let assign_error = error.clone();
    let assign_dialog = dialog.clone();
    assign.connect_clicked(move |_| {
        let Some(file) = assign_prepared.borrow().clone() else {
            return;
        };
        match assign_store.borrow_mut().assign(
            number,
            &assign_name.text(),
            &assign_emoji.label().unwrap_or_default(),
            &file,
        ) {
            Ok(()) => {
                assign_prepared.borrow_mut().take();
                on_changed();
                assign_dialog.close();
            }
            Err(problem) => {
                assign_error.set_text(&problem.to_string());
                assign_error.set_visible(true);
            }
        }
    });

    let cancel_dialog = dialog.clone();
    let prepared_for_cancel = prepared.clone();
    cancel.connect_clicked(move |_| {
        if let Some(file) = prepared_for_cancel.borrow_mut().take() {
            discard_temporary(file);
        }
        cancel_dialog.close();
    });

    let prepared_for_close = prepared.clone();
    dialog.connect_closed(move |_| {
        if let Some(file) = prepared_for_close.borrow_mut().take() {
            discard_temporary(file);
        }
        on_closed();
    });

    let browse_parent = parent.clone();
    let prepared_for_browse = prepared.clone();
    let name_entry_for_browse = name_entry.clone();
    let encoding_for_browse = encoding.clone();
    let spinner_for_browse = spinner.clone();
    let error_for_browse = error.clone();
    let update_for_browse = update_actions.clone();
    browse.connect_clicked(move |_| {
        let is_temporary = prepared_for_browse
            .borrow()
            .as_ref()
            .is_some_and(|file| file.is_temporary);
        if is_temporary && let Some(file) = prepared_for_browse.borrow_mut().take() {
            discard_temporary(file);
        }
        update_for_browse();
        let should_prefill = normalize_name(&name_entry_for_browse.text()).is_empty();
        let dialog = gtk::FileDialog::builder().title("Choose Audio").build();
        let filter = gtk::FileFilter::new();
        filter.set_name(Some("Audio"));
        filter.add_mime_type("audio/*");
        let filters = gio::ListStore::new::<gtk::FileFilter>();
        filters.append(&filter);
        dialog.set_filters(Some(&filters));
        dialog.set_default_filter(Some(&filter));
        let prepared = prepared_for_browse.clone();
        let name_entry = name_entry_for_browse.clone();
        let encoding = encoding_for_browse.clone();
        let spinner = spinner_for_browse.clone();
        let error = error_for_browse.clone();
        let update = update_for_browse.clone();
        let cancellable: Option<&gio::Cancellable> = None;
        dialog.open(Some(&browse_parent), cancellable, move |result| {
            let Ok(file) = result else {
                return;
            };
            let Some(path) = file.path() else {
                error.set_text(&ReactionImportError::Unreadable.to_string());
                error.set_visible(true);
                return;
            };
            if should_prefill {
                let stem = path
                    .file_stem()
                    .and_then(|stem| stem.to_str())
                    .unwrap_or_default();
                name_entry.set_text(&normalize_name(stem));
            }
            error.set_visible(false);
            encoding.set(true);
            spinner.set_visible(true);
            spinner.start();
            update();
            let (tx, rx) = async_channel::bounded(1);
            std::thread::spawn(move || {
                let _ = tx.send_blocking(prepare(&path));
            });
            glib::spawn_future_local(async move {
                if let Ok(result) = rx.recv().await {
                    match result {
                        Ok(file) => {
                            if let Some(previous) = prepared.borrow_mut().replace(file) {
                                discard_temporary(previous);
                            }
                        }
                        Err(problem) => {
                            error.set_text(&problem.to_string());
                            error.set_visible(true);
                        }
                    }
                }
                encoding.set(false);
                spinner.stop();
                spinner.set_visible(false);
                update();
            });
        });
    });

    dialog.present(Some(parent));
    name_entry.grab_focus();
}

fn discard_temporary(file: PreparedReactionFile) {
    if file.is_temporary {
        let _ = std::fs::remove_file(file.path);
    }
}
