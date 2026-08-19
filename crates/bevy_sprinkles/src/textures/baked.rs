use bevy::{
    prelude::*,
    render::{
        extract_resource::ExtractResource,
        render_resource::{Extent3d, TextureDimension, TextureFormat, TextureUsages},
    },
};
use std::collections::HashMap;

use crate::asset::{
    CurveTexture, Gradient, GradientInterpolation, ParticlesAsset, SolidOrGradientColor,
};
use crate::r#override::{OverrideBakedTextures, ParticleOverride};
use crate::runtime::{EmitterEntity, EmitterRuntime, Particles3d};

const TEXTURE_WIDTH: u32 = 256;

/// Cache for baked gradient textures, avoiding redundant texture creation.
///
/// Each unique gradient (identified by its [`Gradient::cache_key`]) is baked into
/// a 1D RGBA texture once and reused across all emitters that reference it.
#[derive(Resource, Default)]
pub struct GradientTextureCache {
    cache: HashMap<u64, Handle<Image>>,
}

impl GradientTextureCache {
    /// Returns a cached texture handle for the gradient, creating and baking a new
    /// texture if one doesn't already exist.
    pub fn get_or_create(
        &mut self,
        gradient: &Gradient,
        images: &mut Assets<Image>,
    ) -> Handle<Image> {
        let key = gradient.cache_key();
        if let Some(handle) = self.cache.get(&key) {
            return handle.clone();
        }
        let image = bake_gradient_texture(gradient);
        let handle = images.add(image);
        self.cache.insert(key, handle.clone());
        handle
    }

    /// Returns the cached texture handle for the gradient, if it exists.
    pub fn get(&self, gradient: &Gradient) -> Option<Handle<Image>> {
        self.cache.get(&gradient.cache_key()).cloned()
    }
}

/// Produces the raw 256-wide RGBA8 bytes for a gradient (shared by bake + rebake).
pub fn gradient_texture_data(gradient: &Gradient) -> Vec<u8> {
    let mut data = Vec::with_capacity((TEXTURE_WIDTH * 4) as usize);

    for i in 0..TEXTURE_WIDTH {
        let t = if TEXTURE_WIDTH > 1 {
            i as f32 / (TEXTURE_WIDTH - 1) as f32
        } else {
            0.0
        };
        let color = sample_gradient(gradient, t);
        data.push((color[0] * 255.0).clamp(0.0, 255.0) as u8);
        data.push((color[1] * 255.0).clamp(0.0, 255.0) as u8);
        data.push((color[2] * 255.0).clamp(0.0, 255.0) as u8);
        data.push((color[3] * 255.0).clamp(0.0, 255.0) as u8);
    }

    data
}

fn bake_gradient_texture(gradient: &Gradient) -> Image {
    create_1d_texture(
        gradient_texture_data(gradient),
        TextureFormat::Rgba8UnormSrgb,
    )
}

/// Overwrites an existing gradient image's pixels in place (keeps the same handle).
pub fn rebake_gradient_image(image: &mut Image, gradient: &Gradient) {
    image.data = Some(gradient_texture_data(gradient));
}

fn sample_gradient(gradient: &Gradient, t: f32) -> [f32; 4] {
    let stops = &gradient.stops;

    if stops.is_empty() {
        return [1.0, 1.0, 1.0, 1.0];
    }
    if stops.len() == 1 {
        return stops[0].color;
    }

    let t = t.clamp(0.0, 1.0);
    let mut left_idx = 0;
    let mut right_idx = stops.len() - 1;

    for (i, stop) in stops.iter().enumerate() {
        if stop.position <= t {
            left_idx = i;
        }
    }
    for (i, stop) in stops.iter().enumerate() {
        if stop.position >= t {
            right_idx = i;
            break;
        }
    }

    let left = &stops[left_idx];
    let right = &stops[right_idx];

    if left_idx == right_idx {
        return left.color;
    }

    let range = right.position - left.position;
    if range <= 0.0 {
        return left.color;
    }

    let local_t = (t - left.position) / range;

    match gradient.interpolation {
        GradientInterpolation::Steps => left.color,
        GradientInterpolation::Linear => lerp_color(left.color, right.color, local_t),
        GradientInterpolation::Smoothstep => {
            let smooth_t = local_t * local_t * (3.0 - 2.0 * local_t);
            lerp_color(left.color, right.color, smooth_t)
        }
    }
}

fn lerp_color(a: [f32; 4], b: [f32; 4], t: f32) -> [f32; 4] {
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
        a[3] + (b[3] - a[3]) * t,
    ]
}

/// A 1x1 white fallback texture used when no gradient texture is available.
#[derive(Resource, Clone, ExtractResource)]
pub struct FallbackGradientTexture {
    /// Handle to the fallback image.
    pub handle: Handle<Image>,
}

/// Bakes gradient textures for all active particle systems.
pub fn prepare_gradient_textures(
    mut cache: ResMut<GradientTextureCache>,
    mut images: ResMut<Assets<Image>>,
    particle_systems: Query<&Particles3d>,
    assets: Res<Assets<ParticlesAsset>>,
) {
    for system in &particle_systems {
        let Some(asset) = assets.get(system) else {
            continue;
        };
        for emitter in &asset.emitters {
            if let SolidOrGradientColor::Gradient { gradient } = &emitter.colors.initial_color {
                cache.get_or_create(gradient, &mut images);
            }
            cache.get_or_create(&emitter.colors.color_over_lifetime, &mut images);
        }
    }
}

/// Cache for baked curve textures, avoiding redundant texture creation.
///
/// Each unique curve (identified by its [`CurveTexture::cache_key`]) is baked into
/// a 1D grayscale texture once and reused across all emitters that reference it.
#[derive(Resource, Default)]
pub struct CurveTextureCache {
    cache: HashMap<u64, Handle<Image>>,
}

impl CurveTextureCache {
    /// Returns a cached texture handle for the curve, creating and baking a new
    /// texture if one doesn't already exist.
    pub fn get_or_create(
        &mut self,
        curve: &CurveTexture,
        images: &mut Assets<Image>,
    ) -> Handle<Image> {
        let key = curve.cache_key();
        if let Some(handle) = self.cache.get(&key) {
            return handle.clone();
        }
        let image = bake_curve_texture(curve);
        let handle = images.add(image);
        self.cache.insert(key, handle.clone());
        handle
    }

    /// Returns the cached texture handle for the curve, if it exists.
    pub fn get(&self, curve: &CurveTexture) -> Option<Handle<Image>> {
        self.cache.get(&curve.cache_key()).cloned()
    }
}

/// Produces the raw 256-wide RGBA8 bytes for a curve (shared by bake + rebake).
pub fn curve_texture_data(curve: &CurveTexture) -> Vec<u8> {
    let mut data = Vec::with_capacity((TEXTURE_WIDTH * 4) as usize);

    for i in 0..TEXTURE_WIDTH {
        let t = if TEXTURE_WIDTH > 1 {
            i as f32 / (TEXTURE_WIDTH - 1) as f32
        } else {
            0.0
        };
        let x = curve.sample(t);
        let y = curve.sample_channel(1, t);
        let z = curve.sample_channel(2, t);
        data.push((x.clamp(0.0, 1.0) * 255.0) as u8); // R
        data.push((y.clamp(0.0, 1.0) * 255.0) as u8); // G
        data.push((z.clamp(0.0, 1.0) * 255.0) as u8); // B
        data.push(255); // A
    }

    data
}

fn bake_curve_texture(curve: &CurveTexture) -> Image {
    create_1d_texture(curve_texture_data(curve), TextureFormat::Rgba8Unorm)
}

/// Overwrites an existing curve image's pixels in place (keeps the same handle).
pub fn rebake_curve_image(image: &mut Image, curve: &CurveTexture) {
    image.data = Some(curve_texture_data(curve));
}

/// A 1x1 white fallback texture used when no curve texture is available.
#[derive(Resource, Clone, ExtractResource)]
pub struct FallbackCurveTexture {
    /// Handle to the fallback image.
    pub handle: Handle<Image>,
}

impl CurveTextureCache {
    fn prepare_optional(&mut self, curve: &Option<CurveTexture>, images: &mut Assets<Image>) {
        if let Some(c) = curve.as_ref().filter(|c| !c.is_constant()) {
            self.get_or_create(c, images);
        }
    }
}

/// Bakes curve textures for all active particle systems.
pub fn prepare_curve_textures(
    mut cache: ResMut<CurveTextureCache>,
    mut images: ResMut<Assets<Image>>,
    particle_systems: Query<&Particles3d>,
    assets: Res<Assets<ParticlesAsset>>,
) {
    for system in &particle_systems {
        let Some(asset) = assets.get(system) else {
            continue;
        };
        for emitter in &asset.emitters {
            cache.prepare_optional(&emitter.scale.scale_over_lifetime, &mut images);
            cache.prepare_optional(&emitter.colors.alpha_over_lifetime, &mut images);
            cache.prepare_optional(&emitter.colors.emission_over_lifetime, &mut images);
            cache.prepare_optional(&emitter.turbulence.influence_over_lifetime, &mut images);
            cache.prepare_optional(&emitter.angle.angle_over_lifetime, &mut images);
            cache.prepare_optional(
                &emitter.velocities.radial_velocity.velocity_over_lifetime,
                &mut images,
            );
            cache.prepare_optional(
                &emitter.velocities.angular_velocity.velocity_over_lifetime,
                &mut images,
            );
            cache.prepare_optional(
                &emitter.velocities.orbit_velocity.velocity_over_lifetime,
                &mut images,
            );
            cache.prepare_optional(
                &emitter
                    .velocities
                    .directional_velocity
                    .velocity_over_lifetime,
                &mut images,
            );
        }
    }
}

/// Bakes one emitter's owned color/size keys into its STABLE per-instance image,
/// re-baking in place only when the key hash changes. Bypasses the global caches,
/// so memory is bounded (one image per instance-with-keys, never accumulating).
///
/// Pure aside from the `images` asset store: no ECS lookups, so it's directly
/// unit-testable without wiring a full `ParticlesAsset`/emitter hierarchy.
pub(crate) fn bake_keys_into(
    baked: &mut OverrideBakedTextures,
    ovr: &ParticleOverride,
    images: &mut Assets<Image>,
) {
    // Color gradient
    match &ovr.color_keys {
        Some(gradient) => {
            let hash = gradient.cache_key();
            if baked.color_hash != Some(hash) {
                // Only advance the stored hash once the pixels are confirmed
                // written, so a failed in-place rebake (handle no longer
                // resolves) doesn't strand stale pixels for a future retry.
                let wrote = match baked.color.clone() {
                    Some(handle) => match images.get_mut(&handle) {
                        Some(mut img) => {
                            rebake_gradient_image(&mut img, gradient);
                            true
                        }
                        None => false,
                    },
                    None => {
                        baked.color = Some(images.add(bake_gradient_texture(gradient)));
                        true
                    }
                };
                if wrote {
                    baked.color_hash = Some(hash);
                }
            }
        }
        None => {
            baked.color = None;
            baked.color_hash = None;
        }
    }

    // Size curve
    match &ovr.size_keys {
        Some(curve) => {
            let hash = curve.cache_key();
            if baked.size_hash != Some(hash) {
                // Same confirmed-write-before-hash-advance guard as above.
                let wrote = match baked.size.clone() {
                    Some(handle) => match images.get_mut(&handle) {
                        Some(mut img) => {
                            rebake_curve_image(&mut img, curve);
                            true
                        }
                        None => false,
                    },
                    None => {
                        baked.size = Some(images.add(bake_curve_texture(curve)));
                        true
                    }
                };
                if wrote {
                    baked.size_hash = Some(hash);
                }
            }
        }
        None => {
            baked.size = None;
            baked.size_hash = None;
        }
    }
}

/// Resolves each emitter entity's effective override (per-emitter map, wins
/// entirely, else the whole-system override) and bakes its owned color/size
/// keys into its own [`OverrideBakedTextures`]. Emitters with no effective
/// override have their baked handles cleared (so a previously-keyed emitter
/// that loses its override reverts to the asset/global-cache texture).
pub(crate) fn bake_override_textures(
    mut images: ResMut<Assets<Image>>,
    particle_systems: Query<&Particles3d>,
    whole_overrides: Query<&ParticleOverride>,
    per_emitter_overrides: Query<&crate::r#override::ParticleEmitterOverrides>,
    assets: Res<Assets<ParticlesAsset>>,
    mut query: Query<(&EmitterEntity, &EmitterRuntime, &mut OverrideBakedTextures)>,
) {
    for (emitter_entity, runtime, mut baked) in query.iter_mut() {
        let Some(emitter_data) = crate::spawning::get_emitter_data(
            emitter_entity.parent_system,
            runtime.emitter_index,
            &particle_systems,
            &assets,
        ) else {
            continue;
        };
        let ovr = crate::r#override::effective_override(
            &emitter_data.name,
            whole_overrides.get(emitter_entity.parent_system).ok(),
            per_emitter_overrides.get(emitter_entity.parent_system).ok(),
        );
        let Some(ovr) = ovr else {
            baked.color = None;
            baked.color_hash = None;
            baked.size = None;
            baked.size_hash = None;
            continue;
        };
        bake_keys_into(&mut baked, ovr, &mut images);
    }
}

fn create_1d_texture(data: Vec<u8>, format: TextureFormat) -> Image {
    let mut image = Image::new(
        Extent3d {
            width: TEXTURE_WIDTH,
            height: 1,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        format,
        default(),
    );
    image.texture_descriptor.usage =
        TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST | TextureUsages::COPY_SRC;
    image
}

fn create_fallback_texture(format: TextureFormat) -> Image {
    let mut image = Image::new(
        Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        vec![255, 255, 255, 255],
        format,
        default(),
    );
    image.texture_descriptor.usage =
        TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST | TextureUsages::COPY_SRC;
    image
}

/// Creates and inserts the [`FallbackGradientTexture`] resource.
pub fn create_fallback_gradient_texture(mut commands: Commands, mut images: ResMut<Assets<Image>>) {
    let handle = images.add(create_fallback_texture(TextureFormat::Rgba8UnormSrgb));
    commands.insert_resource(FallbackGradientTexture { handle });
}

/// Creates and inserts the [`FallbackCurveTexture`] resource.
pub fn create_fallback_curve_texture(mut commands: Commands, mut images: ResMut<Assets<Image>>) {
    let handle = images.add(create_fallback_texture(TextureFormat::Rgba8Unorm));
    commands.insert_resource(FallbackCurveTexture { handle });
}

// `OverrideBakedTextures` now lives on emitter entities and `bake_override_textures`
// resolves each emitter's effective override via `get_emitter_data` +
// `effective_override`, which both need a real `ParticlesAsset` + emitter
// hierarchy to exercise through the full system. Wiring that up in a headless
// test is heavy for what's actually being guaranteed here (the memory bound:
// distinct keys bake once each, unchanged keys never re-allocate, in-place
// rebakes don't grow the pool, non-key overrides never bake). That guarantee
// lives entirely in `bake_keys_into`, which is pure aside from `Assets<Image>`,
// so we unit-test it directly instead of the full ECS system. The per-emitter
// resolution it's fed by (`get_emitter_data` + `effective_override`) is covered
// separately: `effective_override` has its own unit tests in `override.rs`, and
// the emitter/asset wiring is exercised by every other `get_emitter_data`-based
// system's tests (e.g. `sync_particle_buffers`, `write_emitter_uniforms`).
#[cfg(test)]
mod override_bake_tests {
    use super::*;
    use crate::asset::{Gradient, GradientStop};
    use crate::r#override::{OverrideBakedTextures, ParticleOverride};

    fn grad(r: f32) -> Gradient {
        Gradient {
            stops: vec![
                GradientStop {
                    color: [r, 0.0, 0.0, 1.0],
                    position: 0.0,
                },
                GradientStop {
                    color: [1.0, 1.0, 1.0, 1.0],
                    position: 1.0,
                },
            ],
            interpolation: Default::default(),
        }
    }

    fn app() -> App {
        let mut app = App::new();
        app.add_plugins(AssetPlugin::default())
            .init_asset::<Image>();
        app
    }

    fn image_count(app: &App) -> usize {
        app.world().resource::<Assets<Image>>().len()
    }

    #[test]
    fn distinct_keys_bake_one_image_each_then_stay_stable() {
        let mut app = app();
        let mut baked1 = OverrideBakedTextures::default();
        let mut baked2 = OverrideBakedTextures::default();
        let ovr1 = ParticleOverride {
            color_keys: Some(grad(1.0)),
            ..Default::default()
        };
        let ovr2 = ParticleOverride {
            color_keys: Some(grad(0.5)),
            ..Default::default()
        };

        app.world_mut()
            .resource_scope(|_, mut images: Mut<Assets<Image>>| {
                bake_keys_into(&mut baked1, &ovr1, &mut images);
                bake_keys_into(&mut baked2, &ovr2, &mut images);
            });
        assert_eq!(image_count(&app), 2, "two distinct gradients -> two images");

        // Re-running with unchanged keys must NOT create new images.
        app.world_mut()
            .resource_scope(|_, mut images: Mut<Assets<Image>>| {
                bake_keys_into(&mut baked1, &ovr1, &mut images);
                bake_keys_into(&mut baked2, &ovr2, &mut images);
            });
        assert_eq!(image_count(&app), 2, "unchanged keys must not re-allocate");

        // Changing baked1's keys re-bakes IN PLACE: still 2 images total.
        let ovr1_changed = ParticleOverride {
            color_keys: Some(grad(0.25)),
            ..Default::default()
        };
        app.world_mut()
            .resource_scope(|_, mut images: Mut<Assets<Image>>| {
                bake_keys_into(&mut baked1, &ovr1_changed, &mut images);
            });
        assert_eq!(
            image_count(&app),
            2,
            "in-place rebake must not grow the pool"
        );
    }

    #[test]
    fn continuous_non_key_override_never_bakes() {
        let mut app = app();
        let mut baked = OverrideBakedTextures::default();
        let ovr = ParticleOverride {
            tint: Some(LinearRgba::WHITE),
            ..Default::default()
        };

        app.world_mut()
            .resource_scope(|_, mut images: Mut<Assets<Image>>| {
                bake_keys_into(&mut baked, &ovr, &mut images);
                bake_keys_into(&mut baked, &ovr, &mut images);
            });
        assert_eq!(image_count(&app), 0, "no owned keys -> zero baked images");
    }

    #[test]
    fn none_keys_clear_handle_and_hash() {
        let mut app = app();
        let mut baked = OverrideBakedTextures::default();
        let with_keys = ParticleOverride {
            color_keys: Some(grad(1.0)),
            ..Default::default()
        };
        let without_keys = ParticleOverride::default();

        app.world_mut()
            .resource_scope(|_, mut images: Mut<Assets<Image>>| {
                bake_keys_into(&mut baked, &with_keys, &mut images);
            });
        assert!(baked.color.is_some());
        assert!(baked.color_hash.is_some());

        app.world_mut()
            .resource_scope(|_, mut images: Mut<Assets<Image>>| {
                bake_keys_into(&mut baked, &without_keys, &mut images);
            });
        assert!(baked.color.is_none(), "None keys must clear the handle");
        assert!(baked.color_hash.is_none(), "None keys must clear the hash");
        // The image itself isn't removed from the store (no dangling pointer to
        // clean up mid-frame); only the emitter's own reference is dropped.
        assert_eq!(image_count(&app), 1);
    }

    /// Covers the ECS wiring (`get_emitter_data` + `effective_override` +
    /// clear-on-None) that `bake_keys_into` itself doesn't exercise: a parent
    /// with a real `ParticlesAsset` + one child emitter entity, run through the
    /// actual `bake_override_textures` system.
    #[test]
    fn full_system_bakes_per_emitter_and_clears_on_none() {
        use crate::asset::{EmitterData, ParticlesAsset, ParticlesAuthors, ParticlesDimension};
        use crate::runtime::{EmitterEntity, EmitterRuntime, Particles3d};

        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_plugins(AssetPlugin::default())
            .init_asset::<Image>()
            .init_asset::<ParticlesAsset>()
            .add_systems(Update, bake_override_textures);

        let emitter_data = EmitterData {
            name: "Fire".to_string(),
            ..Default::default()
        };
        let particles_asset = ParticlesAsset::new(
            "Test".to_string(),
            ParticlesDimension::D3,
            Default::default(),
            vec![emitter_data],
            Vec::new(),
            false,
            ParticlesAuthors::default(),
        );

        let asset_handle = app
            .world_mut()
            .resource_mut::<Assets<ParticlesAsset>>()
            .add(particles_asset);

        let parent = app
            .world_mut()
            .spawn((
                Particles3d(asset_handle),
                ParticleOverride {
                    color_keys: Some(grad(1.0)),
                    ..Default::default()
                },
            ))
            .id();
        let emitter = app
            .world_mut()
            .spawn((
                EmitterEntity {
                    parent_system: parent,
                },
                EmitterRuntime::new(0, None),
                OverrideBakedTextures::default(),
            ))
            .id();

        app.update();
        let baked = app.world().get::<OverrideBakedTextures>(emitter).unwrap();
        assert!(
            baked.color.is_some(),
            "emitter's owned key resolves through the parent's whole-system override"
        );

        // Drop the parent's override -> the emitter's baked handle must clear.
        app.world_mut()
            .entity_mut(parent)
            .remove::<ParticleOverride>();
        app.update();
        let baked = app.world().get::<OverrideBakedTextures>(emitter).unwrap();
        assert!(
            baked.color.is_none(),
            "no effective override -> baked handle must clear"
        );
    }
}

#[cfg(test)]
mod rebake_tests {
    use super::*;
    use crate::asset::{Gradient, GradientStop};

    #[test]
    fn gradient_data_matches_baked_texture() {
        let g = Gradient {
            stops: vec![
                GradientStop {
                    color: [1.0, 0.0, 0.0, 1.0],
                    position: 0.0,
                },
                GradientStop {
                    color: [0.0, 0.0, 1.0, 1.0],
                    position: 1.0,
                },
            ],
            interpolation: Default::default(),
        };
        let baked = bake_gradient_texture(&g);
        let data = gradient_texture_data(&g);
        assert_eq!(baked.data.as_deref(), Some(data.as_slice()));
    }

    #[test]
    fn rebake_overwrites_in_place() {
        let g1 = Gradient::white();
        let g2 = Gradient {
            stops: vec![
                GradientStop {
                    color: [0.0, 1.0, 0.0, 1.0],
                    position: 0.0,
                },
                GradientStop {
                    color: [0.0, 1.0, 0.0, 1.0],
                    position: 1.0,
                },
            ],
            interpolation: Default::default(),
        };
        let mut img = bake_gradient_texture(&g1);
        rebake_gradient_image(&mut img, &g2);
        assert_eq!(
            img.data.as_deref(),
            Some(gradient_texture_data(&g2).as_slice())
        );
    }
}
