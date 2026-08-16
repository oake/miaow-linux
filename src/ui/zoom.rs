use std::{
    cell::{Cell, OnceCell},
    rc::Rc,
};

use adw::prelude::*;
use gtk::{gdk, subclass::prelude::*};

const MIN_GESTURE_SCALE: f64 = 0.65;
const MAX_SCALE: f64 = 6.0;

mod imp {
    use super::*;

    pub struct ZoomablePicture {
        pub(super) picture: OnceCell<gtk::Picture>,
        pub(super) previous: OnceCell<gtk::Picture>,
        pub(super) scale: Cell<f64>,
        pub(super) offset_x: Cell<f64>,
        pub(super) offset_y: Cell<f64>,
        pub(super) pinch_scale: Cell<f64>,
        pub(super) pinch_offset_x: Cell<f64>,
        pub(super) pinch_offset_y: Cell<f64>,
        pub(super) pinch_focus_x: Cell<f64>,
        pub(super) pinch_focus_y: Cell<f64>,
        pub(super) transition: Cell<f64>,
        pub(super) transition_direction: Cell<f64>,
        pub(super) animation: Cell<u64>,
        pub(super) mirrored: Cell<bool>,
    }

    impl Default for ZoomablePicture {
        fn default() -> Self {
            Self {
                picture: OnceCell::new(),
                previous: OnceCell::new(),
                scale: Cell::new(1.0),
                offset_x: Cell::new(0.0),
                offset_y: Cell::new(0.0),
                pinch_scale: Cell::new(1.0),
                pinch_offset_x: Cell::new(0.0),
                pinch_offset_y: Cell::new(0.0),
                pinch_focus_x: Cell::new(0.0),
                pinch_focus_y: Cell::new(0.0),
                transition: Cell::new(1.0),
                transition_direction: Cell::new(1.0),
                animation: Cell::new(0),
                mirrored: Cell::new(false),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ZoomablePicture {
        const NAME: &'static str = "MiaowZoomablePicture";
        type Type = super::ZoomablePicture;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for ZoomablePicture {
        fn constructed(&self) {
            self.parent_constructed();
            let picture = gtk::Picture::new();
            picture.set_can_shrink(true);
            picture.set_content_fit(gtk::ContentFit::Cover);
            picture.set_parent(&*self.obj());
            self.picture.set(picture).expect("picture initialized once");
            let previous = gtk::Picture::new();
            previous.set_can_shrink(true);
            previous.set_content_fit(gtk::ContentFit::Cover);
            previous.set_parent(&*self.obj());
            self.previous
                .set(previous)
                .expect("previous picture initialized once");
        }

        fn dispose(&self) {
            if let Some(picture) = self.picture.get() {
                picture.unparent();
            }
            if let Some(previous) = self.previous.get() {
                previous.unparent();
            }
        }
    }

    impl WidgetImpl for ZoomablePicture {
        fn measure(&self, _orientation: gtk::Orientation, _for_size: i32) -> (i32, i32, i32, i32) {
            (0, 0, -1, -1)
        }

        fn size_allocate(&self, width: i32, height: i32, baseline: i32) {
            let Some(picture) = self.picture.get() else {
                return;
            };
            self.clamp_offsets(width, height);
            let scale = self.scale.get();
            let child_width = (f64::from(width) * scale).round() as i32;
            let child_height = (f64::from(height) * scale).round() as i32;
            let x = (f64::from(width - child_width) / 2.0 + self.offset_x.get()) as f32;
            let y = (f64::from(height - child_height) / 2.0 + self.offset_y.get()) as f32;
            let progress = self.transition.get();
            let direction = self.transition_direction.get();
            if progress < 1.0
                && let Some(previous) = self.previous.get()
            {
                let old_x = x - (direction * progress * f64::from(width)) as f32;
                let transform =
                    gtk::gsk::Transform::new().translate(&gtk::graphene::Point::new(old_x, y));
                previous.allocate(child_width, child_height, baseline, Some(transform));
            }
            let x = x + (direction * (1.0 - progress) * f64::from(width)) as f32;
            let transform = gtk::gsk::Transform::new().translate(&gtk::graphene::Point::new(x, y));
            picture.allocate(child_width, child_height, baseline, Some(transform));
        }

        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            if self.transition.get() < 1.0
                && let Some(previous) = self.previous.get()
            {
                self.obj().snapshot_child(previous, snapshot);
            }
            if let Some(picture) = self.picture.get() {
                self.obj().snapshot_child(picture, snapshot);
            }
        }
    }

    impl ZoomablePicture {
        pub(super) fn clamp_offsets(&self, width: i32, height: i32) {
            let excess = (self.scale.get() - 1.0).max(0.0);
            let max_x = f64::from(width) * excess / 2.0;
            let max_y = f64::from(height) * excess / 2.0;
            self.offset_x.set(self.offset_x.get().clamp(-max_x, max_x));
            self.offset_y.set(self.offset_y.get().clamp(-max_y, max_y));
        }
    }
}

glib::wrapper! {
    pub struct ZoomablePicture(ObjectSubclass<imp::ZoomablePicture>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl ZoomablePicture {
    pub fn new() -> Self {
        let widget: Self = glib::Object::new();
        widget.set_overflow(gtk::Overflow::Hidden);
        widget
    }

    // The controllers deliberately live on the window, not this widget. The
    // full-size PIP layer is a sibling above the video and is normally the GDK
    // event target; window capture sees the gesture before that overlay does.
    pub fn install_zoom_controls(&self, window: &adw::ApplicationWindow) {
        let picture = self.clone();
        let pinch = gtk::EventControllerLegacy::new();
        pinch.set_propagation_phase(gtk::PropagationPhase::Capture);
        pinch.connect_event(move |_, event| {
            if event.event_type() != gdk::EventType::TouchpadPinch || !picture.is_visible() {
                return glib::Propagation::Proceed;
            }
            let Some(event) = event.downcast_ref::<gdk::TouchpadEvent>() else {
                return glib::Propagation::Proceed;
            };
            match event.gesture_phase() {
                gdk::TouchpadGesturePhase::Begin => {
                    let focus = event.upcast_ref().position().unwrap_or((
                        f64::from(picture.width()) / 2.0,
                        f64::from(picture.height()) / 2.0,
                    ));
                    picture.begin_pinch_at(focus);
                }
                gdk::TouchpadGesturePhase::Update => {
                    picture.update_pinch(event.pinch_scale());
                }
                gdk::TouchpadGesturePhase::End | gdk::TouchpadGesturePhase::Cancel => {
                    picture.finish_pinch();
                }
                _ => {}
            }
            glib::Propagation::Stop
        });
        window.add_controller(pinch);

        // GtkGestureZoom supplies real touchscreen sequences. Native
        // touchpad pinches stay on the direct GDK path above.
        let picture = self.clone();
        let touch_active = Rc::new(Cell::new(false));
        let touch_zoom = gtk::GestureZoom::new();
        touch_zoom.set_propagation_phase(gtk::PropagationPhase::Capture);
        let picture_begin = picture.clone();
        let active_begin = touch_active.clone();
        touch_zoom.connect_begin(move |gesture, _| {
            if gesture
                .current_event()
                .is_some_and(|event| event.event_type() == gdk::EventType::TouchpadPinch)
            {
                return;
            }
            let focus = gesture.bounding_box_center().unwrap_or((
                f64::from(picture_begin.width()) / 2.0,
                f64::from(picture_begin.height()) / 2.0,
            ));
            picture_begin.begin_pinch_at(focus);
            active_begin.set(true);
        });
        let picture_update = picture.clone();
        let active_update = touch_active.clone();
        touch_zoom.connect_update(move |gesture, _| {
            if active_update.get() {
                picture_update
                    .update_pinch_at(gesture.scale_delta(), gesture.bounding_box_center());
            }
        });
        let picture_end = picture.clone();
        let active_end = touch_active.clone();
        touch_zoom.connect_end(move |_, _| {
            if active_end.replace(false) {
                picture_end.finish_pinch();
            }
        });
        let active_cancel = touch_active.clone();
        touch_zoom.connect_cancel(move |_, _| {
            if active_cancel.replace(false) {
                picture.finish_pinch();
            }
        });
        window.add_controller(touch_zoom);

        let picture = self.clone();
        let pan = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::BOTH_AXES);
        pan.set_propagation_phase(gtk::PropagationPhase::Capture);
        pan.connect_scroll(move |controller, dx, dy| {
            let imp = picture.imp();
            if !picture.is_visible() {
                return glib::Propagation::Proceed;
            }
            if controller
                .current_event_state()
                .contains(gdk::ModifierType::CONTROL_MASK)
            {
                let focus = controller
                    .current_event()
                    .and_then(|event| event.position())
                    .unwrap_or((
                        f64::from(picture.width()) / 2.0,
                        f64::from(picture.height()) / 2.0,
                    ));
                picture.zoom_at((-dy * 0.10).exp(), focus);
                return glib::Propagation::Stop;
            }
            if imp.scale.get() <= 1.0 {
                return glib::Propagation::Proceed;
            }
            // GDK deltas describe viewport scrolling, so invert them to move
            // the video itself with the user's fingers.
            picture.set_transform(
                imp.scale.get(),
                imp.offset_x.get() - dx,
                imp.offset_y.get() - dy,
            );
            glib::Propagation::Stop
        });
        window.add_controller(pan);

        let picture = self.clone();
        let start_x = Rc::new(Cell::new(0.0));
        let start_y = Rc::new(Cell::new(0.0));
        let active = Rc::new(Cell::new(false));
        let drag = gtk::GestureDrag::new();
        drag.set_button(gdk::BUTTON_PRIMARY);
        drag.set_propagation_phase(gtk::PropagationPhase::Capture);
        let picture_begin = picture.clone();
        let start_x_begin = start_x.clone();
        let start_y_begin = start_y.clone();
        let active_begin = active.clone();
        drag.connect_drag_begin(move |gesture, _, _| {
            let enabled = picture_begin.is_visible()
                && picture_begin.imp().scale.get() > 1.0
                && gesture
                    .current_event_state()
                    .contains(gdk::ModifierType::CONTROL_MASK);
            active_begin.set(enabled);
            if enabled {
                start_x_begin.set(picture_begin.imp().offset_x.get());
                start_y_begin.set(picture_begin.imp().offset_y.get());
                gesture.set_state(gtk::EventSequenceState::Claimed);
            }
        });
        drag.connect_drag_update(move |gesture, dx, dy| {
            if active.get() {
                picture.set_transform(
                    picture.imp().scale.get(),
                    start_x.get() + dx,
                    start_y.get() + dy,
                );
                gesture.set_state(gtk::EventSequenceState::Claimed);
            }
        });
        window.add_controller(drag);
    }

    pub fn set_paintable(&self, paintable: Option<&impl IsA<gdk::Paintable>>) {
        self.imp()
            .picture
            .get()
            .expect("constructed")
            .set_paintable(paintable);
    }

    pub fn set_paintable_animated(
        &self,
        paintable: Option<&impl IsA<gdk::Paintable>>,
        mirrored: bool,
        forward: bool,
    ) {
        let imp = self.imp();
        let picture = imp.picture.get().expect("constructed");
        let previous = imp.previous.get().expect("constructed");
        previous.set_paintable(picture.paintable().as_ref());
        set_picture_mirrored(previous, imp.mirrored.get());
        picture.set_paintable(paintable);
        set_picture_mirrored(picture, mirrored);
        imp.mirrored.set(mirrored);
        imp.transition_direction
            .set(if forward { 1.0 } else { -1.0 });
        imp.transition.set(0.0);
        let generation = imp.animation.get().wrapping_add(1);
        imp.animation.set(generation);
        let started = Rc::new(Cell::new(None::<i64>));
        let started_for_tick = started.clone();
        self.add_tick_callback(move |widget, clock| {
            if widget.imp().animation.get() != generation {
                return glib::ControlFlow::Break;
            }
            let now = clock.frame_time();
            let start = started_for_tick.get().unwrap_or_else(|| {
                started_for_tick.set(Some(now));
                now
            });
            let elapsed = (now - start).max(0) as f64 / 1000.0;
            let linear = (elapsed / 220.0).clamp(0.0, 1.0);
            let progress = 1.0 - (1.0 - linear).powi(3);
            widget.imp().transition.set(progress);
            widget.queue_allocate();
            if linear >= 1.0 {
                widget
                    .imp()
                    .previous
                    .get()
                    .expect("constructed")
                    .set_paintable(gdk::Paintable::NONE);
                glib::ControlFlow::Break
            } else {
                glib::ControlFlow::Continue
            }
        });
    }

    pub fn set_mirrored(&self, mirrored: bool) {
        let picture = self.imp().picture.get().expect("constructed");
        set_picture_mirrored(picture, mirrored);
        self.imp().mirrored.set(mirrored);
    }

    pub fn reset_zoom(&self) {
        self.set_transform(1.0, 0.0, 0.0);
    }

    fn begin_pinch_at(&self, focus: (f64, f64)) {
        let imp = self.imp();
        imp.pinch_scale.set(imp.scale.get());
        imp.pinch_offset_x.set(imp.offset_x.get());
        imp.pinch_offset_y.set(imp.offset_y.get());
        imp.pinch_focus_x.set(focus.0);
        imp.pinch_focus_y.set(focus.1);
    }

    fn update_pinch(&self, gesture_delta: f64) {
        self.update_pinch_at(gesture_delta, None);
    }

    fn update_pinch_at(&self, gesture_delta: f64, current_focus: Option<(f64, f64)>) {
        let imp = self.imp();
        let old_scale = imp.pinch_scale.get();
        let new_scale = (old_scale * gesture_delta).clamp(MIN_GESTURE_SCALE, MAX_SCALE);
        let ratio = new_scale / old_scale;
        let center_x = f64::from(self.width()) / 2.0;
        let center_y = f64::from(self.height()) / 2.0;
        let focus_x = current_focus.map_or(imp.pinch_focus_x.get(), |focus| focus.0);
        let focus_y = current_focus.map_or(imp.pinch_focus_y.get(), |focus| focus.1);
        self.set_transform(
            new_scale,
            focus_x
                - center_x
                - ratio * (imp.pinch_focus_x.get() - center_x - imp.pinch_offset_x.get()),
            focus_y
                - center_y
                - ratio * (imp.pinch_focus_y.get() - center_y - imp.pinch_offset_y.get()),
        );
    }

    fn zoom_at(&self, factor: f64, focus: (f64, f64)) {
        let imp = self.imp();
        let old_scale = imp.scale.get();
        let new_scale = (old_scale * factor).clamp(1.0, MAX_SCALE);
        let ratio = new_scale / old_scale;
        let center_x = f64::from(self.width()) / 2.0;
        let center_y = f64::from(self.height()) / 2.0;
        self.set_transform(
            new_scale,
            (focus.0 - center_x) * (1.0 - ratio) + imp.offset_x.get() * ratio,
            (focus.1 - center_y) * (1.0 - ratio) + imp.offset_y.get() * ratio,
        );
    }

    fn finish_pinch(&self) {
        if self.imp().scale.get() < 1.0 {
            self.reset_zoom();
        } else {
            let imp = self.imp();
            self.set_transform(imp.scale.get(), imp.offset_x.get(), imp.offset_y.get());
        }
    }

    fn set_transform(&self, scale: f64, offset_x: f64, offset_y: f64) {
        let imp = self.imp();
        imp.scale.set(scale);
        imp.offset_x.set(offset_x);
        imp.offset_y.set(offset_y);
        imp.clamp_offsets(self.width(), self.height());
        self.queue_allocate();
    }
}

fn set_picture_mirrored(picture: &gtk::Picture, mirrored: bool) {
    if mirrored {
        picture.add_css_class("mirrored-video");
    } else {
        picture.remove_css_class("mirrored-video");
    }
}

impl Default for ZoomablePicture {
    fn default() -> Self {
        Self::new()
    }
}
