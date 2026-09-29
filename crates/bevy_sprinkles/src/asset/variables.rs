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
    /// Authored bounds, and the domain every drive's curve is read over: a host
    /// value is mapped `(v - min) / span` before it samples a curve
    /// (`crate::drives`'s `normalize_to_curve_domain`), so declaring
    /// `(0, 100)` gives a curve that spans all hundred rather than one living
    /// inside its first percent. Also the editor slider's range.
    ///
    /// NOT a clamp on what the host may set, because a host legitimately
    /// overshoots for punch: a value past `max` maps past `1.0` and is USED --
    /// never rejected, wrapped, or swapped for the default -- where the curve
    /// holds at its endpoint.
    pub range: Range,
}

impl Default for VariableDecl {
    fn default() -> Self {
        Self { name: String::new(), default: 0.0, range: Range { min: 0.0, max: 1.0 } }
    }
}
