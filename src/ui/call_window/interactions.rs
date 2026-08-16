use super::*;

impl CallWindow {
    pub(super) fn install_actions(&self, app: &adw::Application) {
        let weak_window = self.window.downgrade();
        let call = self.clone();
        app.add_action_entries([ActionEntry::builder("add-room")
            .activate(move |_, _, _| {
                let Some(window) = weak_window.upgrade() else {
                    return;
                };
                let call = call.clone();
                present_add_room(&window, move |pending| {
                    call.submit_pending(pending);
                });
            })
            .build()]);

        let sender = self.sender.clone();
        let selected = self.selected.clone();
        let weak_window = self.window.downgrade();
        app.add_action_entries([ActionEntry::builder("delete-room")
            .activate(move |_, _, _| {
                let Some(room) = selected.borrow().clone() else {
                    return;
                };
                let Some(window) = weak_window.upgrade() else {
                    return;
                };
                let dialog = adw::AlertDialog::builder()
                    .heading(format!("Delete room with {}?", room.remote_identity))
                    .body("The token will be removed from your keyring.")
                    .build();
                dialog.add_response("cancel", "Cancel");
                dialog.add_response("delete", "Delete");
                dialog.set_close_response("cancel");
                dialog.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
                let sender = sender.clone();
                dialog.connect_response(None, move |_, response| {
                    if response == "delete" {
                        let _ = sender.try_send(BackendCommand::DeleteSelected);
                    }
                });
                dialog.present(Some(&window));
            })
            .build()]);

        let sender = self.sender.clone();
        app.add_action_entries([ActionEntry::builder("select-room")
            .parameter_type(Some(&String::static_variant_type()))
            .activate(move |_, _, parameter| {
                if let Some(id) = parameter
                    .and_then(|value| value.str())
                    .and_then(|value| Uuid::parse_str(value).ok())
                {
                    let _ = sender.try_send(BackendCommand::Select(id));
                }
            })
            .build()]);
    }

    pub(super) fn install_interactions(
        &self,
        header: &adw::HeaderBar,
        close_button: &gtk::Button,
        pin_top_left: &gtk::Button,
        pin_bottom_left: &gtk::Button,
        pin_bottom_right: &gtk::Button,
    ) {
        let window = self.window.clone();
        let compact = self.compact.clone();
        let controls = self.controls.clone();
        let pip_layer = self.pip_layer.widget.clone();
        let header_for_toggle = header.clone();
        let stream_controls = self.stream_controls.clone();
        let stream_controls_allowed = self.stream_controls_allowed.clone();
        let chrome_visible = self.chrome_visible.clone();
        let call_for_compact = self.clone();
        let pin_top_left_for_toggle = pin_top_left.clone();
        let pin_bottom_left_for_toggle = pin_bottom_left.clone();
        let pin_bottom_right_for_toggle = pin_bottom_right.clone();
        let close_for_toggle = close_button.clone();
        let remote_badge_for_toggle = self.remote_badge.clone();
        let local_mute_badge_for_toggle = self.local_mute_badge.clone();
        let debug_stats_for_toggle = self.debug_stats.clone();
        let settings = gio::Settings::new("ke.oa.miaow");
        let set_pinned: Rc<dyn Fn(bool)> = Rc::new(move |enabled| {
            if compact.replace(enabled) == enabled {
                return;
            }
            let corner = settings.string("pinned-corner");
            let title = if enabled {
                format!("miaow:pinned:{corner}")
            } else {
                "miaow".to_owned()
            };
            header_for_toggle.set_visible(!enabled && chrome_visible.get());
            pip_layer.set_visible(!enabled);
            pin_top_left_for_toggle.set_visible(false);
            pin_bottom_left_for_toggle.set_visible(false);
            pin_bottom_right_for_toggle.set_visible(false);
            if enabled {
                close_for_toggle.set_visible(false);
            }
            remote_badge_for_toggle.set_margin_top(if enabled { 12 } else { 22 });
            local_mute_badge_for_toggle.set_margin_end(if enabled { 12 } else { 24 });
            local_mute_badge_for_toggle.set_margin_bottom(if enabled { 12 } else { 24 });
            debug_stats_for_toggle.set_margin_top(if enabled { 10 } else { 20 });
            debug_stats_for_toggle.set_margin_start(if enabled { 10 } else { 20 });
            set_animated_visible(
                &stream_controls,
                chrome_visible.get() && stream_controls_allowed.get(),
            );
            controls.set_margin_bottom(if enabled { 12 } else { 28 });
            if enabled {
                controls.add_css_class("compact");
            } else {
                controls.remove_css_class("compact");
            }
            if enabled {
                let _ = settings.set_int("normal-width", window.width());
                let _ = settings.set_int("normal-height", window.height());
                window.set_size_request(240, 150);
            } else {
                window
                    .set_default_size(settings.int("normal-width"), settings.int("normal-height"));
                window.set_size_request(720, 450);
            }
            // Publish the compositor marker only after GTK has queued the
            // corresponding minimum-size change for its next Wayland commit.
            window.set_title(Some(&title));
            call_for_compact.refresh_video_layout();
        });

        for (button, corner) in [
            (pin_top_left, "top-left"),
            (pin_bottom_left, "bottom-left"),
            (pin_bottom_right, "bottom-right"),
        ] {
            let settings = gio::Settings::new("ke.oa.miaow");
            let set_pinned = set_pinned.clone();
            button.connect_clicked(move |_| {
                let _ = settings.set_string("pinned-corner", corner);
                set_pinned(true);
            });
        }

        let window_for_close = self.window.clone();
        close_button.connect_clicked(move |_| window_for_close.close());

        // Count two completed, stationary press/release cycles. Mouse clicks
        // and click-producing trackpad taps generate the same GDK events, while
        // a second press that turns into a drag explicitly cancels the pair.
        let fullscreen_click = Rc::new(RefCell::new(FullscreenClickState::default()));
        let fullscreen_click_for_event = fullscreen_click.clone();
        let window_for_double = self.window.clone();
        let compact_for_double = self.compact.clone();
        let set_pinned_for_double = set_pinned.clone();
        let controls_for_double = self.controls.clone();
        let streams_for_double = self.stream_controls.clone();
        let local_pip_for_double = self.pip_layer.local.clone();
        let remote_pip_for_double = self.pip_layer.remote.clone();
        let raw_clicks = gtk::EventControllerLegacy::new();
        raw_clicks.set_propagation_phase(gtk::PropagationPhase::Capture);
        raw_clicks.connect_event(move |_, event| {
            if event.event_type() != gdk::EventType::ButtonPress {
                return glib::Propagation::Proceed;
            }
            let Some(button) = event.downcast_ref::<gdk::ButtonEvent>() else {
                return glib::Propagation::Proceed;
            };
            if button.button() != gdk::BUTTON_PRIMARY {
                return glib::Propagation::Proceed;
            }
            if event
                .modifier_state()
                .contains(gdk::ModifierType::CONTROL_MASK)
            {
                let mut state = fullscreen_click_for_event.borrow_mut();
                if let Some(timer) = state.pending.take() {
                    timer.remove();
                }
                state.last_press = None;
                return glib::Propagation::Proceed;
            }
            let Some((x, y)) = event.position() else {
                return glib::Propagation::Proceed;
            };
            let interactive = window_for_double
                .pick(x, y, gtk::PickFlags::DEFAULT)
                .is_some_and(|target| {
                    is_interactive_widget(&target)
                        || is_within(&target, &controls_for_double)
                        || is_within(&target, &streams_for_double)
                        || is_within(&target, &local_pip_for_double)
                        || is_within(&target, &remote_pip_for_double)
                });
            if interactive {
                let mut state = fullscreen_click_for_event.borrow_mut();
                if let Some(timer) = state.pending.take() {
                    timer.remove();
                }
                state.last_press = None;
                return glib::Propagation::Proceed;
            }

            let time = event.time();
            let double = fullscreen_click_for_event.borrow().last_press.is_some_and(
                |(last_time, last_x, last_y)| {
                    time.wrapping_sub(last_time) <= 650 && (x - last_x).hypot(y - last_y) <= 12.0
                },
            );
            if !double {
                fullscreen_click_for_event.borrow_mut().last_press = Some((time, x, y));
                return glib::Propagation::Proceed;
            }

            let mut state = fullscreen_click_for_event.borrow_mut();
            state.last_press = None;
            if let Some(timer) = state.pending.take() {
                timer.remove();
            }
            drop(state);
            let state_for_timeout = fullscreen_click_for_event.clone();
            let window = window_for_double.clone();
            let compact = compact_for_double.clone();
            let set_pinned = set_pinned_for_double.clone();
            let timer =
                glib::timeout_add_local_once(std::time::Duration::from_millis(120), move || {
                    state_for_timeout.borrow_mut().pending.take();
                    if compact.get() {
                        set_pinned(false);
                    } else if window.is_fullscreen() {
                        window.unfullscreen();
                    } else {
                        window.fullscreen();
                    }
                });
            fullscreen_click_for_event.borrow_mut().pending = Some(timer);
            glib::Propagation::Proceed
        });
        self.window.add_controller(raw_clicks);

        // The window-level recognizer deliberately waits before acting so a
        // control reached through GTK's capture chain can cancel the pending
        // bare-video double click.
        let fullscreen_click_for_controls = fullscreen_click.clone();
        let control_clicks = gtk::GestureClick::new();
        control_clicks.set_button(gdk::BUTTON_PRIMARY);
        control_clicks.set_propagation_phase(gtk::PropagationPhase::Capture);
        control_clicks.connect_pressed(move |_, _, _, _| {
            let mut state = fullscreen_click_for_controls.borrow_mut();
            if let Some(timer) = state.pending.take() {
                timer.remove();
            }
            state.last_press = None;
        });
        self.controls.add_controller(control_clicks);

        let fullscreen_click_for_stream_controls = fullscreen_click.clone();
        let stream_control_clicks = gtk::GestureClick::new();
        stream_control_clicks.set_button(gdk::BUTTON_PRIMARY);
        stream_control_clicks.set_propagation_phase(gtk::PropagationPhase::Capture);
        stream_control_clicks.connect_pressed(move |_, _, _, _| {
            let mut state = fullscreen_click_for_stream_controls.borrow_mut();
            if let Some(timer) = state.pending.take() {
                timer.remove();
            }
            state.last_press = None;
        });
        self.stream_controls.add_controller(stream_control_clicks);

        // Do not hand a plain click to the compositor: doing so consumes the
        // click sequence on Wayland and prevents double-click fullscreen.
        // Begin the compositor move only after GTK recognizes actual motion.
        let window_for_drag = self.window.clone();
        let controls_for_drag = self.controls.clone();
        let streams_for_drag = self.stream_controls.clone();
        let local_pip_for_drag = self.pip_layer.local.clone();
        let remote_pip_for_drag = self.pip_layer.remote.clone();
        let drag_allowed = Rc::new(Cell::new(false));
        let drag_started = Rc::new(Cell::new(false));
        let window_drag = gtk::GestureDrag::new();
        window_drag.set_button(gdk::BUTTON_PRIMARY);
        window_drag.set_propagation_phase(gtk::PropagationPhase::Capture);
        let window_for_drag_begin = window_for_drag.clone();
        let drag_allowed_begin = drag_allowed.clone();
        let drag_started_begin = drag_started.clone();
        window_drag.connect_drag_begin(move |gesture, x, y| {
            drag_started_begin.set(false);
            let allowed = !gesture
                .current_event_state()
                .contains(gdk::ModifierType::CONTROL_MASK)
                && window_for_drag_begin
                    .pick(x, y, gtk::PickFlags::DEFAULT)
                    .is_some_and(|target| {
                        !is_interactive_widget(&target)
                            && !is_within(&target, &controls_for_drag)
                            && !is_within(&target, &streams_for_drag)
                            && !is_within(&target, &local_pip_for_drag)
                            && !is_within(&target, &remote_pip_for_drag)
                    });
            drag_allowed_begin.set(allowed);
        });
        let fullscreen_click_for_drag = fullscreen_click.clone();
        window_drag.connect_drag_update(move |gesture, offset_x, offset_y| {
            if offset_x.hypot(offset_y) < 8.0 {
                return;
            }
            if !drag_allowed.get() || drag_started.replace(true) {
                return;
            }
            let mut click = fullscreen_click_for_drag.borrow_mut();
            if let Some(timer) = click.pending.take() {
                timer.remove();
            }
            click.last_press = None;
            drop(click);
            let Some(event) = gesture.current_event() else {
                return;
            };
            let Some(device) = event.device() else {
                return;
            };
            let Some(toplevel) = window_for_drag
                .surface()
                .and_then(|surface| surface.downcast::<gdk::Toplevel>().ok())
            else {
                return;
            };
            let Some((surface_x, surface_y)) = event.position() else {
                return;
            };
            toplevel.begin_move(
                &device,
                gdk::BUTTON_PRIMARY as i32,
                surface_x,
                surface_y,
                event.time(),
            );
        });
        self.window.add_controller(window_drag);

        let chrome = chrome::CallChrome {
            call: self.clone(),
            header: header.clone(),
            close: close_button.clone(),
            pins: [
                pin_top_left.clone(),
                pin_bottom_left.clone(),
                pin_bottom_right.clone(),
            ],
            timer: Rc::new(RefCell::new(None)),
        };
        let pointer_position = Rc::new(Cell::new(None::<(f64, f64)>));
        let motion = gtk::EventControllerMotion::new();
        let chrome_for_enter = chrome.clone();
        let pointer_for_enter = pointer_position.clone();
        motion.connect_enter(move |_, x, y| {
            pointer_for_enter.set(Some((x, y)));
            chrome_for_enter.pointer_moved(x, y);
        });
        let chrome_for_motion = chrome.clone();
        let pointer_for_motion = pointer_position.clone();
        motion.connect_motion(move |_, x, y| {
            let moved = pointer_for_motion
                .get()
                .is_none_or(|(old_x, old_y)| (x - old_x).hypot(y - old_y) >= 1.0);
            pointer_for_motion.set(Some((x, y)));
            if moved || chrome_for_motion.is_over_control(x, y) {
                chrome_for_motion.pointer_moved(x, y);
            }
        });
        motion.connect_leave(move |_| {
            pointer_position.set(None);
            chrome.pointer_left();
        });
        self.window.add_controller(motion);

        let call = self.clone();
        self.microphone_button.connect_toggled(move |button| {
            let muted = button.is_active();
            button.set_icon_name(if muted {
                "microphone-disabled-symbolic"
            } else {
                "audio-input-microphone-symbolic"
            });
            call.update_local_mute_badges();
            let _ = call
                .sender
                .try_send(BackendCommand::Call(CallCommand::SetMicrophoneMuted(muted)));
        });
        let sender = self.sender.clone();
        self.camera_button.connect_toggled(move |button| {
            button.set_icon_name(if button.is_active() {
                "camera-disabled-symbolic"
            } else {
                "camera-web-symbolic"
            });
            let _ = sender.try_send(BackendCommand::Call(CallCommand::SetCameraEnabled(
                !button.is_active(),
            )));
        });
        let sender = self.sender.clone();
        self.screen_button.connect_toggled(move |button| {
            button.set_icon_name(if button.is_active() {
                "media-playback-stop-symbolic"
            } else {
                "video-display-symbolic"
            });
            let _ = sender.try_send(BackendCommand::Call(CallCommand::SetScreenShare(
                button.is_active(),
            )));
        });

        let microphone = self.microphone_button.clone();
        let camera = self.camera_button.clone();
        let reactions = self.reactions.clone();
        let window_for_keys = self.window.clone();
        let push_to_talk_timer = Rc::new(RefCell::new(None::<glib::SourceId>));
        let push_to_talk_timer_pressed = push_to_talk_timer.clone();
        let push_to_talk_engaged = Rc::new(Cell::new(false));
        let push_to_talk_engaged_pressed = push_to_talk_engaged.clone();
        let key = gtk::EventControllerKey::new();
        key.connect_key_pressed(move |_, keyval, keycode, state| {
            if state.intersects(
                gdk::ModifierType::CONTROL_MASK
                    | gdk::ModifierType::ALT_MASK
                    | gdk::ModifierType::SUPER_MASK,
            ) {
                return glib::Propagation::Proceed;
            }
            if reactions.editor_is_open() || is_editable_focused(&window_for_keys) {
                return glib::Propagation::Proceed;
            }
            if let Some(number) = reaction_number(keyval, keycode) {
                reactions.press_number(number);
                return glib::Propagation::Stop;
            }
            match keyval {
                gdk::Key::m => {
                    microphone.set_active(!microphone.is_active());
                    glib::Propagation::Stop
                }
                gdk::Key::v => {
                    camera.set_active(!camera.is_active());
                    glib::Propagation::Stop
                }
                gdk::Key::space => {
                    if !push_to_talk_engaged_pressed.replace(true) {
                        if let Some(timer) = push_to_talk_timer_pressed.borrow_mut().take() {
                            timer.remove();
                        }
                        microphone.set_active(false);
                    }
                    glib::Propagation::Stop
                }
                _ => glib::Propagation::Proceed,
            }
        });
        let microphone = self.microphone_button.clone();
        let reactions = self.reactions.clone();
        let push_to_talk_timer_released = push_to_talk_timer.clone();
        key.connect_key_released(move |_, keyval, keycode, _| {
            if let Some(number) = reaction_number(keyval, keycode) {
                reactions.release_number(number);
            }
            if keyval == gdk::Key::space && push_to_talk_engaged.replace(false) {
                if let Some(timer) = push_to_talk_timer.borrow_mut().take() {
                    timer.remove();
                }
                let microphone = microphone.clone();
                let timer_slot = push_to_talk_timer_released.clone();
                let timer =
                    glib::timeout_add_local(std::time::Duration::from_millis(200), move || {
                        timer_slot.borrow_mut().take();
                        microphone.set_active(true);
                        glib::ControlFlow::Break
                    });
                *push_to_talk_timer_released.borrow_mut() = Some(timer);
            }
        });
        self.window.add_controller(key);
    }

    pub(super) fn install_pip_interactions(&self) {
        let call = self.clone();
        let close_local = gtk::GestureClick::new();
        close_local.set_button(gdk::BUTTON_SECONDARY);
        close_local.connect_released(move |gesture, _, _, _| {
            if call.local_pip_enabled.get() {
                let _ = gtk::prelude::WidgetExt::activate_action(
                    &call.window,
                    "call.show-local-pip",
                    None,
                );
            }
            gesture.set_state(gtk::EventSequenceState::Claimed);
        });
        self.pip_layer.local.add_controller(close_local);

        let call = self.clone();
        let remote_click = gtk::GestureClick::new();
        remote_click.set_button(gdk::BUTTON_PRIMARY);
        remote_click.connect_released(move |gesture, presses, _, _| {
            if presses == 2
                && call.remote_camera.paintable().is_some()
                && call.remote_screen.paintable().is_some()
            {
                let show_screen = !call.remote_main_screen.get();
                call.remote_main_screen.set(show_screen);
                call.animate_stream_switch.set(Some(show_screen));
                call.remote_pip_enabled.set(true);
                call.main_video.reset_zoom();
                call.refresh_video_layout();
                gesture.set_state(gtk::EventSequenceState::Claimed);
            }
        });
        self.pip_layer.remote.add_controller(remote_click);

        let call = self.clone();
        let close = gtk::GestureClick::new();
        close.set_button(gdk::BUTTON_SECONDARY);
        close.connect_released(move |_, _, _, _| {
            call.remote_pip_enabled.set(false);
            call.refresh_video_layout();
        });
        self.pip_layer.remote.add_controller(close);
    }

    pub(super) fn install_context_menu(&self) {
        let actions = gio::SimpleActionGroup::new();

        let call = self.clone();
        let microphone =
            gio::SimpleAction::new("select-microphone", Some(&String::static_variant_type()));
        microphone.connect_activate(move |_, value| {
            let Some(id) = value.and_then(|value| value.str()) else {
                return;
            };
            *call.selected_microphone.borrow_mut() = id.to_owned();
            let _ = call
                .sender
                .try_send(BackendCommand::Call(CallCommand::SetMicrophoneDevice(
                    id.to_owned(),
                )));
            call.rebuild_context_menu();
        });
        actions.add_action(&microphone);

        let call = self.clone();
        let speaker =
            gio::SimpleAction::new("select-speaker", Some(&String::static_variant_type()));
        speaker.connect_activate(move |_, value| {
            let Some(id) = value.and_then(|value| value.str()) else {
                return;
            };
            *call.selected_speaker.borrow_mut() = id.to_owned();
            let _ = call
                .sender
                .try_send(BackendCommand::Call(CallCommand::SetSpeakerDevice(
                    id.to_owned(),
                )));
            call.rebuild_context_menu();
        });
        actions.add_action(&speaker);

        let call = self.clone();
        let camera = gio::SimpleAction::new("select-camera", Some(&String::static_variant_type()));
        camera.connect_activate(move |_, value| {
            let Some(id) = value.and_then(|value| value.str()) else {
                return;
            };
            *call.selected_camera.borrow_mut() = id.to_owned();
            let _ = call
                .sender
                .try_send(BackendCommand::Call(CallCommand::SetCameraDevice(
                    id.to_owned(),
                )));
            call.rebuild_context_menu();
        });
        actions.add_action(&camera);

        let call = self.clone();
        let edit_reaction =
            gio::SimpleAction::new("edit-reaction", Some(&i32::static_variant_type()));
        edit_reaction.connect_activate(move |_, value| {
            if call.selected.borrow().is_none() {
                return;
            }
            let Some(number) = value.and_then(|value| value.get::<i32>()) else {
                return;
            };
            if !(1..=9).contains(&number) {
                return;
            }
            let call = call.clone();
            let window = call.clone();
            call.reactions.edit(number as u8, move || {
                window.rebuild_context_menu();
            });
        });
        actions.add_action(&edit_reaction);

        let builtin = gio::SimpleAction::new("edit-builtin-reaction", None);
        builtin.set_enabled(false);
        actions.add_action(&builtin);

        let call = self.clone();
        let noise = gio::SimpleAction::new_stateful(
            "noise-suppression",
            None,
            &self.noise_suppression.get().to_variant(),
        );
        noise.connect_activate(move |action, _| {
            let enabled = !action.state().and_then(|state| state.get()).unwrap_or(true);
            action.set_state(&enabled.to_variant());
            call.noise_suppression.set(enabled);
            let _ = gio::Settings::new("ke.oa.miaow").set_boolean("noise-suppression", enabled);
            let _ = call
                .sender
                .try_send(BackendCommand::Call(CallCommand::SetNoiseSuppression(
                    enabled,
                )));
        });
        actions.add_action(&noise);

        let call = self.clone();
        let echo = gio::SimpleAction::new_stateful(
            "echo-cancellation",
            None,
            &self.echo_cancellation.get().to_variant(),
        );
        echo.connect_activate(move |action, _| {
            let enabled = !action.state().and_then(|state| state.get()).unwrap_or(true);
            action.set_state(&enabled.to_variant());
            call.echo_cancellation.set(enabled);
            let _ = gio::Settings::new("ke.oa.miaow").set_boolean("echo-cancellation", enabled);
            let _ = call
                .sender
                .try_send(BackendCommand::Call(CallCommand::SetEchoCancellation(
                    enabled,
                )));
        });
        actions.add_action(&echo);

        let call = self.clone();
        let local_pip = gio::SimpleAction::new_stateful(
            "show-local-pip",
            None,
            &self.local_pip_enabled.get().to_variant(),
        );
        local_pip.connect_activate(move |action, _| {
            let enabled = !action.state().and_then(|state| state.get()).unwrap_or(true);
            action.set_state(&enabled.to_variant());
            call.local_pip_enabled.set(enabled);
            call.refresh_video_layout();
        });
        actions.add_action(&local_pip);

        let call = self.clone();
        let debug = gio::SimpleAction::new_stateful(
            "debug-stats",
            None,
            &self.show_debug_stats.get().to_variant(),
        );
        debug.connect_activate(move |action, _| {
            let enabled = !action
                .state()
                .and_then(|state| state.get())
                .unwrap_or(false);
            action.set_state(&enabled.to_variant());
            call.show_debug_stats.set(enabled);
            call.debug_stats.set_visible(enabled);
            let _ = call
                .sender
                .try_send(BackendCommand::Call(CallCommand::SetDebugStats(enabled)));
            if let Err(error) =
                gio::Settings::new("ke.oa.miaow").set_boolean("show-debug-stats", enabled)
            {
                log::warn!("could not save debug stats preference: {error}");
            }
        });
        actions.add_action(&debug);
        self.window.insert_action_group("call", Some(&actions));

        let call = self.clone();
        let suppress_menu = Rc::new(Cell::new(false));
        let menu_click = gtk::GestureClick::new();
        menu_click.set_button(gdk::BUTTON_SECONDARY);
        let suppress_menu_pressed = suppress_menu.clone();
        let call_for_pressed = call.clone();
        menu_click.connect_pressed(move |_, _, x, y| {
            let suppressed = call_for_pressed
                .window
                .pick(x, y, gtk::PickFlags::DEFAULT)
                .is_some_and(|target| {
                    is_within(&target, &call_for_pressed.pip_layer.local)
                        || is_within(&target, &call_for_pressed.pip_layer.remote)
                });
            suppress_menu_pressed.set(suppressed);
        });
        menu_click.connect_released(move |gesture, _, x, y| {
            if suppress_menu.replace(false) {
                return;
            }
            call.rebuild_context_menu();
            call.context_menu.set_pointing_to(Some(&gdk::Rectangle::new(
                x.round() as i32,
                y.round() as i32,
                1,
                1,
            )));
            call.context_menu.popup();
            gesture.set_state(gtk::EventSequenceState::Claimed);
        });
        self.window.add_controller(menu_click);
    }
}
