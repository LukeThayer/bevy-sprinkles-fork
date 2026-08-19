use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use std::hash::{Hash, Hasher};
use std::str::FromStr;

use super::Range;

/// Interpolation mode between two [`CurvePoint`]s.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Default, Reflect)]
pub enum CurveMode {
    /// A single easing function applied across the entire segment.
    SingleCurve,
    /// Two easing functions, one for each half of the segment, producing an
    /// S-curve shape.
    #[default]
    DoubleCurve,
    /// No interpolation; holds the left point's value for the entire segment.
    Hold,
    /// Staircase interpolation with discrete steps. The number of steps is
    /// derived from the tension parameter.
    Stairs,
    /// Staircase interpolation with smooth transitions between steps.
    SmoothStairs,
}

impl FromStr for CurveMode {
    type Err = ();
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "SingleCurve" => Ok(Self::SingleCurve),
            "DoubleCurve" => Ok(Self::DoubleCurve),
            "Hold" => Ok(Self::Hold),
            "Stairs" => Ok(Self::Stairs),
            "SmoothStairs" => Ok(Self::SmoothStairs),
            _ => Err(()),
        }
    }
}

/// The easing function used when interpolating between curve points.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Default, Reflect)]
pub enum CurveEasing {
    /// Power-based easing. The exponent is derived from the tension parameter.
    #[default]
    Power,
    /// Sinusoidal easing.
    Sine,
    /// Exponential easing.
    Expo,
    /// Circular easing.
    Circ,
}

impl FromStr for CurveEasing {
    type Err = ();
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "Power" => Ok(Self::Power),
            "Sine" => Ok(Self::Sine),
            "Expo" => Ok(Self::Expo),
            "Circ" => Ok(Self::Circ),
            _ => Err(()),
        }
    }
}

fn default_tension() -> f64 {
    0.0
}

/// A single control point in a [`CurveTexture`].
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Reflect)]
pub struct CurvePoint {
    /// Horizontal position along the curve, from `0.0` (start) to `1.0` (end).
    pub position: f32,
    /// The value at this point, typically in `[0.0, 1.0]`.
    pub value: f64,
    /// Interpolation mode for the segment leading to this point.
    #[serde(default)]
    pub mode: CurveMode,
    /// Tension parameter that controls the curvature. The effect depends on the
    /// [`mode`](Self::mode) and [`easing`](Self::easing). Defaults to `0.0` (linear).
    #[serde(default = "default_tension")]
    pub tension: f64,
    /// Easing function applied within this segment.
    #[serde(default)]
    pub easing: CurveEasing,
}

impl CurvePoint {
    /// Creates a new curve point at the given position with the given value.
    pub fn new(position: f32, value: f64) -> Self {
        Self {
            position,
            value,
            mode: CurveMode::default(),
            tension: 0.0,
            easing: CurveEasing::default(),
        }
    }

    /// Sets the interpolation mode for this point's segment.
    pub fn with_mode(mut self, mode: CurveMode) -> Self {
        self.mode = mode;
        self
    }

    /// Sets the tension parameter for this point's segment.
    pub fn with_tension(mut self, tension: f64) -> Self {
        self.tension = tension;
        self
    }

    /// Sets the easing function for this point's segment.
    pub fn with_easing(mut self, easing: CurveEasing) -> Self {
        self.easing = easing;
        self
    }
}

fn is_empty_string(s: &Option<String>) -> bool {
    s.as_ref().is_none_or(|s| s.is_empty())
}

/// A single channel of a [`CurveTexture`], pairing control points with an output range.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Reflect)]
pub struct Curve {
    /// The control points that define this channel's shape.
    pub points: Vec<CurvePoint>,
    /// The output range for this channel. Defaults to `0.0..1.0`.
    #[serde(default)]
    pub range: Range,
}

impl Default for Curve {
    fn default() -> Self {
        Self {
            points: vec![CurvePoint::new(0.0, 1.0), CurvePoint::new(1.0, 1.0)],
            range: Range::new(0.0, 1.0),
        }
    }
}

impl Curve {
    /// Creates a new curve from the given control points with a default range.
    pub fn new(points: Vec<CurvePoint>) -> Self {
        Self {
            points,
            range: Range::default(),
        }
    }

    /// Sets the output range for this curve.
    pub fn with_range(mut self, range: Range) -> Self {
        self.range = range;
        self
    }
}

/// A piecewise curve defined by control points, baked into a 1D texture for GPU sampling.
///
/// Curve textures are used to animate particle properties (scale, alpha, velocity, etc.)
/// over each particle's lifetime. The curve maps a normalized lifetime position `[0.0, 1.0]`
/// to an output value, which is then scaled by the channel's [`range`](Curve::range).
///
/// Each curve texture has a primary X channel and optionally separate Y and Z channels.
/// When `y` or `z` is `None`, those channels fall back to the X channel. This allows a
/// single `CurveTexture` to represent both scalar curves and per-axis curves without a
/// separate type.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Reflect)]
pub struct CurveTexture {
    /// Optional display name for this curve (e.g., "Constant", "Fade Out").
    #[serde(default, skip_serializing_if = "is_empty_string")]
    pub name: Option<String>,
    /// The X (primary) channel.
    pub x: Curve,
    /// Optional Y channel. Falls back to `x` when `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub y: Option<Curve>,
    /// Optional Z channel. Falls back to `x` when `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub z: Option<Curve>,
}

impl Default for CurveTexture {
    fn default() -> Self {
        Self {
            name: Some("Constant".to_string()),
            x: Curve::default(),
            y: None,
            z: None,
        }
    }
}

impl CurveTexture {
    /// Creates a new single-channel curve from the given control points with a default range.
    pub fn new(points: Vec<CurvePoint>) -> Self {
        Self {
            name: None,
            x: Curve::new(points),
            y: None,
            z: None,
        }
    }

    /// Creates a new three-channel curve with separate control points per axis.
    pub fn new_xyz(
        points_x: Vec<CurvePoint>,
        points_y: Vec<CurvePoint>,
        points_z: Vec<CurvePoint>,
    ) -> Self {
        Self {
            name: None,
            x: Curve::new(points_x),
            y: Some(Curve::new(points_y)),
            z: Some(Curve::new(points_z)),
        }
    }

    /// Sets the display name for this curve.
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Sets the output range for the X (primary) channel.
    pub fn with_range(mut self, range: Range) -> Self {
        self.x.range = range;
        self
    }

    /// Returns the effective range for the Y channel, falling back to the X range when unset.
    pub fn effective_range_y(&self) -> &Range {
        self.y.as_ref().map(|c| &c.range).unwrap_or(&self.x.range)
    }

    /// Returns the effective range for the Z channel, falling back to the X range when unset.
    pub fn effective_range_z(&self) -> &Range {
        self.z.as_ref().map(|c| &c.range).unwrap_or(&self.x.range)
    }

    /// Computes a hash key for texture caching.
    pub fn cache_key(&self) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        hash_points(&self.x.points, &mut hasher);
        self.x.range.min.to_bits().hash(&mut hasher);
        self.x.range.max.to_bits().hash(&mut hasher);
        if let Some(y) = &self.y {
            1u8.hash(&mut hasher);
            hash_points(&y.points, &mut hasher);
            y.range.min.to_bits().hash(&mut hasher);
            y.range.max.to_bits().hash(&mut hasher);
        } else {
            0u8.hash(&mut hasher);
        }
        if let Some(z) = &self.z {
            1u8.hash(&mut hasher);
            hash_points(&z.points, &mut hasher);
            z.range.min.to_bits().hash(&mut hasher);
            z.range.max.to_bits().hash(&mut hasher);
        } else {
            0u8.hash(&mut hasher);
        }
        hasher.finish()
    }

    /// Returns `true` if all channels are flat and would produce the same output,
    /// meaning the curve can be safely skipped without affecting the result.
    pub fn is_constant(&self) -> bool {
        if !points_are_constant(&self.x.points) {
            return false;
        }
        let x_value = self.x.points.first().map(|p| p.value).unwrap_or(1.0);

        if let Some(y) = &self.y {
            if !points_are_constant(&y.points) {
                return false;
            }
            let y_value = y.points.first().map(|p| p.value).unwrap_or(1.0);
            if (y_value - x_value).abs() > f64::EPSILON {
                return false;
            }
            if y.range != self.x.range {
                return false;
            }
        }
        if let Some(z) = &self.z {
            if !points_are_constant(&z.points) {
                return false;
            }
            let z_value = z.points.first().map(|p| p.value).unwrap_or(1.0);
            if (z_value - x_value).abs() > f64::EPSILON {
                return false;
            }
            if z.range != self.x.range {
                return false;
            }
        }

        true
    }

    /// Samples the X (primary) channel at position `t` (clamped to `[0.0, 1.0]`).
    pub fn sample(&self, t: f32) -> f32 {
        sample_points(&self.x.points, t)
    }

    /// Samples a specific channel at position `t`. Channel 0 is X, 1 is Y, 2 is Z.
    /// Y and Z fall back to the X channel when unset.
    pub fn sample_channel(&self, channel: usize, t: f32) -> f32 {
        let points = match channel {
            1 => self
                .y
                .as_ref()
                .map(|c| &c.points[..])
                .unwrap_or(&self.x.points),
            2 => self
                .z
                .as_ref()
                .map(|c| &c.points[..])
                .unwrap_or(&self.x.points),
            _ => &self.x.points,
        };
        sample_points(points, t)
    }
}

fn hash_points(points: &[CurvePoint], hasher: &mut impl Hasher) {
    for point in points {
        point.position.to_bits().hash(hasher);
        (point.value as f32).to_bits().hash(hasher);
        std::mem::discriminant(&point.mode).hash(hasher);
        (point.tension as f32).to_bits().hash(hasher);
    }
}

fn points_are_constant(points: &[CurvePoint]) -> bool {
    if points.len() < 2 {
        return true;
    }
    let first_value = points[0].value;
    points
        .iter()
        .all(|p| (p.value - first_value).abs() < f64::EPSILON)
}

fn sample_points(points: &[CurvePoint], t: f32) -> f32 {
    if points.is_empty() {
        return 1.0;
    }
    if points.len() == 1 {
        return points[0].value as f32;
    }

    let t = t.clamp(0.0, 1.0);

    let mut left_idx = 0;
    let mut right_idx = points.len() - 1;

    for (i, point) in points.iter().enumerate() {
        if point.position <= t {
            left_idx = i;
        }
    }
    for (i, point) in points.iter().enumerate() {
        if point.position >= t {
            right_idx = i;
            break;
        }
    }

    let left = &points[left_idx];
    let right = &points[right_idx];

    if left_idx == right_idx {
        return left.value as f32;
    }

    let segment_range = right.position - left.position;
    if segment_range <= 0.0 {
        return left.value as f32;
    }

    let local_t = (t - left.position) / segment_range;

    let slope_sign = (right.value - left.value).signum() as f32;
    let effective_tension = right.tension as f32 * slope_sign;
    let curved_t = apply_curve(local_t, right.mode, right.easing, effective_tension);

    (left.value + (right.value - left.value) * curved_t as f64) as f32
}

fn apply_curve(t: f32, mode: CurveMode, easing: CurveEasing, tension: f32) -> f32 {
    match mode {
        CurveMode::SingleCurve => apply_easing(t, easing, tension),
        CurveMode::DoubleCurve => {
            if t < 0.5 {
                let local_t = t * 2.0;
                apply_easing(local_t, easing, tension) * 0.5
            } else {
                let local_t = (t - 0.5) * 2.0;
                0.5 + apply_easing(local_t, easing, -tension) * 0.5
            }
        }
        CurveMode::Hold => 0.0,
        CurveMode::Stairs => {
            let steps = tension_to_steps(tension);
            (t * steps as f32).floor() / (steps - 1).max(1) as f32
        }
        CurveMode::SmoothStairs => {
            let steps = tension_to_steps(tension);
            let step_size = 1.0 / steps as f32;
            let current_step = (t / step_size).floor();
            let local_t = (t - current_step * step_size) / step_size;
            let smooth_t = local_t * local_t * (3.0 - 2.0 * local_t);
            let start = current_step / (steps - 1).max(1) as f32;
            let end = (current_step + 1.0).min(steps as f32 - 1.0) / (steps - 1).max(1) as f32;
            start + (end - start) * smooth_t
        }
    }
}

fn apply_easing(t: f32, easing: CurveEasing, tension: f32) -> f32 {
    match easing {
        CurveEasing::Power => apply_power(t, tension),
        CurveEasing::Sine => apply_sine(t, tension),
        CurveEasing::Expo => apply_expo(t, tension),
        CurveEasing::Circ => apply_circ(t, tension),
    }
}

fn apply_power(t: f32, tension: f32) -> f32 {
    if tension.abs() < f32::EPSILON {
        return t;
    }
    let exp = 1.0 / (1.0 - tension.abs() * 0.999);
    if tension > 0.0 {
        t.powf(exp)
    } else {
        1.0 - (1.0 - t).powf(exp)
    }
}

fn apply_sine(t: f32, tension: f32) -> f32 {
    use std::f32::consts::PI;
    let intensity = tension.abs();
    if intensity < f32::EPSILON {
        return t;
    }
    let eased = if tension >= 0.0 {
        1.0 - (t * PI * 0.5).cos()
    } else {
        (t * PI * 0.5).sin()
    };
    t + (eased - t) * intensity
}

fn apply_expo(t: f32, tension: f32) -> f32 {
    let intensity = tension.abs();
    if intensity < f32::EPSILON {
        return t;
    }
    let eased = if tension >= 0.0 {
        if t <= 0.0 {
            0.0
        } else {
            (2.0_f32).powf(10.0 * (t - 1.0))
        }
    } else {
        if t >= 1.0 {
            1.0
        } else {
            1.0 - (2.0_f32).powf(-10.0 * t)
        }
    };
    t + (eased - t) * intensity
}

fn apply_circ(t: f32, tension: f32) -> f32 {
    let intensity = tension.abs();
    if intensity < f32::EPSILON {
        return t;
    }
    let eased = if tension >= 0.0 {
        1.0 - (1.0 - t * t).sqrt()
    } else {
        (1.0 - (1.0 - t) * (1.0 - t)).sqrt()
    };
    t + (eased - t) * intensity
}

fn tension_to_steps(tension: f32) -> u32 {
    let tension = tension.clamp(0.0, 1.0);
    2 + (64.0 * tension) as u32
}
