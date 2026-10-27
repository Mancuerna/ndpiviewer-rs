use super::*;
use std::time::Duration;

const LIMIT: usize = 100;
const SETTLE: Duration = Duration::from_millis(500);

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct View {
    center: egui::Vec2,
    zoom: f32,
    plane: u32,
    angle: f32,
    rotation: u8,
    mirrored: [bool; 2],
}

/// Small, session-only snapshots; no pixels or textures are retained.
#[derive(Default)]
pub(super) struct History {
    entries: Vec<View>,
    cursor: usize,
    pending: Option<(View, Instant)>,
}

impl History {
    fn observe(&mut self, view: View, now: Instant, interacting: bool) {
        if self.entries.is_empty() {
            self.entries.push(view);
            return;
        }
        if self.entries[self.cursor] == view {
            self.pending = None;
            return;
        }
        match self.pending {
            Some((previous, since)) if previous == view && !interacting => {
                if now.duration_since(since) >= SETTLE {
                    self.commit(view);
                }
            }
            _ => self.pending = Some((view, now)),
        }
    }

    fn commit(&mut self, view: View) {
        self.pending = None;
        if self.entries.get(self.cursor) == Some(&view) {
            return;
        }
        self.entries.truncate(self.cursor + 1);
        self.entries.push(view);
        if self.entries.len() > LIMIT {
            self.entries.remove(0);
        }
        self.cursor = self.entries.len() - 1;
    }

    fn can_back(&self) -> bool {
        self.cursor > 0 || self.pending.is_some()
    }

    fn can_forward(&self) -> bool {
        self.pending.is_none() && self.cursor + 1 < self.entries.len()
    }

    fn navigate(&mut self, current: View, forward: bool) -> Option<View> {
        // Capture even an unsettled movement so Back never loses the live view.
        if self.entries.get(self.cursor) != Some(&current) {
            self.commit(current);
        }
        self.pending = None;
        if forward {
            if !self.can_forward() {
                return None;
            }
            self.cursor += 1;
        } else {
            if self.cursor == 0 {
                return None;
            }
            self.cursor -= 1;
        }
        Some(self.entries[self.cursor])
    }
}

impl Viewer {
    fn navigation_view(&self) -> View {
        View {
            center: self.center,
            zoom: self.zoom,
            plane: self.plane,
            angle: self.features.angle,
            rotation: self.features.rotation,
            mirrored: self.features.mirrored,
        }
    }

    pub(super) fn navigate_history(&mut self, forward: bool) {
        if self.slide.is_none() || self.benchmark.is_some() {
            return;
        }
        let current = self.navigation_view();
        if let Some(view) = self.features.history.navigate(current, forward) {
            self.features.cancel_gesture();
            self.center = view.center;
            self.zoom = view.zoom;
            self.plane = view.plane;
            self.features.rotation = view.rotation;
            self.features.angle = view.angle;
            self.features.mirrored = view.mirrored;
            self.fit = false;
            self.changed();
        }
    }

    pub(super) fn navigation_buttons(&mut self, ui: &mut egui::Ui) {
        let enabled = self.slide.is_some() && self.benchmark.is_none();
        if ui
            .add_enabled(
                enabled && self.features.history.can_back(),
                egui::Button::new("Back"),
            )
            .on_hover_text("Previous view (Alt+Left): position, zoom, Z and rotation")
            .clicked()
        {
            self.navigate_history(false);
        }
        if ui
            .add_enabled(
                enabled && self.features.history.can_forward(),
                egui::Button::new("Forward"),
            )
            .on_hover_text("Next view (Alt+Right)")
            .clicked()
        {
            self.navigate_history(true);
        }
    }

    pub(super) fn observe_navigation(&mut self, ctx: &egui::Context) {
        if self.slide.is_none() || self.benchmark.is_some() {
            return;
        }
        let view = self.navigation_view();
        let interacting = ctx.input(|i| {
            i.pointer.any_down()
                || i.smooth_scroll_delta != egui::Vec2::ZERO
                || [
                    egui::Key::ArrowLeft,
                    egui::Key::ArrowRight,
                    egui::Key::ArrowUp,
                    egui::Key::ArrowDown,
                    egui::Key::PageUp,
                    egui::Key::PageDown,
                ]
                .iter()
                .any(|key| i.key_down(*key))
        });
        self.features
            .history
            .observe(view, Instant::now(), interacting);
        if self.features.history.pending.is_some() {
            ctx.request_repaint_after(SETTLE);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view(x: f32) -> View {
        View {
            center: egui::vec2(x, 10.0),
            zoom: 1.0,
            plane: 0,
            angle: 0.0,
            rotation: 0,
            mirrored: [false; 2],
        }
    }

    #[test]
    fn continuous_gesture_becomes_one_step_and_restores_all_coordinates() {
        let mut h = History::default();
        let now = Instant::now();
        h.observe(view(0.0), now, false);
        for x in 1..100 {
            h.observe(view(x as f32), now + SETTLE, true);
        }
        let destination = View {
            zoom: 2.0,
            plane: 4,
            angle: 0.0,
            rotation: 3,
            mirrored: [true, false],
            ..view(100.0)
        };
        h.observe(destination, now + SETTLE, false);
        h.observe(destination, now + SETTLE * 2, false);
        assert_eq!(h.entries.len(), 2);
        assert_eq!(h.navigate(destination, false), Some(view(0.0)));
        assert_eq!(h.navigate(view(0.0), true), Some(destination));
        assert!(!h.can_forward());
    }

    #[test]
    fn back_captures_unsettled_view_and_new_navigation_discards_forward_branch() {
        let mut h = History::default();
        h.commit(view(0.0));
        h.commit(view(1.0));
        assert_eq!(h.navigate(view(2.0), false), Some(view(1.0)));
        assert!(h.can_forward());
        h.observe(view(3.0), Instant::now(), false);
        assert!(!h.can_forward());
        assert_eq!(h.navigate(view(3.0), false), Some(view(1.0)));
        assert_eq!(h.navigate(view(1.0), true), Some(view(3.0)));
        assert!(!h.entries.contains(&view(2.0)));
    }

    #[test]
    fn history_is_bounded_and_returning_to_current_view_cancels_pending_step() {
        let mut h = History::default();
        for x in 0..200 {
            h.commit(view(x as f32));
        }
        assert_eq!(h.entries.len(), LIMIT);
        assert_eq!(h.entries[0], view(100.0));
        let now = Instant::now();
        h.observe(view(201.0), now, false);
        h.observe(view(199.0), now + SETTLE, false);
        assert!(h.pending.is_none());
        for _ in 1..LIMIT {
            assert!(h.navigate(h.entries[h.cursor], false).is_some());
        }
        assert!(h.navigate(h.entries[h.cursor], false).is_none());
    }
}
