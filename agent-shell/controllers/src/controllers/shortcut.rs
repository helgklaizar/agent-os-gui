//! `ShortcutController`: turns a key press into a bus event using the [`crate::keys::resolve`] table.

use crate::time::Instant;

use crate::controller::{Controller, Effect};
use crate::keys::{Focus, KeyPress, resolve};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Msg {
    Key {
        press: KeyPress,
        focus: Focus,
    },
    /// How many dialogs and palettes are open now.
    Layers(usize),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ShortcutView {
    pub layers: usize,
}

#[derive(Debug, Default)]
pub struct ShortcutController {
    layers: usize,
}

impl ShortcutController {
    /// Whether to stop the key press from propagating: `false` means the terminal and browser receive the key themselves.
    pub fn consumes(&self, press: KeyPress, focus: Focus) -> bool {
        resolve(press, focus, self.layers).is_some()
    }
}

impl Controller for ShortcutController {
    type Msg = Msg;
    type View = ShortcutView;

    fn update(&mut self, msg: Msg, _now: Instant) -> Vec<Effect> {
        match msg {
            Msg::Layers(n) => {
                self.layers = n;
                Vec::new()
            }
            Msg::Key { press, focus } => resolve(press, focus, self.layers)
                .map(Effect::Publish)
                .into_iter()
                .collect(),
        }
    }

    fn view(&self) -> ShortcutView {
        ShortcutView {
            layers: self.layers,
        }
    }
}
