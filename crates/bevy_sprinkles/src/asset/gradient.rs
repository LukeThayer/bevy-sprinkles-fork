use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use std::hash::{Hash, Hasher};

/// Interpolation mode for sampling between gradient stops.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq, Hash, Reflect)]
pub enum GradientInterpolation {
    /// No interpolation. Holds the left stop's color until the next stop.
    Steps,
    /// Linear interpolation between stops.
    #[default]
    Linear,
    /// Smooth Hermite interpolation between stops, producing softer transitions.
    Smoothstep,
}

impl GradientInterpolation {
    pub(crate) fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

/// A single color stop within a [`Gradient`].
#[derive(Debug, Clone, Serialize, Deserialize, Reflect)]
pub struct GradientStop {
    /// The color at this stop, as linear RGBA values in `[0.0, 1.0]`.
    pub color: [f32; 4],
    /// Position of this stop along the gradient, from `0.0` (start) to `1.0` (end).
    pub position: f32,
}

/// A color gradient defined by a series of [`GradientStop`]s.
///
/// Gradients are baked into 1D textures for efficient GPU sampling. The
/// [`interpolation`](Self::interpolation) mode controls how colors are blended
/// between stops.
#[derive(Debug, Clone, Serialize, Deserialize, Reflect)]
pub struct Gradient {
    /// The ordered list of color stops that define this gradient.
    pub stops: Vec<GradientStop>,
    /// Interpolation mode between stops. Defaults to [`GradientInterpolation::Linear`].
    #[serde(default, skip_serializing_if = "GradientInterpolation::is_default")]
    pub interpolation: GradientInterpolation,
}

impl Default for Gradient {
    fn default() -> Self {
        Self {
            stops: vec![
                GradientStop {
                    color: [0.0, 0.0, 0.0, 1.0],
                    position: 0.0,
                },
                GradientStop {
                    color: [1.0, 1.0, 1.0, 1.0],
                    position: 1.0,
                },
            ],
            interpolation: GradientInterpolation::Linear,
        }
    }
}

impl Gradient {
    /// Creates a constant white gradient (white at both ends).
    pub fn white() -> Self {
        Self {
            stops: vec![
                GradientStop {
                    color: [1.0, 1.0, 1.0, 1.0],
                    position: 0.0,
                },
                GradientStop {
                    color: [1.0, 1.0, 1.0, 1.0],
                    position: 1.0,
                },
            ],
            interpolation: GradientInterpolation::Linear,
        }
    }

    /// Computes a hash key for texture caching, based on all stops and the interpolation mode.
    pub fn cache_key(&self) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        for stop in &self.stops {
            for c in stop.color {
                c.to_bits().hash(&mut hasher);
            }
            stop.position.to_bits().hash(&mut hasher);
        }
        self.interpolation.hash(&mut hasher);
        hasher.finish()
    }
}

/// A color that is either a single solid value or a gradient.
///
/// When used as an initial particle color, [`Solid`](Self::Solid) applies the same color
/// to every particle, while [`Gradient`](Self::Gradient) samples a random position along
/// the gradient for each particle.
#[derive(Debug, Clone, Serialize, Deserialize, Reflect)]
#[reflect(Clone)]
pub enum SolidOrGradientColor {
    /// A single solid color, as linear RGBA values in `[0.0, 1.0]`.
    Solid {
        /// The color value.
        color: [f32; 4],
    },
    /// A gradient from which colors are sampled.
    Gradient {
        /// The gradient definition.
        gradient: Gradient,
    },
}

impl Default for SolidOrGradientColor {
    fn default() -> Self {
        Self::Solid {
            color: [1.0, 1.0, 1.0, 1.0],
        }
    }
}

impl SolidOrGradientColor {
    /// Creates a [`SolidOrGradientColor::Solid`] with the given linear RGBA color.
    pub fn solid(color: [f32; 4]) -> Self {
        Self::Solid { color }
    }

    /// Returns `true` if this is a solid color.
    pub fn is_solid(&self) -> bool {
        matches!(self, Self::Solid { .. })
    }

    /// Returns `true` if this is a gradient.
    pub fn is_gradient(&self) -> bool {
        matches!(self, Self::Gradient { .. })
    }

    /// Returns the solid color value, or `None` if this is a gradient.
    pub fn as_solid_color(&self) -> Option<[f32; 4]> {
        match self {
            Self::Solid { color } => Some(*color),
            _ => None,
        }
    }
}
