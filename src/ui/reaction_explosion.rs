use std::time::Instant;

use gtk::pango;
use gtk::{gdk, graphene, prelude::*, subclass::prelude::*};

const PARTICLE_COUNT: usize = 24;
const EMOJI_SIZE: i32 = 42;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum LaunchEdge {
    Top,
    #[default]
    Bottom,
}

impl LaunchEdge {
    pub fn opposite(self) -> Self {
        match self {
            Self::Top => Self::Bottom,
            Self::Bottom => Self::Top,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Side {
    Left,
    Right,
}

#[derive(Clone, Copy, Debug)]
struct ReactionParticle {
    side: Side,
    angle: f64,
    initial_speed: f64,
    final_horizontal_speed: f64,
    drag: f64,
    initial_rotation: f64,
}

impl ReactionParticle {
    fn new(side: Side) -> Self {
        let degrees = random_in(15.0, 82.0);
        Self {
            side,
            angle: degrees * std::f64::consts::PI / 180.0
                * if side == Side::Left { 1.0 } else { -1.0 },
            initial_speed: random_in(0.9, 1.7),
            final_horizontal_speed: random_in(0.2, 0.6),
            drag: random_in(0.0005, 0.0009),
            initial_rotation: random_in(0.0, std::f64::consts::PI * 2.0),
        }
    }
}

/// Native GTK port of the SwiftUI/js-confetti particle motion in
/// `../miaow-macos/Sources/miaow/ReactionConfetti.swift`.
fn particle_state(
    particle: &ReactionParticle,
    elapsed_milliseconds: f64,
    width: f64,
    height: f64,
    launch_edge: LaunchEdge,
) -> (f64, f64, f64) {
    let width_coefficient = width.max(2.0).ln() / 1_920f64.ln();
    let initial_speed = particle.initial_speed * width_coefficient;
    // The original side-fired cone is too shallow when launched from the
    // bottom of a landscape call window. Tighten only that cone so the
    // particles climb through the canvas before gravity takes over.
    let flight_angle = if launch_edge == LaunchEdge::Bottom {
        particle.angle * 0.55
    } else {
        particle.angle
    };
    let sin_angle = flight_angle.sin();
    let cos_angle = flight_angle.cos();
    let time = elapsed_milliseconds;
    let initial_x = if particle.side == Side::Left {
        0.0
    } else {
        width
    };

    let deceleration_time =
        ((initial_speed - particle.final_horizontal_speed) / particle.drag).max(0.0);
    let slowing_time = time.min(deceleration_time);
    let horizontal_distance = initial_speed * slowing_time
        - particle.drag * slowing_time * slowing_time / 2.0
        + particle.final_horizontal_speed * (time - deceleration_time).max(0.0);

    let x = initial_x + horizontal_distance * sin_angle;
    let gravity_travel = 0.00125 * time * time / 2.0;
    let y = if launch_edge == LaunchEdge::Bottom {
        height - initial_speed * cos_angle * time + gravity_travel
    } else {
        initial_speed * cos_angle * time + gravity_travel
    };

    // Emoji particles in js-confetti begin at 0.01 radians/ms and gradually
    // slow. This time-based equivalent stays smooth at variable refresh rates.
    let angular_velocity = (0.01 - 0.000002 * time).max(0.0);
    let rotation = particle.initial_rotation + angular_velocity * time;
    (x, y, rotation)
}

fn fade_opacity(elapsed_seconds: f64) -> f64 {
    let fade_start = 1.0;
    if elapsed_seconds <= fade_start {
        return 1.0;
    }
    let fade_end = 4.2;
    let remaining = (1.0 - (elapsed_seconds - fade_start) / (fade_end - fade_start)).max(0.0);
    remaining * remaining
}

fn random_in(min: f64, max: f64) -> f64 {
    min + (max - min) * glib::random_double()
}

mod imp {
    use super::*;
    use std::cell::{Cell, RefCell};

    #[derive(Default)]
    pub struct ReactionExplosion {
        pub emoji: RefCell<String>,
        pub launch_edge: Cell<LaunchEdge>,
        pub started_at: Cell<Option<Instant>>,
        pub(super) particles: RefCell<Vec<ReactionParticle>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ReactionExplosion {
        const NAME: &'static str = "MiaowReactionExplosion";
        type Type = super::ReactionExplosion;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for ReactionExplosion {
        fn constructed(&self) {
            self.parent_constructed();
            self.obj().set_can_target(false);
            self.obj().set_focusable(false);
            self.obj().set_overflow(gtk::Overflow::Hidden);
            self.obj().set_hexpand(true);
            self.obj().set_vexpand(true);
        }
    }

    impl WidgetImpl for ReactionExplosion {
        fn measure(&self, _orientation: gtk::Orientation, _for_size: i32) -> (i32, i32, i32, i32) {
            (0, 0, -1, -1)
        }

        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            let obj = self.obj();
            let started = self.started_at.get().unwrap_or_else(Instant::now);
            let elapsed_seconds = started.elapsed().as_secs_f64().clamp(0.0, 5.0);
            let elapsed = elapsed_seconds * 1_000.0;
            let width = f64::from(obj.width());
            let height = f64::from(obj.height());
            let emoji = self.emoji.borrow().clone();
            let launch_edge = self.launch_edge.get();
            let opacity = fade_opacity(elapsed_seconds);
            if opacity <= 0.0 || emoji.is_empty() {
                return;
            }

            let layout = obj.create_pango_layout(Some(&emoji));
            let mut font = obj.pango_context().font_description().unwrap_or_default();
            font.set_size(EMOJI_SIZE * pango::SCALE);
            layout.set_font_description(Some(&font));
            let (text_width, text_height) = layout.pixel_size();
            let origin_x = -(text_width as f32) / 2.0;
            let origin_y = -(text_height as f32) / 2.0;

            snapshot.push_opacity(opacity);
            for particle in self.particles.borrow().iter() {
                let (x, y, rotation) =
                    particle_state(particle, elapsed, width, height, launch_edge);
                snapshot.save();
                snapshot.translate(&graphene::Point::new(x as f32, y as f32));
                snapshot.rotate(rotation.to_degrees() as f32);
                snapshot.translate(&graphene::Point::new(origin_x, origin_y));
                snapshot.append_layout(&layout, &gdk::RGBA::new(1.0, 1.0, 1.0, 1.0));
                snapshot.restore();
            }
            snapshot.pop();
        }
    }
}

glib::wrapper! {
    pub struct ReactionExplosion(ObjectSubclass<imp::ReactionExplosion>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl ReactionExplosion {
    pub fn new(emoji: &str, launch_edge: LaunchEdge) -> Self {
        let widget: Self = glib::Object::new();
        widget.set_accessible_role(gtk::AccessibleRole::Presentation);
        let particles = (0..PARTICLE_COUNT)
            .map(|index| {
                ReactionParticle::new(if index % 2 == 0 {
                    Side::Left
                } else {
                    Side::Right
                })
            })
            .collect();
        let imp = widget.imp();
        imp.emoji.replace(emoji.to_owned());
        imp.launch_edge.set(launch_edge);
        imp.started_at.set(Some(Instant::now()));
        imp.particles.replace(particles);
        widget.add_tick_callback(|widget, _| {
            widget.queue_draw();
            if widget
                .imp()
                .started_at
                .get()
                .is_some_and(|started| started.elapsed().as_secs_f64() >= 5.0)
            {
                widget.unparent();
                glib::ControlFlow::Break
            } else {
                glib::ControlFlow::Continue
            }
        });
        widget
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fade_matches_swift() {
        assert_eq!(fade_opacity(0.0), 1.0);
        assert_eq!(fade_opacity(1.0), 1.0);
        let mid = fade_opacity(2.6);
        assert!((mid - 0.25).abs() < 1e-9);
        assert_eq!(fade_opacity(4.2), 0.0);
        assert_eq!(fade_opacity(5.0), 0.0);
    }

    #[test]
    fn bottom_launch_starts_at_the_bottom_edge() {
        let particle = ReactionParticle {
            side: Side::Left,
            angle: 0.0,
            initial_speed: 1.0,
            final_horizontal_speed: 0.2,
            drag: 0.0007,
            initial_rotation: 0.0,
        };
        let (_, y, _) = particle_state(&particle, 0.0, 1920.0, 1080.0, LaunchEdge::Bottom);
        assert_eq!(y, 1080.0);
        let (_, top_y, _) = particle_state(&particle, 0.0, 1920.0, 1080.0, LaunchEdge::Top);
        assert_eq!(top_y, 0.0);
    }
}
