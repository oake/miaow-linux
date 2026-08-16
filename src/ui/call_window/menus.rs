use super::*;

impl CallWindow {
    pub(super) fn rebuild_room_menu(&self) {
        let menu = self.build_room_menu();
        self.room_menu.set_menu_model(Some(&menu));
        self.rebuild_context_menu();
    }

    fn build_room_menu(&self) -> gio::Menu {
        let menu = gio::Menu::new();
        let selected_id = self.selected.borrow().as_ref().map(|room| room.id);
        for room in self.rooms.borrow().iter() {
            let label = if Some(room.id) == selected_id {
                format!("✓ {}", room.remote_identity)
            } else {
                room.remote_identity.clone()
            };
            let item = gio::MenuItem::new(Some(&label), None);
            item.set_action_and_target_value(
                Some("app.select-room"),
                Some(&room.id.to_string().to_variant()),
            );
            menu.append_item(&item);
        }
        if !self.rooms.borrow().is_empty() {
            menu.append_section(None, &gio::Menu::new());
        }
        menu.append(Some("Add Room…"), Some("app.add-room"));
        if let Some(room) = self.selected.borrow().as_ref() {
            menu.append(
                Some(&format!("Delete {}…", room.remote_identity)),
                Some("app.delete-room"),
            );
        }
        menu
    }

    pub(super) fn rebuild_context_menu(&self) {
        let menu = gio::Menu::new();
        let reactions = gio::Menu::new();
        for slot in self.reactions.store().borrow().slots() {
            let item = gio::MenuItem::new(Some(&slot.menu_label()), None);
            if slot.is_built_in() {
                item.set_action_and_target_value(Some("call.edit-builtin-reaction"), None);
            } else {
                item.set_action_and_target_value(
                    Some("call.edit-reaction"),
                    Some(&(i32::from(slot.number)).to_variant()),
                );
            }
            reactions.append_item(&item);
        }
        menu.append_submenu(Some("Reactions"), &reactions);
        menu.append_section(
            Some("Microphone"),
            &device_menu(
                &self.microphones.borrow(),
                &self.selected_microphone.borrow(),
                "call.select-microphone",
                "No microphones found",
            ),
        );
        menu.append_section(
            Some("Speaker"),
            &device_menu(
                &self.speakers.borrow(),
                &self.selected_speaker.borrow(),
                "call.select-speaker",
                "No speakers found",
            ),
        );
        menu.append_section(
            Some("Camera"),
            &device_menu(
                &self.cameras.borrow(),
                &self.selected_camera.borrow(),
                "call.select-camera",
                "No cameras found",
            ),
        );

        let settings = gio::Menu::new();
        settings.append(Some("Noise Suppression"), Some("call.noise-suppression"));
        settings.append(Some("Echo Cancellation"), Some("call.echo-cancellation"));
        settings.append(Some("Show Local PIP"), Some("call.show-local-pip"));
        settings.append(Some("Debug Stats"), Some("call.debug-stats"));
        menu.append_section(Some("Settings"), &settings);

        let rooms = self.build_room_menu();
        menu.append_submenu(Some("Rooms"), &rooms);
        self.context_menu.set_menu_model(Some(&menu));
    }
}

fn device_menu(
    devices: &[MediaDevice],
    selected: &str,
    action: &str,
    empty_label: &str,
) -> gio::Menu {
    let menu = gio::Menu::new();
    if devices.is_empty() {
        let item = gio::MenuItem::new(Some(empty_label), None);
        menu.append_item(&item);
        return menu;
    }
    let selected_index = devices.iter().position(|device| device.id == selected);
    for (index, device) in devices.iter().enumerate() {
        let label = if Some(index) == selected_index {
            format!("✓ {}", device.name)
        } else {
            device.name.clone()
        };
        let item = gio::MenuItem::new(Some(&label), None);
        item.set_action_and_target_value(Some(action), Some(&device.id.to_variant()));
        menu.append_item(&item);
    }
    menu
}
