use serde::{Deserialize, Serialize};
use bevy::prelude::*;
use super::Range;

/// Index of a [`VariableDecl`] within [`super::ParticlesAsset::variables`].
///
/// Typed rather than a name string on purpose: a `Drive` naming its variable by
/// index can be rejected at load if the index is out of range, instead of
/// silently falling through to render stock forever the way a typo'd name would.
/// Names exist for humans and for the host API; the file stores indices.
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize, Reflect)]
pub struct VariableId(pub u16);

/// One knob an effect exposes to the host game.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, Reflect)]
pub struct VariableDecl {
    /// The name the host uses: `vars.set("temperature", 0.8)`.
    pub name: String,
    /// The value used when the host sets nothing.
    pub default: f32,
    /// Authored bounds. Drives the editor slider's range; NOT a clamp on what
    /// the host may set, because a host legitimately overshoots for punch.
    pub range: Range,
}

impl Default for VariableDecl {
    fn default() -> Self {
        Self { name: String::new(), default: 0.0, range: Range { min: 0.0, max: 1.0 } }
    }
}
