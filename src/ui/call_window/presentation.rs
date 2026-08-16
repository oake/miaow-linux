use super::*;

impl CallWindow {
    pub fn handle_event(&self, event: BackendEvent) {
        match event {
            BackendEvent::RoomsLoaded {
                rooms,
                selected,
                remember_selection,
            } => {
                let persisted_id = gio::Settings::new("ke.oa.miaow")
                    .string("selected-room-id")
                    .as_str()
                    .parse::<Uuid>()
                    .ok();
                if let Some(room) = selected
                    .as_ref()
                    .filter(|room| remember_selection && Some(room.id) == persisted_id)
                {
                    self.reactions.restore_identity(&room.local_identity);
                }
                self.reactions
                    .select_identity(selected.as_ref().map(|room| room.local_identity.as_str()));
                self.rooms_loaded.set(true);
                *self.rooms.borrow_mut() = rooms;
                *self.selected.borrow_mut() = selected.clone();
                if remember_selection {
                    self.persist_selected(selected.as_ref().map(|room| room.id));
                }
                self.update_idle_inhibition(selected.is_some());
                self.rebuild_room_menu();
                if selected.is_none() && !self.join_pending.get() {
                    self.spinner.stop();
                    self.spinner.set_visible(false);
                    self.status_icon.set_visible(false);
                    self.status.set_text("Add Room");
                    self.present_add_room();
                }
            }
            BackendEvent::RoomSelected(room) => {
                self.reactions.reset_network();
                self.reactions.select_identity(Some(&room.local_identity));
                self.update_idle_inhibition(true);
                self.persist_selected(Some(room.id));
                *self.selected.borrow_mut() = Some(room);
                self.rebuild_room_menu();
            }
            BackendEvent::RoomAdded(room) => {
                self.reactions.select_identity(Some(&room.local_identity));
                self.update_idle_inhibition(true);
                self.persist_selected(Some(room.id));
                let mut rooms = self.rooms.borrow_mut();
                rooms.retain(|saved| saved.id != room.id);
                rooms.push(room.clone());
                rooms.sort_by_key(|saved| saved.remote_identity.to_lowercase());
                drop(rooms);
                *self.selected.borrow_mut() = Some(room);
                self.rebuild_room_menu();
            }
            BackendEvent::RoomDeleted { rooms, selected } => {
                self.reactions
                    .select_identity(selected.as_ref().map(|room| room.local_identity.as_str()));
                *self.rooms.borrow_mut() = rooms;
                *self.selected.borrow_mut() = selected.clone();
                self.persist_selected(selected.as_ref().map(|room| room.id));
                self.update_idle_inhibition(selected.is_some());
                self.rebuild_room_menu();
                if selected.is_none() {
                    self.present_add_room();
                }
            }
            BackendEvent::PersistenceError(error) => {
                self.spinner.stop();
                self.spinner.set_visible(false);
                self.status_icon
                    .set_icon_name(Some("dialog-warning-symbolic"));
                self.status_icon.set_visible(true);
                self.status.set_text(&error);
            }
            BackendEvent::ReactionSlots { identity, slots } => {
                self.reactions.apply_remote_slots(&identity, &slots);
                self.rebuild_context_menu();
            }
            BackendEvent::ReactionSyncFinished(number) => self.reactions.finish_sync(number),
            BackendEvent::Call(call) => self.handle_call_event(call),
            BackendEvent::ShutdownComplete => {}
        }
    }

    pub(super) fn handle_call_event(&self, event: CallEvent) {
        match event {
            CallEvent::Idle => {
                self.reactions.set_in_call(false);
                self.reactions.reset_network();
                self.remote_presence.set(RemotePresence::None);
                self.status_box.set_visible(true);
                set_animated_visible(&self.controls, false);
                self.spinner.stop();
                self.spinner.set_visible(false);
                self.status_icon.set_visible(false);
                self.status.set_text("Add Room");
            }
            CallEvent::Connecting { remote, .. } => {
                self.reactions.set_in_call(false);
                self.reactions.reset_network();
                self.remote_presence.set(RemotePresence::None);
                *self.remote_name.borrow_mut() = remote.clone();
                self.remote_muted.set(false);
                set_animated_visible(&self.remote_badge, false);
                self.status_box.set_visible(true);
                set_animated_visible(&self.controls, self.chrome_visible.get());
                self.spinner.set_visible(true);
                self.spinner.start();
                self.status_icon.set_visible(false);
                self.status.set_text(&format!("Connecting to {remote}…"));
            }
            CallEvent::Waiting { remote, .. } => {
                self.reactions.set_in_call(true);
                self.remote_presence.set(RemotePresence::Waiting);
                *self.remote_name.borrow_mut() = remote.clone();
                let has_video = self.has_remote_video();
                self.status_box.set_visible(!has_video);
                self.set_remote_badge(&format!("Waiting for {remote}…"), false);
                set_animated_visible(&self.remote_badge, has_video);
                set_animated_visible(&self.controls, self.chrome_visible.get());
                self.spinner.set_visible(true);
                self.spinner.start();
                self.status_icon.set_visible(false);
                self.status.set_text(&format!("Waiting for {remote}…"));
            }
            CallEvent::RemoteJoining { remote, .. } => {
                self.reactions.set_in_call(true);
                self.remote_presence.set(RemotePresence::Joining);
                *self.remote_name.borrow_mut() = remote.clone();
                let has_video = self.has_remote_video();
                self.status_box.set_visible(!has_video);
                self.set_remote_badge(&format!("{remote} is joining…"), false);
                set_animated_visible(&self.remote_badge, has_video);
                self.spinner.set_visible(true);
                self.spinner.start();
                self.status_icon.set_visible(false);
                self.status.set_text(&format!("{remote} is joining…"));
            }
            CallEvent::Ready {
                remote,
                remote_muted,
                ..
            } => {
                self.reactions.set_in_call(true);
                self.remote_presence.set(RemotePresence::Ready);
                *self.remote_name.borrow_mut() = remote.clone();
                self.remote_muted.set(remote_muted);
                self.spinner.stop();
                self.spinner.set_visible(false);
                self.status_icon.set_icon_name(Some(if remote_muted {
                    "microphone-disabled-symbolic"
                } else {
                    "avatar-default-symbolic"
                }));
                self.status_icon.set_visible(true);
                let has_video = self.has_remote_video();
                self.status_box.set_visible(!has_video);
                self.set_remote_badge(&remote, remote_muted);
                set_animated_visible(&self.remote_badge, has_video && remote_muted);
                self.status.set_text(&remote);
            }
            CallEvent::MicrophoneMuted(value) => self.microphone_button.set_active(value),
            CallEvent::Camera(value) => self.camera_button.set_active(!value),
            CallEvent::ScreenShare(value) => self.screen_button.set_active(value),
            CallEvent::DebugStats(text) => self.debug_stats.set_text(&text),
            CallEvent::MediaDevices {
                microphones,
                speakers,
                cameras,
                selected_microphone,
                selected_speaker,
                selected_camera,
                noise_suppression,
                echo_cancellation,
            } => {
                *self.microphones.borrow_mut() = microphones;
                *self.speakers.borrow_mut() = speakers;
                *self.cameras.borrow_mut() = cameras;
                *self.selected_microphone.borrow_mut() = selected_microphone;
                *self.selected_speaker.borrow_mut() = selected_speaker;
                *self.selected_camera.borrow_mut() = selected_camera;
                self.noise_suppression.set(noise_suppression);
                self.echo_cancellation.set(echo_cancellation);
                self.rebuild_context_menu();
            }
            CallEvent::VideoFrame {
                kind,
                width,
                height,
                rgba,
            } => {
                let bytes = glib::Bytes::from_owned(rgba);
                let texture = gtk::gdk::MemoryTexture::new(
                    width as i32,
                    height as i32,
                    // Video frames are opaque. Ignoring the alpha byte keeps
                    // another stream from showing through the main picture.
                    gtk::gdk::MemoryFormat::R8g8b8x8,
                    &bytes,
                    (width * 4) as usize,
                );
                match kind {
                    VideoKind::LocalCamera => {
                        self.local_camera.set_paintable(Some(&texture));
                        self.refresh_video_layout();
                        self.status_box.set_visible(false);
                        self.update_remote_status_badge();
                    }
                    VideoKind::LocalScreenShare => {
                        let was_missing = self.local_screen.paintable().is_none();
                        self.local_screen.set_paintable(Some(&texture));
                        if was_missing {
                            self.local_main_screen.set(true);
                        }
                        self.refresh_video_layout();
                        self.status_box.set_visible(false);
                        self.update_remote_status_badge();
                    }
                    VideoKind::Camera => {
                        let was_missing = self.remote_camera.paintable().is_none();
                        self.remote_camera.set_paintable(Some(&texture));
                        if was_missing && self.remote_screen.paintable().is_some() {
                            self.remote_pip_enabled.set(true);
                        }
                        self.refresh_video_layout();
                        self.status_box.set_visible(false);
                        self.update_remote_status_badge();
                    }
                    VideoKind::ScreenShare => {
                        let was_missing = self.remote_screen.paintable().is_none();
                        self.remote_screen.set_paintable(Some(&texture));
                        if was_missing {
                            self.remote_main_screen.set(true);
                            self.remote_pip_enabled.set(true);
                        }
                        self.refresh_video_layout();
                        self.status_box.set_visible(false);
                        self.update_remote_status_badge();
                    }
                }
            }
            CallEvent::VideoEnded(kind) => {
                self.main_video.reset_zoom();
                match kind {
                    VideoKind::LocalCamera => {
                        self.local_camera.set_paintable(gtk::gdk::Paintable::NONE);
                        self.refresh_video_layout();
                    }
                    VideoKind::LocalScreenShare => {
                        self.local_screen.set_paintable(gtk::gdk::Paintable::NONE);
                        self.local_main_screen.set(false);
                        self.refresh_video_layout();
                    }
                    VideoKind::Camera => {
                        self.remote_camera.set_paintable(gtk::gdk::Paintable::NONE);
                        self.remote_pip_enabled.set(false);
                        self.refresh_video_layout();
                    }
                    VideoKind::ScreenShare => {
                        self.remote_screen.set_paintable(gtk::gdk::Paintable::NONE);
                        self.remote_main_screen.set(false);
                        self.remote_pip_enabled.set(false);
                        self.refresh_video_layout();
                    }
                }
                if !self.has_remote_video() {
                    set_animated_visible(&self.remote_badge, false);
                    self.status_box.set_visible(true);
                }
                self.update_remote_status_badge();
            }
            CallEvent::Error(error) => {
                self.status_box.set_visible(true);
                set_animated_visible(&self.remote_badge, false);
                self.spinner.stop();
                self.spinner.set_visible(false);
                self.status_icon
                    .set_icon_name(Some("dialog-warning-symbolic"));
                self.status_icon.set_visible(true);
                self.status.set_text(&error);
            }
            CallEvent::Reaction(message) => self.reactions.handle_remote(message),
            CallEvent::ReactionFileReady { hash } => self.reactions.handle_file_ready(&hash),
        }
    }

    pub(super) fn present_add_room(&self) {
        let call = self.clone();
        present_add_room(&self.window, move |pending| {
            call.submit_pending(pending);
        });
    }

    pub fn submit_pending(&self, pending: PendingRoom) {
        self.join_pending.set(false);
        if let Some(room) = self
            .rooms
            .borrow()
            .iter()
            .find(|room| room.token == pending.token)
        {
            let _ = self.sender.try_send(BackendCommand::Select(room.id));
            return;
        }
        present_confirmation(&self.window, self.sender.clone(), pending);
    }

    pub fn rooms_are_loaded(&self) -> bool {
        self.rooms_loaded.get()
    }

    pub fn set_join_pending(&self, pending: bool) {
        self.join_pending.set(pending);
    }

    pub(super) fn persist_selected(&self, id: Option<Uuid>) {
        let value = id.map(|id| id.to_string()).unwrap_or_default();
        if let Err(error) = gio::Settings::new("ke.oa.miaow").set_string("selected-room-id", &value)
        {
            log::warn!("could not save selected room: {error}");
        }
    }

    pub(super) fn update_idle_inhibition(&self, active: bool) {
        if active && self.inhibit_cookie.get().is_none() {
            let cookie = self.app.inhibit(
                Some(&self.window),
                gtk::ApplicationInhibitFlags::IDLE | gtk::ApplicationInhibitFlags::SUSPEND,
                Some("A miaow call is active"),
            );
            self.inhibit_cookie.set(Some(cookie));
        } else if !active && let Some(cookie) = self.inhibit_cookie.take() {
            self.app.uninhibit(cookie);
        }
    }

    pub(super) fn has_remote_video(&self) -> bool {
        self.remote_camera.paintable().is_some()
            || self.remote_screen.paintable().is_some()
            || self.local_camera.paintable().is_some()
            || self.local_screen.paintable().is_some()
    }

    pub(super) fn refresh_video_layout(&self) {
        let has_camera = self.remote_camera.paintable().is_some();
        let has_screen = self.remote_screen.paintable().is_some();
        let use_screen = has_screen && (self.remote_main_screen.get() || !has_camera);
        let remote_main = if use_screen {
            self.remote_screen.paintable()
        } else {
            self.remote_camera.paintable()
        };
        let local_screen = self.local_screen.paintable();
        let local_camera = self.local_camera.paintable();
        let use_local_screen =
            local_screen.is_some() && (self.local_main_screen.get() || local_camera.is_none());
        let local_selected = if use_local_screen {
            local_screen.clone()
        } else {
            local_camera.clone()
        };
        let showing_local_camera = remote_main.is_none() && !use_local_screen;
        let main = remote_main.clone().or_else(|| local_selected.clone());
        if let Some(forward) = self.animate_stream_switch.take() {
            self.main_video
                .set_paintable_animated(main.as_ref(), showing_local_camera, forward);
        } else {
            self.main_video.set_paintable(main.as_ref());
            self.main_video.set_mirrored(showing_local_camera);
        }
        self.main_video.set_visible(main.is_some());

        let compact = self.compact.get();
        self.pip_layer.widget.set_visible(!compact);
        if !compact && remote_main.is_some() && self.local_pip_enabled.get() {
            if use_local_screen {
                self.pip_layer
                    .show_local(local_selected.as_ref(), gtk::ContentFit::Contain, false);
            } else {
                self.pip_layer
                    .show_local(local_selected.as_ref(), gtk::ContentFit::Cover, true);
            }
        } else {
            self.pip_layer
                .show_local(None::<&gdk::Paintable>, gtk::ContentFit::Cover, false);
        }
        self.pip_layer.local.set_switch_visible(
            !compact
                && self.local_pip_enabled.get()
                && remote_main.is_some()
                && local_camera.is_some()
                && local_screen.is_some(),
        );
        self.pip_layer.local.set_switch_direction(!use_local_screen);

        if !compact && has_camera && has_screen && self.remote_pip_enabled.get() {
            let pip = if use_screen {
                self.remote_camera.paintable()
            } else {
                self.remote_screen.paintable()
            };
            self.pip_layer.show_remote(
                pip.as_ref(),
                if use_screen {
                    gtk::ContentFit::Cover
                } else {
                    gtk::ContentFit::Contain
                },
            );
        } else {
            self.pip_layer
                .show_remote(None::<&gdk::Paintable>, gtk::ContentFit::Cover);
        }
        self.switch_stream.set_icon_name(if use_screen {
            "go-next-symbolic"
        } else {
            "go-previous-symbolic"
        });
        self.switch_stream.set_visible(true);
        self.reopen_pip.set_visible(!compact);
        let stream_controls_allowed =
            has_camera && has_screen && (compact || !self.remote_pip_enabled.get());
        self.stream_controls_allowed.set(stream_controls_allowed);
        set_animated_visible(
            &self.stream_controls,
            stream_controls_allowed && self.chrome_visible.get(),
        );
        self.update_local_mute_badges();
        self.update_remote_video_preference(use_screen, has_camera, has_screen);
    }

    pub(super) fn update_remote_video_preference(
        &self,
        showing_remote_screen_on_main: bool,
        has_remote_camera: bool,
        has_remote_screen: bool,
    ) {
        let preference = RemoteVideoPreference {
            popup_mode: self.compact.get(),
            showing_remote_screen_on_main,
            remote_pip_enabled: has_remote_camera
                && has_remote_screen
                && self.remote_pip_enabled.get(),
            large_window: self.window.width().min(self.window.height()) >= 400,
        };
        if self.last_remote_video_preference.replace(Some(preference)) != Some(preference) {
            let _ =
                self.sender
                    .try_send(BackendCommand::Call(CallCommand::SetRemoteVideoPreference(
                        preference,
                    )));
        }
    }

    pub(super) fn update_local_mute_badges(&self) {
        let muted = self.microphone_button.is_active();
        let local_pip_visible = self.pip_layer.local.is_visible();
        self.pip_layer
            .local
            .set_mute_visible(muted && local_pip_visible);
        if let Some(room) = self.selected.borrow().as_ref() {
            self.local_mute_name.set_text(&room.local_identity);
        }
        let allowed = muted && !local_pip_visible && self.main_video.is_visible();
        self.local_mute_badge_allowed.set(allowed);
        set_animated_visible(
            &self.local_mute_badge,
            allowed && !self.chrome_visible.get(),
        );
    }

    pub(super) fn update_remote_status_badge(&self) {
        let remote = self.remote_name.borrow();
        let visible = self.main_video.is_visible()
            && match self.remote_presence.get() {
                RemotePresence::Waiting => {
                    self.set_remote_badge(&format!("Waiting for {remote}…"), false);
                    true
                }
                RemotePresence::Joining => {
                    self.set_remote_badge(&format!("{remote} is joining…"), false);
                    true
                }
                RemotePresence::Ready if self.remote_muted.get() && !remote.is_empty() => {
                    self.set_remote_badge(&remote, true);
                    true
                }
                _ => false,
            };
        set_animated_visible(&self.remote_badge, visible);
    }

    pub(super) fn set_remote_badge(&self, text: &str, muted: bool) {
        self.remote_badge_label.set_text(text);
        self.remote_mute_icon.set_visible(muted);
    }
}
