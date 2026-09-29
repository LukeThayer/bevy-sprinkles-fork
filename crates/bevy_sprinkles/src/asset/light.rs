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
/// way an emitter's is. Reusing the whole struct means carrying fields a light
/// cannot obey; [`time`](Self::time) says which ones are live.
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
    /// Timing/lifecycle, authored the same way an emitter's is — but only
    /// THREE of [`EmitterTime`]'s fields mean anything to a light, and
    /// `crate::lights` reads exactly those:
    ///
    /// - [`lifetime`](EmitterTime::lifetime) and [`delay`](EmitterTime::delay),
    ///   via `total_duration()` and `compute_phase`, position the light within
    ///   its cycle; `delay` also holds it dark until it elapses.
    /// - [`one_shot`](EmitterTime::one_shot) stops the clock after one
    ///   completed cycle, and the light stays dark from then on.
    ///
    /// The rest — `lifetime_randomness`, `spawn_time_randomness`,
    /// `explosiveness`, `fixed_fps`, `fixed_seed` — describe per-particle
    /// spawn jitter and simulation stepping. A light spawns no particles and
    /// steps no simulation, so nothing reads them and the editor does not paint
    /// them (`inspector::light::light_time_section`). They persist in the file
    /// because the struct is shared, not because they do anything.
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
