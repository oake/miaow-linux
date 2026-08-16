mod chrome;
mod interactions;
mod menus;
mod presentation;

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use adw::prelude::*;
use async_channel::Sender;
use gio::ActionEntry;
use gtk::{Align, Orientation, gdk, prelude::IsA};
use uuid::Uuid;

use crate::{
    backend::{BackendCommand, BackendEvent},
    call::{CallCommand, CallEvent, MediaDevice, RemoteVideoPreference, VideoKind},
    room::{PendingRoom, SavedRoom},
};

use super::pip::PipLayer;
use super::reactions::{Reactions, reaction_number};
use super::room_dialog::present_add_room;
use super::zoom::ZoomablePicture;

#[derive(Clone)]
pub struct CallWindow {
    app: adw::Application,
    pub window: adw::ApplicationWindow,
    sender: Sender<BackendCommand>,
    rooms: Rc<RefCell<Vec<SavedRoom>>>,
    selected: Rc<RefCell<Option<SavedRoom>>>,
    status_box: gtk::Box,
    status: gtk::Label,
    remote_badge: gtk::Box,
    remote_badge_label: gtk::Label,
    remote_mute_icon: gtk::Image,
    debug_stats: gtk::Label,
    show_debug_stats: Rc<Cell<bool>>,
    local_mute_badge: gtk::Box,
    local_mute_badge_allowed: Rc<Cell<bool>>,
    local_mute_name: gtk::Label,
    remote_name: Rc<RefCell<String>>,
    remote_muted: Rc<Cell<bool>>,
    remote_presence: Rc<Cell<RemotePresence>>,
    main_video: ZoomablePicture,
    remote_camera: gtk::Picture,
    remote_screen: gtk::Picture,
    local_camera: gtk::Picture,
    local_screen: gtk::Picture,
    pip_layer: PipLayer,
    remote_main_screen: Rc<Cell<bool>>,
    remote_pip_enabled: Rc<Cell<bool>>,
    last_remote_video_preference: Rc<Cell<Option<RemoteVideoPreference>>>,
    local_main_screen: Rc<Cell<bool>>,
    local_pip_enabled: Rc<Cell<bool>>,
    animate_stream_switch: Rc<Cell<Option<bool>>>,
    stream_controls: gtk::Box,
    switch_stream: gtk::Button,
    reopen_pip: gtk::Button,
    stream_controls_allowed: Rc<Cell<bool>>,
    chrome_visible: Rc<Cell<bool>>,
    spinner: gtk::Spinner,
    status_icon: gtk::Image,
    room_menu: gtk::MenuButton,
    context_menu: gtk::PopoverMenu,
    microphones: Rc<RefCell<Vec<MediaDevice>>>,
    speakers: Rc<RefCell<Vec<MediaDevice>>>,
    cameras: Rc<RefCell<Vec<MediaDevice>>>,
    selected_microphone: Rc<RefCell<String>>,
    selected_speaker: Rc<RefCell<String>>,
    selected_camera: Rc<RefCell<String>>,
    noise_suppression: Rc<Cell<bool>>,
    echo_cancellation: Rc<Cell<bool>>,
    controls: gtk::Box,
    microphone_button: gtk::ToggleButton,
    camera_button: gtk::ToggleButton,
    screen_button: gtk::ToggleButton,
    compact: Rc<Cell<bool>>,
    inhibit_cookie: Rc<Cell<Option<u32>>>,
    rooms_loaded: Rc<Cell<bool>>,
    join_pending: Rc<Cell<bool>>,
    reactions: Reactions,
}

#[derive(Clone, Copy, Default, Eq, PartialEq)]
enum RemotePresence {
    #[default]
    None,
    Waiting,
    Joining,
    Ready,
}

#[derive(Default)]
struct FullscreenClickState {
    last_press: Option<(u32, f64, f64)>,
    pending: Option<glib::SourceId>,
}

impl CallWindow {
    pub fn new(app: &adw::Application, sender: Sender<BackendCommand>) -> Self {
        let settings = gio::Settings::new("ke.oa.miaow");
        let window = adw::ApplicationWindow::builder()
            .application(app)
            .title("miaow")
            .default_width(settings.int("normal-width"))
            .default_height(settings.int("normal-height"))
            .width_request(720)
            .height_request(450)
            .build();
        window.add_css_class("call-window");

        let overlay = gtk::Overlay::new();
        let main_video = ZoomablePicture::new();
        main_video.add_css_class("call-background");
        overlay.set_child(Some(&main_video));

        // These unparented pictures retain the latest paintable for each
        // stream. Composition below chooses one for the main view and feeds
        // the others into strictly sized, independently movable PIPs.
        let remote_screen = gtk::Picture::new();
        let remote_camera = gtk::Picture::new();
        let local_screen = gtk::Picture::new();
        let local_camera = gtk::Picture::new();

        let pip_layer = PipLayer::new(&settings, &window);
        overlay.add_overlay(&pip_layer.widget);

        let stream_controls = gtk::Box::new(Orientation::Vertical, 0);
        stream_controls.add_css_class("stream-controls");
        stream_controls.set_halign(Align::End);
        stream_controls.set_valign(Align::Center);
        stream_controls.set_margin_end(12);
        let switch_stream = gtk::Button::builder()
            .icon_name("go-previous-symbolic")
            .tooltip_text("Show Other Video")
            .build();
        let reopen_pip = gtk::Button::builder()
            .icon_name("view-restore-symbolic")
            .tooltip_text("Show Picture in Picture")
            .build();
        stream_controls.append(&switch_stream);
        stream_controls.append(&reopen_pip);
        let stream_controls_revealer = animated_revealer(
            &stream_controls,
            gtk::RevealerTransitionType::Crossfade,
            150,
        );
        overlay.add_overlay(&stream_controls_revealer);

        let center = gtk::Box::new(Orientation::Vertical, 14);
        center.set_halign(Align::Center);
        center.set_valign(Align::Center);
        let spinner = gtk::Spinner::new();
        spinner.add_css_class("call-status-spinner");
        spinner.set_size_request(44, 44);
        spinner.set_spinning(true);
        let status_icon = gtk::Image::from_icon_name("avatar-default-symbolic");
        status_icon.set_pixel_size(48);
        status_icon.add_css_class("dim-label");
        status_icon.set_visible(false);
        let status = gtk::Label::new(Some("Loading saved rooms…"));
        status.add_css_class("call-status");
        center.append(&spinner);
        center.append(&status_icon);
        center.append(&status);
        overlay.add_overlay(&center);

        let remote_badge = gtk::Box::new(Orientation::Horizontal, 7);
        remote_badge.set_halign(Align::Center);
        remote_badge.set_valign(Align::Start);
        remote_badge.set_margin_top(22);
        remote_badge.add_css_class("remote-status");
        let remote_badge_label = gtk::Label::new(None);
        let remote_mute_icon = gtk::Image::from_icon_name("microphone-disabled-symbolic");
        remote_mute_icon.set_visible(false);
        remote_badge.append(&remote_badge_label);
        remote_badge.append(&remote_mute_icon);
        let remote_badge_revealer =
            animated_revealer(&remote_badge, gtk::RevealerTransitionType::Crossfade, 150);
        overlay.add_overlay(&remote_badge_revealer);

        let debug_stats = gtk::Label::new(Some("Waiting for media statistics…"));
        debug_stats.add_css_class("debug-stats");
        debug_stats.set_halign(Align::Start);
        debug_stats.set_valign(Align::Start);
        debug_stats.set_xalign(0.0);
        debug_stats.set_margin_top(20);
        debug_stats.set_margin_start(20);
        debug_stats.set_selectable(false);
        debug_stats.set_can_target(false);
        let show_debug_stats = Rc::new(Cell::new(settings.boolean("show-debug-stats")));
        debug_stats.set_visible(show_debug_stats.get());
        overlay.add_overlay(&debug_stats);

        let local_mute_badge = gtk::Box::new(Orientation::Horizontal, 7);
        local_mute_badge.add_css_class("local-mute-badge");
        local_mute_badge.set_halign(Align::End);
        local_mute_badge.set_valign(Align::End);
        local_mute_badge.set_margin_end(24);
        local_mute_badge.set_margin_bottom(24);
        local_mute_badge.set_can_target(false);
        let local_mute_name = gtk::Label::new(None);
        let local_mute_icon = gtk::Image::from_icon_name("microphone-disabled-symbolic");
        local_mute_badge.append(&local_mute_name);
        local_mute_badge.append(&local_mute_icon);
        let local_mute_revealer = animated_revealer(
            &local_mute_badge,
            gtk::RevealerTransitionType::Crossfade,
            150,
        );
        overlay.add_overlay(&local_mute_revealer);

        let header = adw::HeaderBar::new();
        header.add_css_class("call-header");
        let empty_title = gtk::Box::new(Orientation::Horizontal, 0);
        header.set_title_widget(Some(&empty_title));
        let room_menu = gtk::MenuButton::builder()
            .icon_name("system-users-symbolic")
            .tooltip_text("Rooms")
            .build();
        header.pack_start(&room_menu);
        header.set_halign(Align::Fill);
        header.set_valign(Align::Start);

        let controls = gtk::Box::new(Orientation::Horizontal, 12);
        controls.add_css_class("call-controls");
        controls.set_halign(Align::Center);
        controls.set_valign(Align::End);
        controls.set_margin_bottom(28);
        let microphone_button = control_button("audio-input-microphone-symbolic");
        let camera_button = control_button("camera-web-symbolic");
        let screen_button = control_button("video-display-symbolic");
        controls.append(&microphone_button);
        controls.append(&camera_button);
        controls.append(&screen_button);
        let controls_revealer =
            animated_revealer(&controls, gtk::RevealerTransitionType::SlideUp, 190);
        overlay.add_overlay(&controls_revealer);

        let close_button = gtk::Button::builder()
            .icon_name("window-close-symbolic")
            .tooltip_text("Close miaow")
            .build();
        close_button.add_css_class("circular");
        close_button.add_css_class("window-close-button");
        close_button.set_halign(Align::End);
        close_button.set_valign(Align::Start);
        close_button.set_margin_top(12);
        close_button.set_margin_end(12);
        close_button.set_visible(false);
        overlay.add_overlay(&close_button);

        let pin_top_left = pin_corner_button("↖", "Pin to Top Left", Align::Start, Align::Start);
        let pin_bottom_left =
            pin_corner_button("↙", "Pin to Bottom Left", Align::Start, Align::End);
        let pin_bottom_right =
            pin_corner_button("↘", "Pin to Bottom Right", Align::End, Align::End);
        overlay.add_overlay(&pin_top_left);
        overlay.add_overlay(&pin_bottom_left);
        overlay.add_overlay(&pin_bottom_right);

        let context_menu = gtk::PopoverMenu::from_model(None::<&gio::MenuModel>);
        context_menu.set_has_arrow(false);
        context_menu.set_parent(&overlay);

        window.set_content(Some(&overlay));

        let reactions = Reactions::new(&overlay, &window, sender.clone());

        let this = Self {
            app: app.clone(),
            window,
            sender,
            rooms: Rc::new(RefCell::new(Vec::new())),
            selected: Rc::new(RefCell::new(None)),
            status_box: center,
            status,
            remote_badge,
            remote_badge_label,
            remote_mute_icon,
            debug_stats,
            show_debug_stats,
            local_mute_badge,
            local_mute_badge_allowed: Rc::new(Cell::new(false)),
            local_mute_name,
            remote_name: Rc::new(RefCell::new(String::new())),
            remote_muted: Rc::new(Cell::new(false)),
            remote_presence: Rc::new(Cell::new(RemotePresence::None)),
            main_video,
            remote_camera,
            remote_screen,
            local_camera,
            local_screen,
            pip_layer,
            remote_main_screen: Rc::new(Cell::new(false)),
            remote_pip_enabled: Rc::new(Cell::new(true)),
            last_remote_video_preference: Rc::new(Cell::new(None)),
            local_main_screen: Rc::new(Cell::new(false)),
            local_pip_enabled: Rc::new(Cell::new(true)),
            animate_stream_switch: Rc::new(Cell::new(None)),
            stream_controls,
            switch_stream: switch_stream.clone(),
            reopen_pip: reopen_pip.clone(),
            stream_controls_allowed: Rc::new(Cell::new(false)),
            chrome_visible: Rc::new(Cell::new(false)),
            spinner,
            status_icon,
            room_menu,
            context_menu,
            microphones: Rc::new(RefCell::new(Vec::new())),
            speakers: Rc::new(RefCell::new(Vec::new())),
            cameras: Rc::new(RefCell::new(Vec::new())),
            selected_microphone: Rc::new(RefCell::new(String::new())),
            selected_speaker: Rc::new(RefCell::new(String::new())),
            selected_camera: Rc::new(RefCell::new(String::new())),
            noise_suppression: Rc::new(Cell::new(settings.boolean("noise-suppression"))),
            echo_cancellation: Rc::new(Cell::new(settings.boolean("echo-cancellation"))),
            controls,
            microphone_button,
            camera_button,
            screen_button,
            compact: Rc::new(Cell::new(false)),
            inhibit_cookie: Rc::new(Cell::new(None)),
            rooms_loaded: Rc::new(Cell::new(false)),
            join_pending: Rc::new(Cell::new(false)),
            reactions,
        };
        this.main_video.install_zoom_controls(&this.window);
        this.install_actions(app);
        this.install_interactions(
            &header,
            &close_button,
            &pin_top_left,
            &pin_bottom_left,
            &pin_bottom_right,
        );
        this.install_pip_interactions();
        this.install_context_menu();
        {
            let call = this.clone();
            switch_stream.connect_clicked(move |_| {
                let show_screen = !call.remote_main_screen.get();
                call.remote_main_screen.set(show_screen);
                call.animate_stream_switch.set(Some(show_screen));
                call.main_video.reset_zoom();
                call.refresh_video_layout();
            });
        }
        {
            let call = this.clone();
            this.pip_layer
                .local
                .switch_button()
                .connect_clicked(move |_| {
                    call.local_main_screen.set(!call.local_main_screen.get());
                    call.main_video.reset_zoom();
                    call.refresh_video_layout();
                });
        }
        {
            let call = this.clone();
            reopen_pip.connect_clicked(move |_| {
                call.remote_pip_enabled.set(true);
                call.refresh_video_layout();
            });
        }
        this.rebuild_room_menu();
        this.rebuild_context_menu();
        let _ = this
            .sender
            .try_send(BackendCommand::Call(CallCommand::SetDebugStats(
                this.show_debug_stats.get(),
            )));
        let _ = this
            .sender
            .try_send(BackendCommand::Call(CallCommand::SetNoiseSuppression(
                this.noise_suppression.get(),
            )));
        let _ = this
            .sender
            .try_send(BackendCommand::Call(CallCommand::SetEchoCancellation(
                this.echo_cancellation.get(),
            )));
        this
    }
}

fn is_within(widget: &gtk::Widget, ancestor: &impl IsA<gtk::Widget>) -> bool {
    widget == ancestor.as_ref() || widget.is_ancestor(ancestor)
}

fn is_editable_focused(window: &impl IsA<gtk::Window>) -> bool {
    window.focus().is_some_and(|focus| {
        focus.is::<gtk::Text>()
            || focus.is::<gtk::Entry>()
            || focus.is::<gtk::SearchEntry>()
            || focus.ancestor(gtk::Text::static_type()).is_some()
            || focus.ancestor(gtk::Entry::static_type()).is_some()
    })
}

fn is_interactive_widget(widget: &gtk::Widget) -> bool {
    let is_type_or_descendant =
        |widget_type| widget.type_() == widget_type || widget.ancestor(widget_type).is_some();
    is_type_or_descendant(gtk::Button::static_type())
        || is_type_or_descendant(gtk::ToggleButton::static_type())
        || is_type_or_descendant(gtk::MenuButton::static_type())
}

fn animated_revealer(
    child: &impl IsA<gtk::Widget>,
    transition: gtk::RevealerTransitionType,
    duration_ms: u32,
) -> gtk::Revealer {
    let revealer = gtk::Revealer::new();
    revealer.set_halign(child.as_ref().halign());
    revealer.set_valign(child.as_ref().valign());
    revealer.set_transition_type(transition);
    revealer.set_transition_duration(duration_ms);
    revealer.set_reveal_child(false);
    revealer.set_child(Some(child));
    revealer
}

fn set_animated_visible(widget: &impl IsA<gtk::Widget>, visible: bool) {
    if let Some(revealer) = widget
        .as_ref()
        .parent()
        .and_then(|parent| parent.downcast::<gtk::Revealer>().ok())
    {
        revealer.set_reveal_child(visible);
    } else {
        widget.as_ref().set_visible(visible);
    }
}

fn control_button(icon: &str) -> gtk::ToggleButton {
    let button = gtk::ToggleButton::builder().icon_name(icon).build();
    button.add_css_class("circular");
    button.add_css_class("call-control");
    button
}

fn pin_corner_button(label: &str, tooltip: &str, halign: Align, valign: Align) -> gtk::Button {
    let button = gtk::Button::builder()
        .label(label)
        .tooltip_text(tooltip)
        .halign(halign)
        .valign(valign)
        .build();
    button.add_css_class("circular");
    button.add_css_class("pin-corner-button");
    button.set_margin_top(12);
    button.set_margin_bottom(12);
    button.set_margin_start(12);
    button.set_margin_end(12);
    button.set_visible(false);
    button
}

fn update_pin_buttons(
    top_left: &gtk::Button,
    bottom_left: &gtk::Button,
    bottom_right: &gtk::Button,
    window: &adw::ApplicationWindow,
    allowed: bool,
    x: f64,
    y: f64,
) {
    let allowed = allowed && !window.is_fullscreen();
    let near_left = x <= 96.0;
    let near_right = x >= f64::from(window.width() - 96);
    let near_top = y <= 96.0;
    let near_bottom = y >= f64::from(window.height() - 96);
    top_left.set_visible(allowed && near_left && near_top);
    bottom_left.set_visible(allowed && near_left && near_bottom);
    bottom_right.set_visible(allowed && near_right && near_bottom);
}

fn present_confirmation(
    window: &adw::ApplicationWindow,
    sender: Sender<BackendCommand>,
    pending: PendingRoom,
) {
    let dialog = adw::AlertDialog::builder()
        .heading(pending.confirmation_message())
        .build();
    dialog.add_response("cancel", "Cancel");
    dialog.add_response("add", "Add Room");
    dialog.set_default_response(Some("add"));
    dialog.set_close_response("cancel");
    dialog.set_response_appearance("add", adw::ResponseAppearance::Suggested);
    dialog.connect_response(None, move |_, response| {
        if response == "add" {
            let _ = sender.try_send(BackendCommand::Add(pending.clone()));
        }
    });
    dialog.present(Some(window));
}
