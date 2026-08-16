use super::*;

/// Owns the controls that reveal together and their inactivity timeout.
#[derive(Clone)]
pub(super) struct CallChrome {
    pub call: CallWindow,
    pub header: adw::HeaderBar,
    pub close: gtk::Button,
    pub pins: [gtk::Button; 3],
    pub timer: Rc<RefCell<Option<glib::SourceId>>>,
}

impl CallChrome {
    pub fn pointer_moved(&self, x: f64, y: f64) {
        self.close.set_visible(
            !self.call.compact.get() && x >= f64::from(self.call.window.width() - 96) && y <= 96.0,
        );
        update_pin_buttons(
            &self.pins[0],
            &self.pins[1],
            &self.pins[2],
            &self.call.window,
            self.call.selected.borrow().is_some() && !self.call.compact.get(),
            x,
            y,
        );
        self.show(self.is_over_control(x, y));
    }

    pub fn is_over_control(&self, x: f64, y: f64) -> bool {
        self.call
            .window
            .pick(x, y, gtk::PickFlags::DEFAULT)
            .is_some_and(|target| is_interactive_widget(&target))
    }

    fn cancel_timeout(&self) {
        if let Some(timer) = self.timer.borrow_mut().take() {
            timer.remove();
        }
    }

    fn hide_controls(&self) {
        self.call.chrome_visible.set(false);
        set_animated_visible(&self.call.controls, false);
        self.header.set_visible(false);
        set_animated_visible(&self.call.stream_controls, false);
        self.close.set_visible(false);
        for pin in &self.pins {
            pin.set_visible(false);
        }
        set_animated_visible(
            &self.call.local_mute_badge,
            self.call.local_mute_badge_allowed.get(),
        );
    }

    fn show(&self, keep_visible: bool) {
        self.call.window.set_cursor(None);
        self.call.chrome_visible.set(true);
        set_animated_visible(&self.call.local_mute_badge, false);
        set_animated_visible(&self.call.controls, true);
        self.header.set_visible(!self.call.compact.get());
        set_animated_visible(
            &self.call.stream_controls,
            self.call.stream_controls_allowed.get(),
        );
        self.cancel_timeout();
        if keep_visible {
            return;
        }
        let chrome = self.clone();
        let timer =
            glib::timeout_add_local_once(std::time::Duration::from_millis(2500), move || {
                chrome.timer.borrow_mut().take();
                chrome.hide_controls();
                if let Some(cursor) = gdk::Cursor::from_name("none", None) {
                    chrome.call.window.set_cursor(Some(&cursor));
                }
            });
        *self.timer.borrow_mut() = Some(timer);
    }

    pub fn pointer_left(&self) {
        self.cancel_timeout();
        self.hide_controls();
        self.call.window.set_cursor(None);
    }
}
