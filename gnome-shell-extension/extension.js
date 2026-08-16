import GLib from 'gi://GLib';
import Clutter from 'gi://Clutter';

import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';

const TITLE_PREFIX = 'miaow:pinned:';
const MARGIN = 12;
const SNAP_DISTANCE = 96;

export default class MiaowPinnedWindowsExtension extends Extension {
    enable() {
        this._windows = new Map();
        this._signals = [];
        this._settings = this.getSettings();
        this._signals.push([
            global.display,
            global.display.connect('window-created', (_display, window) => this._watch(window)),
        ]);
        this._signals.push([
            global.display,
            global.display.connect('grab-op-end', (_display, window) => this._grabEnded(window)),
        ]);
        for (const actor of global.get_window_actors())
            this._watch(actor.metaWindow);
    }

    disable() {
        for (const [object, id] of this._signals)
            object.disconnect(id);
        this._signals = [];
        for (const [window, state] of this._windows) {
            this._disconnectWindow(window, state);
            if (state.corner)
                this._release(window, state, false);
        }
        this._windows.clear();
        this._windows = null;
        this._settings = null;
    }

    _watch(window) {
        // A Wayland window-created signal can arrive before GTK has published
        // either its application ID or its initial title. Watch title changes
        // on every window and keep the action itself scoped to TITLE_PREFIX.
        if (!window || this._windows.has(window))
            return;
        const state = {
            corner: null,
            normalRect: null,
            enforcing: false,
            titleId: 0,
            aboveId: 0,
            unmanagedId: 0,
            animationGeneration: 0,
        };
        state.titleId = window.connect('notify::title', () => this._sync(window, state));
        state.aboveId = window.connect('notify::above', () => {
            if (state.corner && !state.enforcing && !window.is_above())
                this._enforce(window, state);
        });
        state.unmanagedId = window.connect('unmanaged', () => {
            this._disconnectWindow(window, state);
            this._windows.delete(window);
        });
        this._windows.set(window, state);
        this._sync(window, state);
    }

    _disconnectWindow(window, state) {
        this._cancelAnimation(window, state);
        for (const id of [state.titleId, state.aboveId, state.unmanagedId]) {
            if (id)
                window.disconnect(id);
        }
    }

    _sync(window, state) {
        const title = window.get_title() ?? '';
        const corner = title.startsWith(TITLE_PREFIX)
            ? title.slice(TITLE_PREFIX.length)
            : null;
        if (['top-left', 'top-right', 'bottom-left', 'bottom-right'].includes(corner)) {
            const entering = !state.corner;
            if (entering)
                state.normalRect = window.get_frame_rect();
            state.corner = corner;
            this._enforce(window, state, entering);
        } else if (state.corner) {
            this._release(window, state, true);
        }
    }

    _enforce(window, state, resize = false) {
        if (!state.corner || state.enforcing)
            return;
        state.enforcing = true;
        window.make_above();
        window.stick();
        if (!resize) {
            state.enforcing = false;
            return;
        }

        // GTK publishes its lower pinned-mode minimum size in a separate
        // Wayland commit. Wait for that commit before asking Mutter to apply
        // the saved compact geometry.
        GLib.timeout_add(GLib.PRIORITY_DEFAULT, 100, () => {
            if (this._windows?.has(window) && state.corner)
                this._resizeAndSnap(window, state, state.corner);
            state.enforcing = false;
            return GLib.SOURCE_REMOVE;
        });
    }

    _release(window, state, restore) {
        state.corner = null;
        window.unmake_above();
        window.unstick();
        if (restore && state.normalRect) {
            const rect = state.normalRect;
            GLib.idle_add(GLib.PRIORITY_DEFAULT_IDLE, () => {
                if (this._windows?.has(window) && !state.corner)
                    this._animateFrame(window, state, rect);
                return GLib.SOURCE_REMOVE;
            });
        }
        state.normalRect = null;
    }

    _position(window, corner, width = null, height = null) {
        const area = window.get_work_area_current_monitor();
        const frame = window.get_frame_rect();
        width ??= frame.width;
        height ??= frame.height;
        const left = area.x + MARGIN;
        const right = area.x + area.width - width - MARGIN;
        const top = area.y + MARGIN;
        const bottom = area.y + area.height - height - MARGIN;
        if (corner === 'top-left')
            return [left, top];
        if (corner === 'top-right')
            return [right, top];
        if (corner === 'bottom-left')
            return [left, bottom];
        return [right, bottom];
    }

    _snap(window, state, corner) {
        const [x, y] = this._position(window, corner);
        const frame = window.get_frame_rect();
        this._animateFrame(window, state, {
            x,
            y,
            width: frame.width,
            height: frame.height,
        });
    }

    _resizeAndSnap(window, state, corner) {
        const width = Math.max(240, this._settings?.get_int('compact-width') ?? 360);
        const height = Math.max(150, this._settings?.get_int('compact-height') ?? 225);
        const [x, y] = this._position(window, corner, width, height);
        this._animateFrame(window, state, {x, y, width, height});
    }

    _animateFrame(window, state, target) {
        const start = window.get_frame_rect();
        if (start.x === target.x && start.y === target.y &&
            start.width === target.width && start.height === target.height)
            return;
        this._cancelAnimation(window, state);
        const generation = state.animationGeneration;
        const actor = window.get_compositor_private();
        if (!actor) {
            window.move_resize_frame(true, target.x, target.y, target.width, target.height);
            return;
        }

        actor.ease({
            translation_x: target.x - start.x,
            translation_y: target.y - start.y,
            scale_x: target.width / start.width,
            scale_y: target.height / start.height,
            duration: 190,
            mode: Clutter.AnimationMode.EASE_IN_OUT_QUAD,
            onComplete: () => {
                if (this._windows?.has(window) && state.animationGeneration === generation) {
                    window.move_resize_frame(
                        true,
                        target.x,
                        target.y,
                        target.width,
                        target.height,
                    );
                    GLib.idle_add(GLib.PRIORITY_HIGH_IDLE, () => {
                        if (this._windows?.has(window) && state.animationGeneration === generation) {
                            actor.set_translation(0, 0, 0);
                            actor.set_scale(1, 1);
                        }
                        return GLib.SOURCE_REMOVE;
                    });
                }
            },
        });
    }

    _cancelAnimation(window, state) {
        ++state.animationGeneration;
        const actor = window.get_compositor_private();
        if (!actor)
            return;
        for (const property of ['translation-x', 'translation-y', 'scale-x', 'scale-y'])
            actor.remove_transition(property);
        actor.set_pivot_point(0, 0);
        actor.set_translation(0, 0, 0);
        actor.set_scale(1, 1);
    }

    _grabEnded(window) {
        const state = this._windows?.get(window);
        if (!state?.corner)
            return;
        const frame = window.get_frame_rect();
        let nearest = null;
        let nearestDistance = Infinity;
        for (const corner of ['top-left', 'top-right', 'bottom-left', 'bottom-right']) {
            const [x, y] = this._position(window, corner);
            const distance = Math.hypot(frame.x - x, frame.y - y);
            if (distance < nearestDistance) {
                nearest = corner;
                nearestDistance = distance;
            }
        }
        if (nearestDistance <= SNAP_DISTANCE)
            state.corner = nearest;
        else
            return;

        this._snap(window, state, state.corner);
        this._settings?.set_int('compact-width', frame.width);
        this._settings?.set_int('compact-height', frame.height);
        this._enforce(window, state);
    }
}
