use super::*;

fn consume(ctx: &egui::Context, modifiers: egui::Modifiers, key: egui::Key) -> bool {
    ctx.input_mut(|i| {
        let exact = i.events.iter().any(|event| {
            matches!(event,
            egui::Event::Key { key: k, pressed: true, modifiers: m, .. }
                if *k == key && *m == modifiers)
        });
        exact && i.consume_key(modifiers, key)
    })
}

impl Viewer {
    // Run before toolbar widgets so Tab cannot give them focus first.
    pub(crate) fn handle_overview_shortcut(&mut self, ctx: &egui::Context) -> bool {
        if self.slide.is_none()
            || self.benchmark.is_some()
            || ctx.wants_keyboard_input()
            || ctx.memory(|m| m.any_popup_open())
        {
            return false;
        }
        if !consume(ctx, egui::Modifiers::NONE, egui::Key::Tab) {
            return false;
        }
        ctx.memory_mut(|m| {
            let id = egui::Id::new("overview-shortcut-focus");
            m.interested_in_focus(id, egui::LayerId::background());
            m.surrender_focus(id);
        });
        self.toggle_overview();
        true
    }

    pub(crate) fn temporary_pan(&self, ctx: &egui::Context) -> bool {
        !ctx.wants_keyboard_input()
            && !ctx.memory(|m| m.any_popup_open())
            && ctx.input(|i| i.modifiers.is_none() && i.key_down(egui::Key::Space))
    }

    pub(crate) fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        if self.benchmark.is_some()
            || ctx.wants_keyboard_input()
            || ctx.memory(|m| m.any_popup_open())
            || self.slide.is_none()
        {
            return;
        }
        let undo = consume(ctx, egui::Modifiers::CTRL, egui::Key::Z);
        let redo = consume(ctx, egui::Modifiers::CTRL, egui::Key::Y);
        if undo || redo {
            if undo {
                self.features.undo_edit();
            } else {
                self.features.redo_edit();
            }
            if let Some(slide) = &self.slide {
                self.features.save(slide);
            }
            return;
        }
        if consume(ctx, egui::Modifiers::NONE, egui::Key::Delete) {
            if let Some(index) = self.features.selected {
                self.features.remove_annotation(index);
                if let Some(slide) = &self.slide {
                    self.features.save(slide);
                }
            }
            return;
        }
        if consume(ctx, egui::Modifiers::ALT, egui::Key::M) {
            self.toggle_mirror(0);
            return;
        }
        if consume(ctx, egui::Modifiers::ALT, egui::Key::F) {
            self.toggle_mirror(1);
            return;
        }
        if self.handle_overview_shortcut(ctx) {
            return;
        }
        if consume(ctx, egui::Modifiers::NONE, egui::Key::B) {
            self.features.show_scale = !self.features.show_scale;
            return;
        }
        // egui-winit translates Ctrl+C (including Ctrl+Shift+C) to Copy.
        // Text inputs are excluded by the guard above.
        if !self.features.info_open
            && ctx.input(|i| {
                i.events
                    .iter()
                    .any(|event| matches!(event, egui::Event::Copy))
            })
        {
            self.request_capture(ctx, CaptureTarget::Clipboard);
            return;
        }
        // Match the modifiers attached to the key event, even if Alt was
        // released before this frame. Consume arrows before the pan bindings.
        if consume(ctx, egui::Modifiers::ALT, egui::Key::ArrowLeft) {
            self.navigate_history(false);
            return;
        }
        if consume(ctx, egui::Modifiers::ALT, egui::Key::ArrowRight) {
            self.navigate_history(true);
            return;
        }
        for (key, tool) in [
            (egui::Key::Q, Tool::Pan),
            (egui::Key::W, Tool::Line),
            (egui::Key::E, Tool::Rectangle),
            (egui::Key::A, Tool::Pointer),
            (egui::Key::S, Tool::Circle),
            (egui::Key::D, Tool::Freehand),
            (egui::Key::C, Tool::Polygon),
        ] {
            if consume(ctx, egui::Modifiers::NONE, key) {
                self.features.cancel_gesture();
                self.features.tool = tool;
                if tool != Tool::Pan {
                    self.features.inspector = inspector::Tab::Annotations;
                }
                return;
            }
        }
        for (key, requested) in [
            (egui::Key::Num1, 2.0),
            (egui::Key::Num2, 4.0),
            (egui::Key::Num3, 10.0),
            (egui::Key::Num4, 20.0),
            (egui::Key::Num5, 40.0),
        ] {
            if consume(ctx, egui::Modifiers::NONE, key) {
                self.set_magnification(requested);
                return;
            }
        }
        ctx.input(|i| {
            if !i.modifiers.is_none() {
                return;
            }
            if i.key_pressed(egui::Key::F) {
                self.fit = true;
            }
            if i.key_pressed(egui::Key::R) {
                self.features.cancel_gesture();
                self.features.rotation = (self.features.rotation + 1) % 4;
                self.changed();
            }
            if i.key_pressed(egui::Key::Escape) {
                self.features.tool = Tool::Pan;
                self.features.cancel_gesture();
            }
            if i.key_pressed(egui::Key::PageUp) || i.key_pressed(egui::Key::Z) {
                self.features.cancel_gesture();
                self.plane = self.plane.saturating_sub(1);
                self.changed();
            }
            if i.key_pressed(egui::Key::PageDown) || i.key_pressed(egui::Key::X) {
                self.features.cancel_gesture();
                self.plane = (self.plane + 1).min(self.slide.as_ref().unwrap().planes - 1);
                self.changed();
            }
            let mut delta = egui::Vec2::ZERO;
            for (key, step) in [
                (egui::Key::ArrowLeft, egui::vec2(-100.0, 0.0)),
                (egui::Key::ArrowRight, egui::vec2(100.0, 0.0)),
                (egui::Key::ArrowUp, egui::vec2(0.0, -100.0)),
                (egui::Key::ArrowDown, egui::vec2(0.0, 100.0)),
            ] {
                if i.key_pressed(key) {
                    delta += step;
                }
            }
            if delta != egui::Vec2::ZERO {
                self.center += self.inverse_transform(delta) / self.zoom;
                self.changed();
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(viewer: &mut Viewer, ctx: &egui::Context, key: egui::Key, modifiers: egui::Modifiers) {
        let _ = ctx.run(
            egui::RawInput {
                modifiers,
                events: vec![egui::Event::Key {
                    key,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers,
                }],
                ..Default::default()
            },
            |ctx| {
                viewer.handle_shortcuts(ctx);
                egui::CentralPanel::default().show(ctx, |ui| {
                    let _ = ui.button("Toolbar control");
                });
            },
        );
    }

    #[test]
    fn tools_focus_and_magnification_obey_modifiers() {
        let (mut viewer, _) = crate::tests::tile_viewer();
        let ctx = egui::Context::default();
        for (key, tool) in [
            (egui::Key::Q, Tool::Pan),
            (egui::Key::W, Tool::Line),
            (egui::Key::E, Tool::Rectangle),
            (egui::Key::A, Tool::Pointer),
            (egui::Key::S, Tool::Circle),
            (egui::Key::D, Tool::Freehand),
            (egui::Key::C, Tool::Polygon),
        ] {
            frame(&mut viewer, &ctx, key, egui::Modifiers::NONE);
            assert!(viewer.features.tool == tool);
        }
        frame(&mut viewer, &ctx, egui::Key::S, egui::Modifiers::CTRL);
        assert!(viewer.features.tool == Tool::Polygon);
        frame(&mut viewer, &ctx, egui::Key::E, egui::Modifiers::SHIFT);
        assert!(viewer.features.tool == Tool::Polygon);
        viewer.plane = 1;
        frame(&mut viewer, &ctx, egui::Key::Z, egui::Modifiers::CTRL);
        assert_eq!(viewer.plane, 1);
        frame(&mut viewer, &ctx, egui::Key::Z, egui::Modifiers::NONE);
        assert_eq!(viewer.plane, 0);
        let mut slide = crate::ndpa::test_slide();
        slide.ndpa.as_mut().unwrap().lens = 40.0;
        viewer.slide = Some(slide);
        for (key, zoom) in [
            (egui::Key::Num1, 0.05),
            (egui::Key::Num2, 0.1),
            (egui::Key::Num3, 0.25),
            (egui::Key::Num4, 0.5),
            (egui::Key::Num5, 1.0),
        ] {
            frame(&mut viewer, &ctx, key, egui::Modifiers::NONE);
            assert_eq!(viewer.zoom, zoom);
        }
    }

    #[test]
    fn overview_and_space_preserve_tool_and_missing_metadata_preserves_zoom() {
        let (mut viewer, _) = crate::tests::tile_viewer();
        let ctx = egui::Context::default();
        viewer.features.tool = Tool::Circle;
        viewer.features.exact_shape.enabled = true;
        viewer.zoom = 0.37;
        frame(&mut viewer, &ctx, egui::Key::Num1, egui::Modifiers::NONE);
        assert_eq!(viewer.zoom, 0.37);
        frame(&mut viewer, &ctx, egui::Key::Tab, egui::Modifiers::NONE);
        assert!(viewer.features.overview_return.is_some());
        assert!(!ctx.wants_keyboard_input());
        frame(&mut viewer, &ctx, egui::Key::Space, egui::Modifiers::NONE);
        assert!(viewer.temporary_pan(&ctx));
        assert!(viewer.features.tool == Tool::Circle);
        assert!(viewer.features.exact_shape.enabled);
        let _ = ctx.run(
            egui::RawInput {
                events: vec![egui::Event::Key {
                    key: egui::Key::Space,
                    physical_key: None,
                    pressed: false,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                }],
                ..Default::default()
            },
            |ctx| {
                viewer.handle_shortcuts(ctx);
            },
        );
        assert!(!viewer.temporary_pan(&ctx));
        assert!(viewer.features.tool == Tool::Circle);
    }

    #[test]
    fn focused_text_and_popups_keep_keys_and_disable_temporary_pan() {
        let (mut viewer, _) = crate::tests::tile_viewer();
        let ctx = egui::Context::default();
        let mut text = String::new();
        let id = egui::Id::new("shortcut-test-input");
        let _ = ctx.run(Default::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.add(egui::TextEdit::singleline(&mut text).id(id))
                    .request_focus();
            });
        });
        for key in [
            egui::Key::S,
            egui::Key::Num1,
            egui::Key::Z,
            egui::Key::Tab,
            egui::Key::Space,
        ] {
            let old_zoom = viewer.zoom;
            let old_plane = viewer.plane;
            let _ = ctx.run(
                egui::RawInput {
                    events: vec![
                        egui::Event::Text("s12345zx ".into()),
                        egui::Event::Key {
                            key,
                            physical_key: None,
                            pressed: true,
                            repeat: false,
                            modifiers: egui::Modifiers::NONE,
                        },
                    ],
                    ..Default::default()
                },
                |ctx| {
                    viewer.handle_shortcuts(ctx);
                    assert!(!viewer.temporary_pan(ctx));
                    egui::CentralPanel::default().show(ctx, |ui| {
                        ui.add(egui::TextEdit::singleline(&mut text).id(id))
                            .request_focus();
                    });
                },
            );
            assert!(viewer.features.tool == Tool::Pan);
            assert_eq!(viewer.zoom, old_zoom);
            assert_eq!(viewer.plane, old_plane);
        }
        assert!(text.contains("s12345zx"));
        ctx.memory_mut(|m| {
            m.surrender_focus(id);
            m.open_popup(egui::Id::new("test-popup"));
        });
        frame(&mut viewer, &ctx, egui::Key::S, egui::Modifiers::NONE);
        assert!(viewer.features.tool == Tool::Pan);
    }
}
