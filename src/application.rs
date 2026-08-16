use std::{cell::RefCell, rc::Rc};

use adw::prelude::*;
use async_channel::{Receiver, Sender};
use gio::ApplicationFlags;
use uuid::Uuid;

use crate::{
    backend::{BackendCommand, BackendEvent, run_backend},
    room::{parse_join_link, pending_room},
    ui::CallWindow,
};

pub struct MiaowApplication {
    app: adw::Application,
    _runtime: tokio::runtime::Runtime,
    commands: Sender<BackendCommand>,
    events: Receiver<BackendEvent>,
    call_window: Rc<RefCell<Option<CallWindow>>>,
    pending_urls: Rc<RefCell<Vec<String>>>,
}

impl MiaowApplication {
    pub fn new() -> Self {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .thread_name("miaow-runtime")
            .build()
            .expect("create miaow runtime");
        let (commands, backend_commands) = async_channel::bounded(32);
        let (backend_events, events) = async_channel::bounded(16);
        runtime.spawn(run_backend(backend_commands, backend_events));

        let app = adw::Application::builder()
            .application_id("ke.oa.miaow")
            .flags(ApplicationFlags::HANDLES_OPEN)
            .build();
        let instance = Self {
            app,
            _runtime: runtime,
            commands,
            events,
            call_window: Rc::new(RefCell::new(None)),
            pending_urls: Rc::new(RefCell::new(Vec::new())),
        };
        instance.connect_signals();
        instance
    }

    pub fn run(&self) -> glib::ExitCode {
        let args: Vec<String> = std::env::args().collect();
        self.app.run_with_args(&args)
    }

    fn connect_signals(&self) {
        let window_cell = self.call_window.clone();
        let commands = self.commands.clone();
        let events = self.events.clone();
        let pending_urls = self.pending_urls.clone();
        self.app.connect_activate(move |app| {
            if let Some(call) = window_cell.borrow().as_ref() {
                call.window.present();
                return;
            }
            load_css();
            let call = CallWindow::new(app, commands.clone());
            call.set_join_pending(!pending_urls.borrow().is_empty());
            let closing = Rc::new(std::cell::Cell::new(false));
            let closing_for_request = closing.clone();
            let commands_for_close = commands.clone();
            call.window.connect_close_request(move |_| {
                if !closing_for_request.replace(true) {
                    let _ = commands_for_close.try_send(BackendCommand::Shutdown);
                }
                glib::Propagation::Stop
            });
            call.window.present();
            *window_cell.borrow_mut() = Some(call.clone());

            let call_for_events = call.clone();
            let events = events.clone();
            let app_for_events = app.clone();
            let pending_urls_for_events = pending_urls.clone();
            glib::spawn_future_local(async move {
                while let Ok(event) = events.recv().await {
                    if matches!(event, BackendEvent::ShutdownComplete) {
                        app_for_events.quit();
                        break;
                    }
                    let rooms_loaded = matches!(&event, BackendEvent::RoomsLoaded { .. });
                    call_for_events.handle_event(event);
                    if rooms_loaded {
                        for uri in pending_urls_for_events.borrow_mut().drain(..) {
                            handle_uri(&call_for_events, &uri);
                        }
                    }
                }
            });

            let selected = gio::Settings::new("ke.oa.miaow")
                .string("selected-room-id")
                .as_str()
                .parse::<Uuid>()
                .ok();
            let temporary_remote = requested_room_name();
            let _ = commands.try_send(BackendCommand::Load {
                selected,
                temporary_remote,
            });
        });

        let window_cell = self.call_window.clone();
        let pending_urls = self.pending_urls.clone();
        self.app.connect_open(move |app, files, _| {
            if window_cell.borrow().is_none() {
                app.activate();
            }
            for file in files {
                let uri = file.uri().to_string();
                if let Some(call) = window_cell.borrow().as_ref() {
                    if call.rooms_are_loaded() {
                        handle_uri(call, &uri);
                    } else {
                        call.set_join_pending(true);
                        pending_urls.borrow_mut().push(uri);
                    }
                    call.window.present();
                } else {
                    pending_urls.borrow_mut().push(uri);
                }
            }
        });

        let commands = self.commands.clone();
        self.app.connect_shutdown(move |_| {
            let _ = commands.try_send(BackendCommand::Shutdown);
        });
    }
}

impl Drop for MiaowApplication {
    fn drop(&mut self) {
        let _ = self.commands.try_send(BackendCommand::Shutdown);
    }
}

fn handle_uri(call: &CallWindow, uri: &str) {
    call.set_join_pending(false);
    match parse_join_link(uri).and_then(|link| pending_room(&link.server, &link.token)) {
        Ok(pending) => {
            call.submit_pending(pending);
        }
        Err(error) => {
            let dialog = adw::AlertDialog::builder()
                .heading("Couldn’t Add Room")
                .body(error.to_string())
                .build();
            dialog.add_response("ok", "OK");
            dialog.present(Some(&call.window));
        }
    }
}

fn requested_room_name() -> Option<String> {
    if let Some(value) = std::env::var("ROOM_NAME")
        .ok()
        .filter(|value| !value.trim().is_empty())
    {
        return Some(value.trim().to_owned());
    }
    let args: Vec<_> = std::env::args().collect();
    for (index, argument) in args.iter().enumerate() {
        if argument == "--room" {
            return args
                .get(index + 1)
                .map(|value| value.trim().to_owned())
                .filter(|value| !value.is_empty());
        }
        if let Some(value) = argument
            .strip_prefix("--room=")
            .filter(|value| !value.trim().is_empty())
        {
            return Some(value.trim().to_owned());
        }
    }
    None
}

fn load_css() {
    let provider = gtk::CssProvider::new();
    provider.load_from_string(include_str!("../data/style.css"));
    if let Some(display) = gtk::gdk::Display::default() {
        gtk::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
}
