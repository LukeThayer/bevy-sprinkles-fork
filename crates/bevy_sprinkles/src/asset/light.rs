use serde::{Deserialize, Serialize};
use bevy::prelude::*;
use super::{CurveTexture, EmitterTime, InitialTransform};

/// The kind of light a [`LightData`] spawns.
#[derive(Copy, Clone, PartialEq, Eq, Debug, Default, Serialize, Deserialize, Reflect)]
pub enum FxLightKind {
    /// An omnidirectional point light.
    #[default]
    Point,
    /// A directional cone light.
    Spot,
}

/// A light the effect owns, spawned as a child entity of the effect.
///
/// Reuses [`EmitterTime`] rather than inventing a timing vocabulary, so a
/// muzzle flash's delay / lifetime / one-shot / loop is authored exactly the
/// way an emitter's is.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, Reflect)]
pub struct LightData {
    /// Display name for this light.
    pub name: String,
    /// Whether this light is active.
    pub enabled: bool,
    /// Point or spot.
    pub kind: FxLightKind,
    /// Initial transform of the light's child entity, relative to the effect.
    pub transform: InitialTransform,
    /// Base color of the light.
    pub color: Color,
    /// HDR. Blowing out into bloom is the intended use.
    pub intensity: f32,
    /// How far the light reaches.
    pub range: f32,
    /// Timing/lifecycle, authored the same way an emitter's is.
    pub time: EmitterTime,
    /// CPU-sampled against this light's own phase. NOT baked to a texture —
    /// it produces one scalar per frame, not a per-particle ramp.
    pub intensity_over_life: Option<CurveTexture>,
    /// Whether this light casts shadows.
    pub shadows: bool,
}

impl Default for LightData {
    fn default() -> Self {
        Self {
            name: "Light".into(),
            enabled: true,
            kind: FxLightKind::Point,
            transform: InitialTransform::default(),
            color: Color::WHITE,
            intensity: 100_000.0,
            range: 8.0,
            time: EmitterTime::default(),
            intensity_over_life: None,
            shadows: false,
        }
    }
}
