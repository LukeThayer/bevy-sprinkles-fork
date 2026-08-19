/// Baked texture generation and caching for gradients and curves.
///
/// Sprinkles represents [`Gradient`](crate::asset::Gradient)s and
/// [`CurveTexture`](crate::asset::CurveTexture)s as 1D textures on the GPU,
/// allowing shaders to sample them with a simple UV lookup instead of
/// evaluating the curves at runtime.
///
/// # Gradient textures
///
/// A gradient is baked into a 256-wide `Rgba8UnormSrgb` image (1 pixel high).
/// The color at each texel is interpolated from the gradient's stops using its
/// [`GradientInterpolation`](crate::asset::GradientInterpolation) mode.
/// See [`GradientTextureCache`].
///
/// # Curve textures
///
/// A curve is baked into a 256-wide `Rgba8Unorm` grayscale image (1 pixel
/// high, same value in R, G, and B). The value at each texel is sampled from
/// the curve's control points using its interpolation mode and easing function.
/// See [`CurveTextureCache`].
///
/// # Caching
///
/// Every gradient and curve produces a `cache_key()` hash from its data.
/// Equal gradients or curves will produce the same hash and map to the
/// same baked texture automatically, so no duplicate textures are created
/// regardless of how many emitters use them. Constant curves (all points
/// with a `1.0` value) skip baking entirely.
///
/// A [`FallbackGradientTexture`] and [`FallbackCurveTexture`] (1x1 white)
/// are created at startup so shaders always have a valid texture binding.
pub mod baked;
/// Preset particle textures and texture reference types.
pub mod preset;

pub use baked::*;
