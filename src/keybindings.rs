//! Serializable key bindings with reserved editing/navigation shortcuts.
use eframe::egui;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(usize)]
pub(crate) enum Action {
    Pan,
    Line,
    Rectangle,
    Pointer,
    Circle,
    Freehand,
    Polygon,
    Mag1,
    Mag2,
    Mag3,
    Mag4,
    Mag5,
    Fit,
    Rotate,
    PreviousZ,
    NextZ,
    Sidebar,
    Minimap,
    Scale,
    CleanView,
    Overview,
}
pub(crate) const ACTIONS: [Action; 21] = [
    Action::Pan,
    Action::Line,
    Action::Rectangle,
    Action::Pointer,
    Action::Circle,
    Action::Freehand,
    Action::Polygon,
    Action::Mag1,
    Action::Mag2,
    Action::Mag3,
    Action::Mag4,
    Action::Mag5,
    Action::Fit,
    Action::Rotate,
    Action::PreviousZ,
    Action::NextZ,
    Action::Sidebar,
    Action::Minimap,
    Action::Scale,
    Action::CleanView,
    Action::Overview,
];
pub(crate) const MAG_ACTIONS: [Action; 5] = [
    Action::Mag1,
    Action::Mag2,
    Action::Mag3,
    Action::Mag4,
    Action::Mag5,
];
impl Action {
    pub fn label(self) -> &'static str {
        [
            "Pan / select",
            "Measure length",
            "Rectangle",
            "Arrow annotation",
            "Circle",
            "Freehand",
            "Polygon",
            "2×",
            "4×",
            "10×",
            "20×",
            "40×",
            "Fit slide",
            "Rotate",
            "Previous Z",
            "Next Z",
            "Sidebar",
            "Minimap",
            "Scale bar",
            "Hide all tools",
            "Overview / return",
        ][self as usize]
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Binding {
    #[serde(with = "key_serde")]
    pub key: egui::Key,
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
}
mod key_serde {
    use super::*;
    pub fn serialize<S: serde::Serializer>(
        key: &egui::Key,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(key.name())
    }
    pub fn deserialize<'de, D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> Result<egui::Key, D::Error> {
        let name = String::deserialize(deserializer)?;
        egui::Key::from_name(&name).ok_or_else(|| serde::de::Error::custom("Unknown shortcut key"))
    }
}
impl Binding {
    pub fn plain(key: egui::Key) -> Self {
        Self {
            key,
            ctrl: false,
            alt: false,
            shift: false,
        }
    }
    pub fn modifiers(self) -> egui::Modifiers {
        egui::Modifiers {
            ctrl: self.ctrl,
            alt: self.alt,
            shift: self.shift,
            ..Default::default()
        }
    }
    pub fn label(self) -> String {
        format!(
            "{}{}{}{}",
            if self.ctrl { "Ctrl+" } else { "" },
            if self.alt { "Alt+" } else { "" },
            if self.shift { "Shift+" } else { "" },
            self.key.name()
        )
    }
    pub fn consume(self, ctx: &egui::Context) -> bool {
        ctx.input_mut(|i| {
            let found = i.events.iter().any(|e| matches!(e, egui::Event::Key { key, pressed: true, modifiers, .. } if *key == self.key && *modifiers == self.modifiers()));
            found && i.consume_key(self.modifiers(), self.key)
        })
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Bindings(pub [Binding; 21]);
impl Default for Bindings {
    fn default() -> Self {
        use egui::Key::*;
        Self(
            [
                Q, W, E, A, S, D, C, Num1, Num2, Num3, Num4, Num5, F, R, Z, X, V, G, B, H, Tab,
            ]
            .map(Binding::plain),
        )
    }
}
impl Bindings {
    pub fn get(self, action: Action) -> Binding {
        self.0[action as usize]
    }
    pub fn error(&self) -> Option<String> {
        use egui::Key::*;
        for (index, binding) in self.0.iter().enumerate() {
            if let Some(other) = self.0[..index].iter().position(|b| b == binding) {
                return Some(format!(
                    "{} is already assigned to {}.",
                    binding.label(),
                    ACTIONS[other].label()
                ));
            }
            let reserved = if binding.shift && binding.key == Slash {
                true
            } else if binding.ctrl {
                matches!(binding.key, C | V | X | A | Z | Y)
            } else if binding.alt {
                matches!(binding.key, F | M | ArrowLeft | ArrowRight | F4)
            } else {
                matches!(
                    binding.key,
                    Escape
                        | Space
                        | ArrowLeft
                        | ArrowRight
                        | ArrowUp
                        | ArrowDown
                        | PageUp
                        | PageDown
                        | F11
                        | Enter
                        | Delete
                        | T
                        | L
                        | N
                        | P
                        | Questionmark
                ) || (binding.key == Tab && ACTIONS[index] != Action::Overview)
            };
            if reserved {
                return Some(format!(
                    "{} is reserved for editing or navigation.",
                    binding.label()
                ));
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bindings_roundtrip_reject_conflicts_and_reserve_standard_shortcuts() {
        let mut bindings = Bindings::default();
        assert!(bindings.error().is_none());
        bindings.0[Action::Line as usize] = Binding::plain(egui::Key::J);
        let encoded = serde_json::to_string(&bindings).unwrap();
        assert_eq!(
            serde_json::from_str::<Bindings>(&encoded).unwrap(),
            bindings
        );
        bindings.0[Action::Circle as usize] = Binding::plain(egui::Key::J);
        assert!(bindings.error().unwrap().contains("already assigned"));
        bindings = Bindings::default();
        bindings.0[Action::Line as usize] = Binding {
            key: egui::Key::V,
            ctrl: true,
            alt: false,
            shift: false,
        };
        assert!(bindings.error().unwrap().contains("reserved"));
        bindings.0[Action::Line as usize] = Binding::plain(egui::Key::Escape);
        assert!(bindings.error().is_some());
        assert!(Bindings::default().error().is_none());
    }
}
