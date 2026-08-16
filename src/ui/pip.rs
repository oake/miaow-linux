use std::{
    cell::{Cell, OnceCell},
    rc::Rc,
    time::{Duration, Instant},
};

use adw::prelude::*;
use gtk::gdk;
use gtk::subclass::prelude::*;

mod sized_picture {
    use super::*;

    pub struct Imp {
        pub(super) picture: OnceCell<gtk::Picture>,
        pub(super) switch_button: OnceCell<gtk::Button>,
        pub(super) mute_badge: OnceCell<gtk::Image>,
        pub(super) switch_available: Cell<bool>,
        pub(super) pointer_inside: Cell<bool>,
        pub(super) switch_visible: Cell<bool>,
        pub(super) mute_visible: Cell<bool>,
        pub(super) width: Cell<i32>,
        pub(super) height: Cell<i32>,
    }

    impl Default for Imp {
        fn default() -> Self {
            Self {
                picture: OnceCell::new(),
                switch_button: OnceCell::new(),
                mute_badge: OnceCell::new(),
                switch_available: Cell::new(false),
                pointer_inside: Cell::new(false),
                switch_visible: Cell::new(false),
                mute_visible: Cell::new(false),
                width: Cell::new(240),
                height: Cell::new(135),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Imp {
        const NAME: &'static str = "MiaowSizedPicture";
        type Type = super::SizedPicture;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for Imp {
        fn constructed(&self) {
            self.parent_constructed();
            let picture = gtk::Picture::new();
            picture.set_can_shrink(true);
            picture.set_content_fit(gtk::ContentFit::Cover);
            picture.set_parent(&*self.obj());
            self.picture.set(picture).expect("picture initialized once");
            let switch_button = gtk::Button::builder()
                .icon_name("go-previous-symbolic")
                .tooltip_text("Switch Camera and Screen")
                .build();
            switch_button.add_css_class("circular");
            switch_button.add_css_class("pip-switch");
            switch_button.set_visible(false);
            switch_button.set_opacity(0.0);
            switch_button.set_parent(&*self.obj());
            self.switch_button
                .set(switch_button)
                .expect("switch button initialized once");

            let mute_badge = gtk::Image::from_icon_name("microphone-disabled-symbolic");
            mute_badge.add_css_class("pip-mute-badge");
            mute_badge.set_tooltip_text(Some("Microphone Muted"));
            mute_badge.set_visible(false);
            mute_badge.set_opacity(0.0);
            mute_badge.set_parent(&*self.obj());
            self.mute_badge
                .set(mute_badge)
                .expect("mute badge initialized once");

            let motion = gtk::EventControllerMotion::new();
            let weak = self.obj().downgrade();
            motion.connect_enter(move |_, _, _| {
                if let Some(widget) = weak.upgrade() {
                    let imp = widget.imp();
                    imp.pointer_inside.set(true);
                    widget.update_switch_visibility();
                }
            });
            let weak = self.obj().downgrade();
            motion.connect_leave(move |_| {
                if let Some(widget) = weak.upgrade() {
                    let imp = widget.imp();
                    imp.pointer_inside.set(false);
                    widget.update_switch_visibility();
                }
            });
            self.obj().add_controller(motion);
        }

        fn dispose(&self) {
            if let Some(picture) = self.picture.get() {
                picture.unparent();
            }
            if let Some(button) = self.switch_button.get() {
                button.unparent();
            }
            if let Some(badge) = self.mute_badge.get() {
                badge.unparent();
            }
        }
    }

    impl WidgetImpl for Imp {
        fn measure(&self, orientation: gtk::Orientation, _for_size: i32) -> (i32, i32, i32, i32) {
            let size = if orientation == gtk::Orientation::Horizontal {
                self.width.get()
            } else {
                self.height.get()
            };
            (size, size, -1, -1)
        }

        fn size_allocate(&self, width: i32, height: i32, baseline: i32) {
            if let Some(picture) = self.picture.get() {
                picture.allocate(width, height, baseline, None);
            }
            if let Some(button) = self.switch_button.get() {
                let transform = gtk::gsk::Transform::new().translate(&gtk::graphene::Point::new(
                    (width - 44) as f32,
                    ((height - 36) / 2) as f32,
                ));
                button.allocate(36, 36, baseline, Some(transform));
            }
            if let Some(badge) = self.mute_badge.get() {
                let transform = gtk::gsk::Transform::new().translate(&gtk::graphene::Point::new(
                    (width - 36) as f32,
                    (height - 30) as f32,
                ));
                badge.allocate(28, 22, baseline, Some(transform));
            }
        }

        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            if let Some(picture) = self.picture.get() {
                self.obj().snapshot_child(picture, snapshot);
            }
            if let Some(button) = self
                .switch_button
                .get()
                .filter(|button| button.is_visible())
            {
                self.obj().snapshot_child(button, snapshot);
            }
            if let Some(badge) = self.mute_badge.get().filter(|badge| badge.is_visible()) {
                self.obj().snapshot_child(badge, snapshot);
            }
        }
    }
}

glib::wrapper! {
    pub struct SizedPicture(ObjectSubclass<sized_picture::Imp>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl SizedPicture {
    fn new(css_class: &str) -> Self {
        let widget: Self = glib::Object::new();
        widget.add_css_class(css_class);
        widget.set_overflow(gtk::Overflow::Hidden);
        widget.set_cursor_from_name(Some("grab"));
        widget
    }

    pub fn set_paintable(&self, paintable: Option<&impl IsA<gdk::Paintable>>) {
        self.imp()
            .picture
            .get()
            .expect("constructed")
            .set_paintable(paintable);
    }

    pub fn set_content_fit(&self, fit: gtk::ContentFit) {
        self.imp()
            .picture
            .get()
            .expect("constructed")
            .set_content_fit(fit);
    }

    pub fn set_mirrored(&self, mirrored: bool) {
        let picture = self.imp().picture.get().expect("constructed");
        if mirrored {
            picture.add_css_class("mirrored-video");
        } else {
            picture.remove_css_class("mirrored-video");
        }
    }

    fn set_pip_size(&self, width: i32, height: i32) {
        self.imp().width.set(width);
        self.imp().height.set(height);
        self.queue_resize();
    }

    pub fn switch_button(&self) -> gtk::Button {
        self.imp().switch_button.get().expect("constructed").clone()
    }

    pub fn set_switch_visible(&self, visible: bool) {
        self.imp().switch_available.set(visible);
        self.update_switch_visibility();
    }

    pub fn set_switch_direction(&self, show_previous: bool) {
        self.imp()
            .switch_button
            .get()
            .expect("constructed")
            .set_icon_name(if show_previous {
                "go-previous-symbolic"
            } else {
                "go-next-symbolic"
            });
    }

    pub fn set_mute_visible(&self, visible: bool) {
        let imp = self.imp();
        if imp.mute_visible.replace(visible) != visible {
            fade_widget(imp.mute_badge.get().expect("constructed"), visible);
        }
    }

    fn update_switch_visibility(&self) {
        let imp = self.imp();
        let visible = imp.switch_available.get() && imp.pointer_inside.get();
        if imp.switch_visible.replace(visible) != visible {
            fade_widget(imp.switch_button.get().expect("constructed"), visible);
        }
    }

    fn stream_aspect(&self) -> f64 {
        self.imp()
            .picture
            .get()
            .and_then(gtk::Picture::paintable)
            .map(|paintable| paintable.intrinsic_aspect_ratio())
            .filter(|aspect| *aspect > 0.05)
            .unwrap_or(16.0 / 9.0)
    }
}

fn fade_widget(widget: &impl IsA<gtk::Widget>, visible: bool) {
    let widget = widget.as_ref();
    let from = widget.opacity();
    if visible {
        widget.set_visible(true);
    }
    let target = if visible { 1.0 } else { 0.0 };
    let started = Rc::new(Cell::new(None::<i64>));
    widget.add_tick_callback(move |widget, clock| {
        let now = clock.frame_time();
        let start = started.get().unwrap_or_else(|| {
            started.set(Some(now));
            now
        });
        let progress = ((now - start).max(0) as f64 / 150_000.0).clamp(0.0, 1.0);
        let eased = 1.0 - (1.0 - progress).powi(3);
        widget.set_opacity(from + (target - from) * eased);
        if progress >= 1.0 {
            if !visible {
                widget.set_visible(false);
            }
            glib::ControlFlow::Break
        } else {
            glib::ControlFlow::Continue
        }
    });
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Corner {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

impl Corner {
    fn parse(value: &str, fallback: Self) -> Self {
        match value {
            "top-left" => Self::TopLeft,
            "top-right" => Self::TopRight,
            "bottom-left" => Self::BottomLeft,
            "bottom-right" => Self::BottomRight,
            _ => fallback,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::TopLeft => "top-left",
            Self::TopRight => "top-right",
            Self::BottomLeft => "bottom-left",
            Self::BottomRight => "bottom-right",
        }
    }

    fn target(self, bounds: (i32, i32), size: (i32, i32)) -> (f64, f64) {
        const MARGIN: i32 = 24;
        let x = if matches!(self, Self::TopLeft | Self::BottomLeft) {
            MARGIN
        } else {
            (bounds.0 - size.0 - MARGIN).max(MARGIN)
        };
        let y = if matches!(self, Self::TopLeft | Self::TopRight) {
            MARGIN
        } else {
            (bounds.1 - size.1 - MARGIN).max(MARGIN)
        };
        (f64::from(x), f64::from(y))
    }

    fn nearest(center: (f64, f64), bounds: (i32, i32), size: (i32, i32)) -> Self {
        let candidates = [
            Self::TopLeft,
            Self::TopRight,
            Self::BottomLeft,
            Self::BottomRight,
        ];
        candidates
            .into_iter()
            .min_by(|first, second| {
                let distance = |corner: Self| {
                    let (x, y) = corner.target(bounds, size);
                    let dx = x + f64::from(size.0) / 2.0 - center.0;
                    let dy = y + f64::from(size.1) / 2.0 - center.1;
                    dx * dx + dy * dy
                };
                distance(*first).total_cmp(&distance(*second))
            })
            .unwrap_or(Self::BottomRight)
    }
}

#[derive(Clone)]
struct Slot {
    widget: SizedPicture,
    corner: Rc<Cell<Corner>>,
    width: Rc<Cell<i32>>,
    height: Rc<Cell<i32>>,
    x: Rc<Cell<f64>>,
    y: Rc<Cell<f64>>,
    dragging: Rc<Cell<bool>>,
}

impl Slot {
    fn size(&self) -> (i32, i32) {
        (self.width.get(), self.height.get())
    }
}

#[derive(Clone)]
pub struct PipLayer {
    pub widget: gtk::Fixed,
    pub local: SizedPicture,
    pub remote: SizedPicture,
    local_slot: Slot,
    remote_slot: Slot,
    settings: gio::Settings,
    last_bounds: Rc<Cell<(i32, i32)>>,
}

impl PipLayer {
    pub fn new(settings: &gio::Settings, window: &adw::ApplicationWindow) -> Self {
        let fixed = gtk::Fixed::new();
        fixed.set_hexpand(true);
        fixed.set_vexpand(true);
        fixed.set_halign(gtk::Align::Fill);
        fixed.set_valign(gtk::Align::Fill);

        let local = SizedPicture::new("local-pip");
        let remote = SizedPicture::new("remote-pip");
        local.set_visible(false);
        remote.set_visible(false);
        fixed.put(&local, 0.0, 0.0);
        fixed.put(&remote, 0.0, 0.0);

        let slot = |widget: &SizedPicture, prefix: &str, fallback: Corner| {
            let width = settings.int(&format!("{prefix}-pip-width")).clamp(160, 420);
            let height = settings.int(&format!("{prefix}-pip-height")).clamp(90, 320);
            widget.set_pip_size(width, height);
            Slot {
                widget: widget.clone(),
                corner: Rc::new(Cell::new(Corner::parse(
                    settings.string(&format!("{prefix}-pip-corner")).as_str(),
                    fallback,
                ))),
                width: Rc::new(Cell::new(width)),
                height: Rc::new(Cell::new(height)),
                x: Rc::new(Cell::new(0.0)),
                y: Rc::new(Cell::new(0.0)),
                dragging: Rc::new(Cell::new(false)),
            }
        };
        let local_slot = slot(&local, "local", Corner::BottomRight);
        let remote_slot = slot(&remote, "remote", Corner::BottomLeft);
        if local_slot.corner.get() == remote_slot.corner.get() {
            remote_slot.corner.set(Corner::BottomLeft);
        }

        let layer = Self {
            widget: fixed,
            local,
            remote,
            local_slot,
            remote_slot,
            settings: settings.clone(),
            last_bounds: Rc::new(Cell::new((0, 0))),
        };
        layer.install_drag(
            layer.local_slot.clone(),
            layer.remote_slot.clone(),
            "local",
            window,
        );
        layer.install_drag(
            layer.remote_slot.clone(),
            layer.local_slot.clone(),
            "remote",
            window,
        );
        layer.install_size_watcher();
        layer
    }

    pub fn show_local(
        &self,
        paintable: Option<&impl IsA<gdk::Paintable>>,
        fit: gtk::ContentFit,
        mirrored: bool,
    ) {
        self.local.set_paintable(paintable);
        self.local.set_content_fit(fit);
        self.local.set_mirrored(mirrored);
        self.local.set_visible(paintable.is_some());
        self.resolve_overlap(true);
        self.layout(false);
    }

    pub fn show_remote(&self, paintable: Option<&impl IsA<gdk::Paintable>>, fit: gtk::ContentFit) {
        self.remote.set_paintable(paintable);
        self.remote.set_content_fit(fit);
        self.remote.set_visible(paintable.is_some());
        self.resolve_overlap(false);
        self.layout(false);
    }

    fn resolve_overlap(&self, keep_local: bool) {
        if self.local.is_visible()
            && self.remote.is_visible()
            && self.local_slot.corner.get() == self.remote_slot.corner.get()
        {
            if keep_local {
                self.remote_slot
                    .corner
                    .set(opposite(self.local_slot.corner.get()));
            } else {
                self.local_slot
                    .corner
                    .set(opposite(self.remote_slot.corner.get()));
            }
        }
    }

    fn install_size_watcher(&self) {
        let layer = self.clone();
        self.widget.add_tick_callback(move |widget, _| {
            let bounds = (widget.width(), widget.height());
            if bounds != layer.last_bounds.replace(bounds) {
                layer.layout(false);
            }
            glib::ControlFlow::Continue
        });
    }

    fn install_drag(
        &self,
        moving: Slot,
        occupied: Slot,
        key: &'static str,
        window: &adw::ApplicationWindow,
    ) {
        let fixed = self.widget.clone();
        let settings = self.settings.clone();
        let start = Rc::new(Cell::new((0.0, 0.0)));
        let pointer_start = Rc::new(Cell::new((0.0, 0.0)));
        let start_size = Rc::new(Cell::new((0, 0)));
        let resizing = Rc::new(Cell::new(false));
        let lock_aspect = Rc::new(Cell::new(false));
        let gesture = gtk::GestureDrag::new();

        let moving_motion = moving.clone();
        let motion = gtk::EventControllerMotion::new();
        motion.connect_motion(move |_, x, y| {
            if !moving_motion.dragging.get() {
                moving_motion.widget.set_cursor_from_name(Some(
                    if is_resize_handle(moving_motion.corner.get(), moving_motion.size(), x, y) {
                        resize_cursor(moving_motion.corner.get())
                    } else {
                        "grab"
                    },
                ));
            }
        });
        let moving_leave = moving.clone();
        motion.connect_leave(move |_| {
            if !moving_leave.dragging.get() {
                moving_leave.widget.set_cursor_from_name(Some("grab"));
            }
        });
        moving.widget.add_controller(motion);

        let resizing_key = resizing.clone();
        let lock_aspect_key = lock_aspect.clone();
        let key_controller = gtk::EventControllerKey::new();
        key_controller.set_propagation_phase(gtk::PropagationPhase::Capture);
        key_controller.connect_key_pressed(move |_, key, _, _| {
            if resizing_key.get() && matches!(key, gdk::Key::Shift_L | gdk::Key::Shift_R) {
                lock_aspect_key.set(true);
            }
            glib::Propagation::Proceed
        });
        window.add_controller(key_controller);

        let moving_begin = moving.clone();
        let occupied_begin = occupied.clone();
        let fixed_begin = fixed.clone();
        let start_begin = start.clone();
        let pointer_start_begin = pointer_start.clone();
        let start_size_begin = start_size.clone();
        let resizing_begin = resizing.clone();
        let lock_aspect_begin = lock_aspect.clone();
        gesture.connect_drag_begin(move |gesture, x, y| {
            moving_begin
                .widget
                .insert_after(&fixed_begin, Some(&occupied_begin.widget));
            moving_begin.dragging.set(true);
            start_begin.set((moving_begin.x.get(), moving_begin.y.get()));
            pointer_start_begin.set(
                gesture
                    .current_event()
                    .and_then(|event| event.position())
                    .unwrap_or((x, y)),
            );
            start_size_begin.set(moving_begin.size());
            let near_resize_corner =
                is_resize_handle(moving_begin.corner.get(), moving_begin.size(), x, y);
            resizing_begin.set(near_resize_corner);
            lock_aspect_begin.set(
                near_resize_corner
                    && gesture
                        .current_event_state()
                        .contains(gdk::ModifierType::SHIFT_MASK),
            );
            if near_resize_corner {
                moving_begin
                    .widget
                    .set_cursor_from_name(Some(resize_cursor(moving_begin.corner.get())));
                gesture.set_state(gtk::EventSequenceState::Claimed);
            } else {
                moving_begin.widget.set_cursor_from_name(Some("grabbing"));
            }
        });

        let fixed_update = fixed.clone();
        let moving_update = moving.clone();
        let start_update = start.clone();
        let pointer_start_update = pointer_start.clone();
        let start_size_update = start_size.clone();
        let resizing_update = resizing.clone();
        let lock_aspect_update = lock_aspect.clone();
        gesture.connect_drag_update(move |gesture, fallback_dx, fallback_dy| {
            let (dx, dy) = gesture
                .current_event()
                .and_then(|event| event.position())
                .map(|position| {
                    let start = pointer_start_update.get();
                    (position.0 - start.0, position.1 - start.1)
                })
                .unwrap_or((fallback_dx, fallback_dy));
            if resizing_update.get() {
                if gesture
                    .current_event_state()
                    .contains(gdk::ModifierType::SHIFT_MASK)
                {
                    lock_aspect_update.set(true);
                }
                let direction_x = if matches!(
                    moving_update.corner.get(),
                    Corner::TopLeft | Corner::BottomLeft
                ) {
                    1.0
                } else {
                    -1.0
                };
                let direction_y = if matches!(
                    moving_update.corner.get(),
                    Corner::TopLeft | Corner::TopRight
                ) {
                    1.0
                } else {
                    -1.0
                };
                let initial = start_size_update.get();
                let width = (f64::from(initial.0) + dx * direction_x).round() as i32;
                let height = (f64::from(initial.1) + dy * direction_y).round() as i32;
                let size = if lock_aspect_update.get() {
                    aspect_locked_size(
                        (width, height),
                        moving_update.widget.stream_aspect(),
                        fixed_update.height(),
                    )
                } else {
                    (
                        width.clamp(160, 420),
                        height.clamp(90, (fixed_update.height() - 48).max(90)),
                    )
                };
                moving_update.width.set(size.0);
                moving_update.height.set(size.1);
                moving_update
                    .widget
                    .set_pip_size(moving_update.width.get(), moving_update.height.get());
                let (x, y) = moving_update.corner.get().target(
                    (fixed_update.width(), fixed_update.height()),
                    moving_update.size(),
                );
                moving_update.x.set(x);
                moving_update.y.set(y);
                fixed_update.move_(&moving_update.widget, x, y);
            } else {
                let origin = start_update.get();
                let x = origin.0 + dx;
                let y = origin.1 + dy;
                moving_update.x.set(x);
                moving_update.y.set(y);
                fixed_update.move_(&moving_update.widget, x, y);
            }
        });

        let layer = self.clone();
        let moving_end = moving.clone();
        let occupied_end = occupied.clone();
        let resizing_end = resizing.clone();
        let lock_aspect_end = lock_aspect.clone();
        gesture.connect_drag_end(move |_, _, _| {
            moving_end.dragging.set(false);
            if resizing_end.get() {
                moving_end
                    .widget
                    .set_cursor_from_name(Some(resize_cursor(moving_end.corner.get())));
                let _ = settings.set_int(&format!("{key}-pip-width"), moving_end.width.get());
                let _ = settings.set_int(&format!("{key}-pip-height"), moving_end.height.get());
            } else {
                moving_end.widget.set_cursor_from_name(Some("grab"));
                let old = moving_end.corner.get();
                let center = (
                    moving_end.x.get() + f64::from(moving_end.width.get()) / 2.0,
                    moving_end.y.get() + f64::from(moving_end.height.get()) / 2.0,
                );
                let nearest = Corner::nearest(
                    center,
                    (layer.widget.width(), layer.widget.height()),
                    moving_end.size(),
                );
                if occupied_end.widget.is_visible() && occupied_end.corner.get() == nearest {
                    occupied_end.corner.set(if old == nearest {
                        opposite(nearest)
                    } else {
                        old
                    });
                }
                moving_end.corner.set(nearest);
                let _ = settings.set_string(&format!("{key}-pip-corner"), nearest.as_str());
            }
            lock_aspect_end.set(false);
            layer.layout(true);
        });
        moving.widget.add_controller(gesture);
    }

    fn layout(&self, animated: bool) {
        for slot in [&self.local_slot, &self.remote_slot] {
            if slot.dragging.get() {
                continue;
            }
            let target = slot
                .corner
                .get()
                .target((self.widget.width(), self.widget.height()), slot.size());
            if animated {
                animate(&self.widget, slot.clone(), target);
            } else {
                slot.x.set(target.0);
                slot.y.set(target.1);
                self.widget.move_(&slot.widget, target.0, target.1);
            }
        }
    }
}

fn is_resize_handle(corner: Corner, size: (i32, i32), x: f64, y: f64) -> bool {
    const HANDLE: f64 = 24.0;
    match corner {
        Corner::TopLeft => x > f64::from(size.0) - HANDLE && y > f64::from(size.1) - HANDLE,
        Corner::TopRight => x < HANDLE && y > f64::from(size.1) - HANDLE,
        Corner::BottomLeft => x > f64::from(size.0) - HANDLE && y < HANDLE,
        Corner::BottomRight => x < HANDLE && y < HANDLE,
    }
}

fn resize_cursor(corner: Corner) -> &'static str {
    match corner {
        Corner::TopLeft | Corner::BottomRight => "nwse-resize",
        Corner::TopRight | Corner::BottomLeft => "nesw-resize",
    }
}

fn aspect_locked_size(proposed: (i32, i32), aspect: f64, available_height: i32) -> (i32, i32) {
    let height_per_width = 1.0 / aspect;
    let projected_width = (f64::from(proposed.0) + f64::from(proposed.1) * height_per_width)
        / (1.0 + height_per_width * height_per_width);
    let minimum_width = 160.0_f64.max(90.0 * aspect);
    let maximum_width = 420.0_f64.min(f64::from((available_height - 48).max(90)) * aspect);
    let width = projected_width
        .clamp(minimum_width, maximum_width.max(minimum_width))
        .round() as i32;
    (width, (f64::from(width) / aspect).round() as i32)
}

fn opposite(corner: Corner) -> Corner {
    match corner {
        Corner::TopLeft => Corner::BottomRight,
        Corner::TopRight => Corner::BottomLeft,
        Corner::BottomLeft => Corner::TopRight,
        Corner::BottomRight => Corner::TopLeft,
    }
}

fn animate(fixed: &gtk::Fixed, slot: Slot, target: (f64, f64)) {
    let fixed = fixed.clone();
    let start = (slot.x.get(), slot.y.get());
    let began = Instant::now();
    glib::timeout_add_local(Duration::from_millis(16), move || {
        let progress = (began.elapsed().as_secs_f64() / 0.24).min(1.0);
        let eased = 1.0 - (1.0 - progress).powi(3);
        let x = start.0 + (target.0 - start.0) * eased;
        let y = start.1 + (target.1 - start.1) * eased;
        slot.x.set(x);
        slot.y.set(y);
        fixed.move_(&slot.widget, x, y);
        if progress >= 1.0 {
            glib::ControlFlow::Break
        } else {
            glib::ControlFlow::Continue
        }
    });
}
