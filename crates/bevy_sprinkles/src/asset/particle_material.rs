use bevy::{material::AlphaMode, prelude::*, render::render_resource::Face};
use serde::{Deserialize, Serialize};
use std::hash::{Hash, Hasher};

use super::fx::FxSettings;
use super::serde_helpers::{is_false, is_true, is_zero_f32};
use crate::textures::preset::TextureRef;

/// Sets how a material's base color alpha channel is used for transparency, copied from Bevy's [`AlphaMode`](bevy::material::AlphaMode).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Reflect)]
pub enum SerializableAlphaMode {
    /// Base color alpha values are overridden to be fully opaque (1.0).
    Opaque,
    /// Reduce transparency to fully opaque or fully transparent based on a threshold.
    ///
    /// Compares the base color alpha value to the specified threshold.
    /// If the value is below the threshold, considers the color to be fully transparent
    /// (alpha is set to 0.0). If it is equal to or above the threshold, considers the
    /// color to be fully opaque (alpha is set to 1.0).
    Mask {
        /// The alpha threshold below which pixels are discarded.
        cutoff: f32,
    },
    /// The base color alpha value defines the opacity of the color.
    /// Standard alpha-blending is used to blend the fragment's color
    /// with the color behind it.
    #[default]
    Blend,
    /// Similar to [`AlphaMode::Blend`](bevy::render::alpha::AlphaMode::Blend), however
    /// assumes RGB channel values are premultiplied.
    ///
    /// For otherwise constant RGB values, behaves more like `Blend` for alpha values
    /// closer to 1.0, and more like `Add` for alpha values closer to 0.0.
    ///
    /// Can be used to avoid "border" or "outline" artifacts that can occur when using
    /// plain alpha-blended textures.
    Premultiplied,
    /// Combines the color of the fragments with the colors behind them in an
    /// additive process, (i.e. like light) producing lighter results.
    ///
    /// Black produces no effect. Alpha values can be used to modulate the result.
    ///
    /// Useful for effects like holograms, ghosts, lasers and other energy beams.
    Add,
    /// Combines the color of the fragments with the colors behind them in a
    /// multiplicative process, (i.e. like pigments) producing darker results.
    ///
    /// White produces no effect. Alpha values can be used to modulate the result.
    ///
    /// Useful for effects like stained glass, window tint film and some colored liquids.
    Multiply,
    /// Spreads the fragment out over a hardware-dependent number of sample locations
    /// proportional to the alpha value. This requires multisample antialiasing; if MSAA
    /// isn't on, this is identical to `Mask` with a value of 0.5.
    ///
    /// Alpha to coverage provides improved performance and better visual fidelity over
    /// `Blend`, as Bevy doesn't have to sort objects when it's in use. It's especially
    /// useful for complex transparent objects like foliage.
    AlphaToCoverage,
}

impl SerializableAlphaMode {
    /// Whether the pixels this mode produces change when the particles that
    /// wrote them are drawn in a different order.
    ///
    /// This is the question the depth sort exists to answer, and for most
    /// modes the answer is no — which is why `sort.rs` can skip the whole
    /// bitonic pass for them. Each arm below is a property of the blend
    /// equation the pipeline actually ends up with, so each is derived rather
    /// than asserted. `src` is one particle's fragment, `dst` what is already
    /// in the target, `a` the fragment's alpha.
    ///
    /// **`Blend` — `true`.** `BlendState::ALPHA_BLENDING` is
    /// `src*a + dst*(1-a)` (`bevy_pbr-0.19.0/src/render/mesh.rs:3394`), with
    /// depth writes off. Swapping two fragments changes the result; this is
    /// the classic case the sort is for.
    ///
    /// **`Premultiplied` — `true`.** `PREMULTIPLIED_ALPHA_BLENDING` is
    /// `src + dst*(1-a)` (`mesh.rs:3400`). Still a per-fragment attenuation of
    /// everything underneath it, so still order-dependent.
    ///
    /// **`Add` — `false`.** `Add` shares `Premultiplied`'s pipeline key and
    /// blend state (`bevy_pbr-0.19.0/src/material.rs:616`) and is separated
    /// from it inside the shader: `premultiply_alpha` returns
    /// `vec4(color.rgb * color.a, 0.0)` for the `ADD` flag
    /// (`bevy_pbr-0.19.0/src/render/pbr_functions.wgsl:963`), and the fork's
    /// fragment reaches it through `main_pass_post_lighting_processing`
    /// (`shaders/particle_material.wgsl:1573`). With `src_alpha` forced to
    /// `0`, `src + dst*(1-0)` collapses to `src + dst`. Addition commutes, so
    /// order cannot matter.
    ///
    /// **`Multiply` — `false`.** The blend is
    /// `dst*src + dst*(1-a)` = `dst*(src + 1 - a)` (`mesh.rs:3406-3415`):
    /// every fragment scales what is underneath by its own factor, and a
    /// product of scale factors does not care about their order. Its alpha
    /// channel is `BlendComponent::OVER`, which composes to `1 - Π(1-aᵢ)` —
    /// also independent of order.
    ///
    /// **`Opaque` and `Mask` — `false`.** Both land on the opaque pipeline
    /// with no blending and depth writes on (`mesh.rs:3432-3440`), so the
    /// nearest fragment wins the depth test whenever it arrives.
    ///
    /// **`AlphaToCoverage` — `true`, and this one is a fork-specific
    /// answer.** Stock Bevy leaves depth writes *on* for it (`mesh.rs:3427`)
    /// and its own docs say sorting is unnecessary as a result — but
    /// `ParticleMaterialExtension::specialize` (`material.rs`) counts
    /// `BLEND_ALPHA_TO_COVERAGE` as transparent and forces
    /// `depth_write_enabled = false`. With no depth write, each MSAA sample is
    /// simply taken by whichever covering fragment wrote it last, which is a
    /// draw-order dependence. Sorted here, therefore, until that override
    /// changes.
    pub fn order_dependent(self) -> bool {
        match self {
            Self::Blend | Self::Premultiplied | Self::AlphaToCoverage => true,
            Self::Opaque | Self::Mask { .. } | Self::Add | Self::Multiply => false,
        }
    }
}

impl From<SerializableAlphaMode> for AlphaMode {
    fn from(mode: SerializableAlphaMode) -> Self {
        match mode {
            SerializableAlphaMode::Opaque => AlphaMode::Opaque,
            SerializableAlphaMode::Mask { cutoff } => AlphaMode::Mask(cutoff),
            SerializableAlphaMode::Blend => AlphaMode::Blend,
            SerializableAlphaMode::Premultiplied => AlphaMode::Premultiplied,
            SerializableAlphaMode::Add => AlphaMode::Add,
            SerializableAlphaMode::Multiply => AlphaMode::Multiply,
            SerializableAlphaMode::AlphaToCoverage => AlphaMode::AlphaToCoverage,
        }
    }
}

impl From<AlphaMode> for SerializableAlphaMode {
    fn from(mode: AlphaMode) -> Self {
        match mode {
            AlphaMode::Opaque => SerializableAlphaMode::Opaque,
            AlphaMode::Mask(cutoff) => SerializableAlphaMode::Mask { cutoff },
            AlphaMode::Blend => SerializableAlphaMode::Blend,
            AlphaMode::Premultiplied => SerializableAlphaMode::Premultiplied,
            AlphaMode::Add => SerializableAlphaMode::Add,
            AlphaMode::Multiply => SerializableAlphaMode::Multiply,
            AlphaMode::AlphaToCoverage => SerializableAlphaMode::AlphaToCoverage,
        }
    }
}

/// Serializable face culling mode, copied from wgpu's [`Face`].
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, Reflect)]
pub enum SerializableFace {
    /// Front face.
    Front,
    /// Back face.
    Back,
}

impl From<SerializableFace> for Face {
    fn from(face: SerializableFace) -> Self {
        match face {
            SerializableFace::Front => Face::Front,
            SerializableFace::Back => Face::Back,
        }
    }
}

impl From<Face> for SerializableFace {
    fn from(face: Face) -> Self {
        match face {
            Face::Front => SerializableFace::Front,
            Face::Back => SerializableFace::Back,
        }
    }
}

fn default_base_color() -> [f32; 4] {
    [1.0, 1.0, 1.0, 1.0]
}

fn default_perceptual_roughness() -> f32 {
    0.5
}

fn default_alpha_mode() -> SerializableAlphaMode {
    SerializableAlphaMode::Opaque
}

fn default_reflectance() -> f32 {
    0.5
}

fn default_fog_enabled() -> bool {
    true
}

macro_rules! serde_default {
    ($name:ident, $ty:ty, $val:expr) => {
        ::paste::paste! {
            fn [<default_ $name>]() -> $ty { $val }
            fn [<is_default_ $name>](v: &$ty) -> bool { *v == [<default_ $name>]() }
        }
    };
}

serde_default!(emissive, [f32; 4], [0.0, 0.0, 0.0, 1.0]);
serde_default!(ior, f32, 1.5);
serde_default!(attenuation_distance, f32, f32::INFINITY);
serde_default!(white_color, [f32; 4], default_base_color());
serde_default!(
    cull_mode,
    Option<SerializableFace>,
    Some(SerializableFace::Back)
);
serde_default!(
    clearcoat_perceptual_roughness,
    f32,
    default_perceptual_roughness()
);

fn color_from_array(c: [f32; 4]) -> Color {
    Color::linear_rgba(c[0], c[1], c[2], c[3])
}

fn color_to_array(c: LinearRgba) -> [f32; 4] {
    [c.red, c.green, c.blue, c.alpha]
}

/// A serializable PBR material for particles, copied from Bevy's [`StandardMaterial`](bevy::pbr::StandardMaterial).
#[derive(Debug, Clone, Serialize, Deserialize, Reflect)]
#[reflect(Clone)]
pub struct StandardParticleMaterial {
    /// The color of the surface of the material before lighting.
    ///
    /// Doubles as diffuse albedo for non-metallic, specular for metallic and a mix
    /// for everything in between. If used together with a `base_color_texture`, this
    /// is factored into the final base color as `base_color * base_color_texture_value`.
    ///
    /// Defaults to white `[1.0, 1.0, 1.0, 1.0]`.
    #[serde(default = "default_base_color")]
    pub base_color: [f32; 4],

    /// The actual pre-lighting color is `base_color * this_texture`.
    ///
    /// You should set `base_color` to white (the default) if you want the texture
    /// to show as-is. Setting `base_color` to something else will tint the texture.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_color_texture: Option<TextureRef>,

    /// Color the material "emits" to the camera.
    ///
    /// This is typically used for monitor screens or LED lights. Anything that can
    /// be visible even in darkness.
    ///
    /// The default emissive color is black `[0.0, 0.0, 0.0, 1.0]`, which doesn't
    /// add anything to the material color.
    #[serde(
        default = "default_emissive",
        skip_serializing_if = "is_default_emissive"
    )]
    pub emissive: [f32; 4],

    /// This color is multiplied by `emissive` to get the final emitted color.
    ///
    /// You should set `emissive` to white if you want to use the full range of
    /// color of the emissive texture.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub emissive_texture: Option<TextureRef>,

    /// The weight in which the camera exposure influences the emissive color.
    ///
    /// A value of `0.0` means the emissive color is not affected by the camera
    /// exposure. In opposition, a value of `1.0` means the emissive color is
    /// multiplied by the camera exposure.
    ///
    /// Defaults to `0.0`.
    #[serde(default, skip_serializing_if = "is_zero_f32")]
    pub emissive_exposure_weight: f32,

    /// How to apply the alpha channel of the `base_color_texture`.
    ///
    /// See [`SerializableAlphaMode`] for details. Defaults to
    /// [`SerializableAlphaMode::Opaque`].
    #[serde(default = "default_alpha_mode")]
    pub alpha_mode: SerializableAlphaMode,

    /// Linear perceptual roughness, clamped to `[0.089, 1.0]` in the shader.
    ///
    /// Defaults to `0.5`. Low values result in a "glossy" material with specular
    /// highlights, while values close to `1.0` result in rough materials.
    ///
    /// If used together with a roughness/metallic texture, this is factored into
    /// the final base color as `roughness * roughness_texture_value`.
    #[serde(default = "default_perceptual_roughness")]
    pub perceptual_roughness: f32,

    /// How "metallic" the material appears, within `[0.0, 1.0]`.
    ///
    /// This should be set to `0.0` for dielectric materials or `1.0` for metallic
    /// materials. For a hybrid surface such as corroded metal, you may need to use
    /// in-between values.
    ///
    /// Defaults to `0.0`, for dielectric.
    #[serde(default, skip_serializing_if = "is_zero_f32")]
    pub metallic: f32,

    /// Specular intensity for non-metals on a linear scale of `[0.0, 1.0]`.
    ///
    /// Use the value as a way to control the intensity of the specular highlight
    /// of the material, i.e. how reflective the material is, rather than the
    /// physical property "reflectance."
    ///
    /// Defaults to `0.5` which is mapped to 4% reflectance in the shader.
    #[serde(default = "default_reflectance")]
    pub reflectance: f32,

    /// The blue channel contains metallic values, and the green channel contains
    /// the roughness values. Other channels are unused.
    ///
    /// Those values are multiplied by the scalar ones of the material, see
    /// [`metallic`](Self::metallic) and [`perceptual_roughness`](Self::perceptual_roughness)
    /// for details.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metallic_roughness_texture: Option<TextureRef>,

    /// A normal map texture for faking surface detail.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub normal_map_texture: Option<TextureRef>,

    /// Normal map textures authored for DirectX have their y-component flipped.
    /// Set this to flip it to right-handed conventions.
    #[serde(default, skip_serializing_if = "is_false")]
    pub flip_normal_map_y: bool,

    /// Specifies the level of exposure to ambient light.
    ///
    /// This is usually generated and stored automatically ("baked") by 3D-modeling
    /// software.
    ///
    /// Typically, steep concave parts of a model (such as the armpit of a shirt)
    /// are darker, because they have little exposure to light. An occlusion map
    /// specifies those parts of the model that light doesn't reach well.
    ///
    /// The material will be less lit in places where this texture is dark. This is
    /// similar to ambient occlusion, but built into the model.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub occlusion_texture: Option<TextureRef>,

    /// A color with which to modulate the [`reflectance`](Self::reflectance) for non-metals.
    ///
    /// The specular highlights and reflection are tinted with this color. Note
    /// that it has no effect for non-metals.
    ///
    /// Defaults to white `[1.0, 1.0, 1.0, 1.0]`.
    #[serde(
        default = "default_white_color",
        skip_serializing_if = "is_default_white_color"
    )]
    pub specular_tint: [f32; 4],

    /// The amount of light transmitted diffusely through the material (i.e. "translucency").
    ///
    /// Implemented as a second, flipped Lambertian diffuse lobe, which provides
    /// an inexpensive but plausible approximation of translucency for thin
    /// dielectric objects (e.g. paper, leaves, some fabrics) or thicker volumetric
    /// materials with short scattering distances (e.g. porcelain, wax).
    ///
    /// - When set to `0.0` (the default) no diffuse light is transmitted;
    /// - When set to `1.0` all diffuse light is transmitted through the material;
    /// - Values higher than `0.5` will cause more diffuse light to be transmitted
    ///   than reflected, resulting in a "darker" appearance on the side facing the
    ///   light than the opposite side. (e.g. plant leaves)
    #[serde(default, skip_serializing_if = "is_zero_f32")]
    pub diffuse_transmission: f32,

    /// The amount of light transmitted specularly through the material (i.e. via refraction).
    ///
    /// - When set to `0.0` (the default) no light is transmitted.
    /// - When set to `1.0` all light is transmitted through the material.
    ///
    /// The material's [`base_color`](Self::base_color) also modulates the
    /// transmitted light.
    #[serde(default, skip_serializing_if = "is_zero_f32")]
    pub specular_transmission: f32,

    /// Thickness of the volume beneath the material surface.
    ///
    /// When set to `0.0` (the default) the material appears as an infinitely-thin
    /// film, transmitting light without distorting it.
    ///
    /// When set to any other value, the material distorts light like a thick lens.
    ///
    /// Typically used in conjunction with [`specular_transmission`](Self::specular_transmission)
    /// and [`ior`](Self::ior), or with [`diffuse_transmission`](Self::diffuse_transmission).
    #[serde(default, skip_serializing_if = "is_zero_f32")]
    pub thickness: f32,

    /// The index of refraction of the material.
    ///
    /// Defaults to `1.5`.
    #[serde(default = "default_ior", skip_serializing_if = "is_default_ior")]
    pub ior: f32,

    /// How far, on average, light travels through the volume beneath the material's
    /// surface before being absorbed.
    ///
    /// Defaults to [`f32::INFINITY`], i.e. light is never absorbed.
    #[serde(
        default = "default_attenuation_distance",
        skip_serializing_if = "is_default_attenuation_distance"
    )]
    pub attenuation_distance: f32,

    /// The resulting (non-absorbed) color after white light travels through the
    /// attenuation distance.
    ///
    /// Defaults to white `[1.0, 1.0, 1.0, 1.0]`, i.e. no change.
    #[serde(
        default = "default_white_color",
        skip_serializing_if = "is_default_white_color"
    )]
    pub attenuation_color: [f32; 4],

    /// An extra thin translucent layer on top of the main PBR layer.
    ///
    /// This is typically used for painted surfaces. This value specifies the
    /// strength of the layer, which affects how visible the clearcoat layer
    /// will be.
    ///
    /// Defaults to `0.0`, specifying no clearcoat layer.
    #[serde(default, skip_serializing_if = "is_zero_f32")]
    pub clearcoat: f32,

    /// The roughness of the clearcoat material.
    ///
    /// This is specified in exactly the same way as
    /// [`perceptual_roughness`](Self::perceptual_roughness). If the
    /// [`clearcoat`](Self::clearcoat) value is zero, this has no effect.
    ///
    /// Defaults to `0.5`.
    #[serde(
        default = "default_clearcoat_perceptual_roughness",
        skip_serializing_if = "is_default_clearcoat_perceptual_roughness"
    )]
    pub clearcoat_perceptual_roughness: f32,

    /// Increases the roughness along a specific direction, so that the specular
    /// highlight will be stretched instead of being a circular lobe.
    ///
    /// This value ranges from `0.0` (perfectly circular) to `1.0` (maximally
    /// stretched).
    ///
    /// This is typically used for modeling surfaces such as brushed metal and
    /// hair, in which one direction of the surface but not the other is smooth.
    #[serde(default, skip_serializing_if = "is_zero_f32")]
    pub anisotropy_strength: f32,

    /// The direction of increased roughness, in radians relative to the mesh tangent.
    ///
    /// This parameter has no effect if [`anisotropy_strength`](Self::anisotropy_strength)
    /// is zero.
    #[serde(default, skip_serializing_if = "is_zero_f32")]
    pub anisotropy_rotation: f32,

    /// Support two-sided lighting by automatically flipping the normals for
    /// "back" faces within the PBR lighting shader.
    ///
    /// Defaults to `false`.
    #[serde(default, skip_serializing_if = "is_false")]
    pub double_sided: bool,

    /// Whether to cull the "front", "back" or neither side of a mesh.
    ///
    /// If set to `None`, the two sides of the mesh are visible.
    ///
    /// Defaults to `Some(Back)`.
    #[serde(
        default = "default_cull_mode",
        skip_serializing_if = "is_default_cull_mode"
    )]
    pub cull_mode: Option<SerializableFace>,

    /// Whether to apply only the base color to this material.
    ///
    /// Normals, occlusion textures, roughness, metallic, reflectance, emissive,
    /// shadows, alpha mode and ambient light are ignored if this is set to `true`.
    ///
    /// Forwarded to [`StandardMaterial::unlit`] by `to_standard_material`
    /// below, which is what actually makes this work: the particle
    /// fragment shader reads it back out at runtime
    /// (`STANDARD_MATERIAL_FLAGS_UNLIT_BIT` in
    /// `shaders/particle_material.wgsl`'s forward fragment) rather than
    /// through a compile-time shader def -- see the comment beside that
    /// read for why a def is unnecessary here, in a shader that does not
    /// share bevy's own `pbr.wgsl` unlit-gating pitfall.
    #[serde(default, skip_serializing_if = "is_false")]
    pub unlit: bool,

    /// Whether to enable fog for this material.
    ///
    /// Defaults to `true`.
    #[serde(default = "default_fog_enabled", skip_serializing_if = "is_true")]
    pub fog_enabled: bool,

    /// Adjust rendered depth.
    ///
    /// A material with a positive depth bias will render closer to the camera
    /// while negative values cause the material to render behind other objects.
    /// This is independent of the viewport.
    ///
    /// Defaults to `0.0`.
    #[serde(default, skip_serializing_if = "is_zero_f32")]
    pub depth_bias: f32,

    /// Stylized-FX settings. See [`FxSettings`]; defaults to entirely inert.
    #[serde(default, skip_serializing_if = "is_default_fx")]
    pub fx: FxSettings,
}

fn is_default_fx(f: &FxSettings) -> bool {
    *f == FxSettings::default()
}

impl Default for StandardParticleMaterial {
    fn default() -> Self {
        Self {
            base_color: default_base_color(),
            base_color_texture: None,
            emissive: [0.0, 0.0, 0.0, 1.0],
            emissive_texture: None,
            emissive_exposure_weight: 0.0,
            alpha_mode: default_alpha_mode(),
            perceptual_roughness: default_perceptual_roughness(),
            metallic: 0.0,
            reflectance: default_reflectance(),
            metallic_roughness_texture: None,
            normal_map_texture: None,
            flip_normal_map_y: false,
            occlusion_texture: None,
            specular_tint: default_white_color(),
            diffuse_transmission: 0.0,
            specular_transmission: 0.0,
            thickness: 0.0,
            ior: default_ior(),
            attenuation_distance: default_attenuation_distance(),
            attenuation_color: default_white_color(),
            clearcoat: 0.0,
            clearcoat_perceptual_roughness: default_clearcoat_perceptual_roughness(),
            anisotropy_strength: 0.0,
            anisotropy_rotation: 0.0,
            double_sided: false,
            cull_mode: default_cull_mode(),
            unlit: false,
            fog_enabled: true,
            depth_bias: 0.0,
            fx: FxSettings::default(),
        }
    }
}

impl StandardParticleMaterial {
    /// Converts this serializable material into a Bevy [`StandardMaterial`],
    /// loading any referenced textures via the provided [`AssetServer`].
    pub fn to_standard_material(
        &self,
        asset_server: &AssetServer,
        assets_folders: &[String],
    ) -> StandardMaterial {
        let load_tex =
            |tex: &Option<TextureRef>| tex.as_ref().map(|t| t.load(asset_server, assets_folders));

        StandardMaterial {
            base_color: color_from_array(self.base_color),
            base_color_texture: load_tex(&self.base_color_texture),
            emissive: color_from_array(self.emissive).into(),
            emissive_texture: load_tex(&self.emissive_texture),
            emissive_exposure_weight: self.emissive_exposure_weight,
            alpha_mode: self.alpha_mode.into(),
            perceptual_roughness: self.perceptual_roughness,
            metallic: self.metallic,
            reflectance: self.reflectance,
            metallic_roughness_texture: load_tex(&self.metallic_roughness_texture),
            normal_map_texture: load_tex(&self.normal_map_texture),
            flip_normal_map_y: self.flip_normal_map_y,
            occlusion_texture: load_tex(&self.occlusion_texture),
            specular_tint: color_from_array(self.specular_tint),
            diffuse_transmission: self.diffuse_transmission,
            specular_transmission: self.specular_transmission,
            thickness: self.thickness,
            ior: self.ior,
            attenuation_distance: self.attenuation_distance,
            attenuation_color: color_from_array(self.attenuation_color),
            clearcoat: self.clearcoat,
            clearcoat_perceptual_roughness: self.clearcoat_perceptual_roughness,
            anisotropy_strength: self.anisotropy_strength,
            anisotropy_rotation: self.anisotropy_rotation,
            double_sided: self.double_sided,
            cull_mode: self.cull_mode.map(|f| f.into()),
            unlit: self.unlit,
            fog_enabled: self.fog_enabled,
            depth_bias: self.depth_bias,
            ..default()
        }
    }

    /// Creates a [`StandardParticleMaterial`] from a Bevy [`StandardMaterial`].
    ///
    /// Texture references are not preserved. Only color and numeric properties are copied.
    pub fn from_standard_material(material: &StandardMaterial) -> Self {
        Self {
            base_color: color_to_array(material.base_color.to_linear()),
            base_color_texture: None,
            emissive: color_to_array(material.emissive),
            emissive_texture: None,
            emissive_exposure_weight: material.emissive_exposure_weight,
            alpha_mode: material.alpha_mode.into(),
            perceptual_roughness: material.perceptual_roughness,
            metallic: material.metallic,
            reflectance: material.reflectance,
            metallic_roughness_texture: None,
            normal_map_texture: None,
            flip_normal_map_y: material.flip_normal_map_y,
            occlusion_texture: None,
            specular_tint: color_to_array(material.specular_tint.to_linear()),
            diffuse_transmission: material.diffuse_transmission,
            specular_transmission: material.specular_transmission,
            thickness: material.thickness,
            ior: material.ior,
            attenuation_distance: material.attenuation_distance,
            attenuation_color: color_to_array(material.attenuation_color.to_linear()),
            clearcoat: material.clearcoat,
            clearcoat_perceptual_roughness: material.clearcoat_perceptual_roughness,
            anisotropy_strength: material.anisotropy_strength,
            anisotropy_rotation: material.anisotropy_rotation,
            double_sided: material.double_sided,
            cull_mode: material.cull_mode.map(|f| f.into()),
            unlit: material.unlit,
            fog_enabled: material.fog_enabled,
            depth_bias: material.depth_bias,
            fx: FxSettings::default(),
        }
    }

    /// Computes a hash key for material caching.
    pub fn cache_key(&self) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        let hash_f32 = |h: &mut std::collections::hash_map::DefaultHasher, v: f32| {
            v.to_bits().hash(h);
        };
        let hash_color = |h: &mut std::collections::hash_map::DefaultHasher, c: &[f32; 4]| {
            for v in c {
                v.to_bits().hash(h);
            }
        };

        hash_color(&mut hasher, &self.base_color);
        self.base_color_texture.hash(&mut hasher);
        hash_color(&mut hasher, &self.emissive);
        self.emissive_texture.hash(&mut hasher);
        hash_f32(&mut hasher, self.emissive_exposure_weight);
        std::mem::discriminant(&self.alpha_mode).hash(&mut hasher);
        if let SerializableAlphaMode::Mask { cutoff } = self.alpha_mode {
            cutoff.to_bits().hash(&mut hasher);
        }
        hash_f32(&mut hasher, self.perceptual_roughness);
        hash_f32(&mut hasher, self.metallic);
        hash_f32(&mut hasher, self.reflectance);
        self.metallic_roughness_texture.hash(&mut hasher);
        self.normal_map_texture.hash(&mut hasher);
        self.flip_normal_map_y.hash(&mut hasher);
        self.occlusion_texture.hash(&mut hasher);
        hash_color(&mut hasher, &self.specular_tint);
        hash_f32(&mut hasher, self.diffuse_transmission);
        hash_f32(&mut hasher, self.specular_transmission);
        hash_f32(&mut hasher, self.thickness);
        hash_f32(&mut hasher, self.ior);
        hash_f32(&mut hasher, self.attenuation_distance);
        hash_color(&mut hasher, &self.attenuation_color);
        hash_f32(&mut hasher, self.clearcoat);
        hash_f32(&mut hasher, self.clearcoat_perceptual_roughness);
        hash_f32(&mut hasher, self.anisotropy_strength);
        hash_f32(&mut hasher, self.anisotropy_rotation);
        self.double_sided.hash(&mut hasher);
        self.cull_mode.hash(&mut hasher);
        self.unlit.hash(&mut hasher);
        self.fog_enabled.hash(&mut hasher);
        hash_f32(&mut hasher, self.depth_bias);
        self.fx.cache_key().hash(&mut hasher);
        hasher.finish()
    }
}

/// The material used for a draw pass, either a standard PBR material or custom shaders.
#[derive(Debug, Clone, Serialize, Deserialize, Reflect)]
pub enum DrawPassMaterial {
    /// A standard PBR material for particles.
    Standard(StandardParticleMaterial),
    /// Custom vertex and/or fragment shaders.
    CustomShader {
        /// Optional path to a custom vertex shader.
        vertex_shader: Option<String>,
        /// Optional path to a custom fragment shader.
        fragment_shader: Option<String>,
    },
}

impl Default for DrawPassMaterial {
    fn default() -> Self {
        Self::Standard(StandardParticleMaterial::default())
    }
}

impl DrawPassMaterial {
    /// Whether this draw pass needs its particles sorted before they are drawn.
    ///
    /// See [`SerializableAlphaMode::order_dependent`] for the per-mode
    /// derivation. A [`DrawPassMaterial::CustomShader`] answers `true`
    /// unconditionally: the fork has no idea what blend state somebody else's
    /// fragment shader ends up with, and the cost of being wrong in that
    /// direction is a sort nobody needed, whereas the cost of being wrong the
    /// other way is a visibly scrambled effect.
    pub fn needs_sorting(&self) -> bool {
        match self {
            Self::Standard(mat) => mat.alpha_mode.order_dependent(),
            Self::CustomShader { .. } => true,
        }
    }

    /// Computes a hash key for material caching.
    pub fn cache_key(&self) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        match self {
            Self::Standard(mat) => {
                0u8.hash(&mut hasher);
                mat.cache_key().hash(&mut hasher);
            }
            Self::CustomShader {
                vertex_shader,
                fragment_shader,
            } => {
                1u8.hash(&mut hasher);
                vertex_shader.hash(&mut hasher);
                fragment_shader.hash(&mut hasher);
            }
        }
        hasher.finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::app::ScheduleRunnerPlugin;
    use bevy::asset::AssetPlugin;
    use bevy::prelude::{App, MinimalPlugins};
    use std::time::Duration;

    /// A minimal `App` good enough to hand `to_standard_material` a real
    /// `AssetServer` -- neither material built below sets a texture, so
    /// `load_tex`'s `Option::map` never actually calls `asset_server.load`,
    /// and no fixture files need to exist on disk for this test.
    fn test_app() -> App {
        let mut app = App::new();
        app.add_plugins(
            MinimalPlugins.set(ScheduleRunnerPlugin::run_loop(Duration::from_millis(10))),
        );
        app.add_plugins(AssetPlugin::default());
        app
    }

    /// `unlit` is the one field the particle fragment shader reads back at
    /// runtime (`STANDARD_MATERIAL_FLAGS_UNLIT_BIT` in
    /// `shaders/particle_material.wgsl`'s forward fragment) to decide
    /// whether to light a particle at all. That only works if the authored
    /// flag actually survives the trip through `to_standard_material` into
    /// the `StandardMaterial` bevy's PBR pipeline sets the flag bit from --
    /// nothing else pins the `unlit: self.unlit` forward line above, and a
    /// silently dropped forward would look, in play, exactly like the
    /// dead-shader-def bug this task removed (`FxDefs.lit`/`FX_LIT`): a
    /// setting that appears to exist and does nothing.
    #[test]
    fn to_standard_material_forwards_the_unlit_flag_in_both_directions() {
        let app = test_app();
        let asset_server = app.world().resource::<AssetServer>();

        let lit = StandardParticleMaterial {
            unlit: false,
            ..Default::default()
        };
        let unlit = StandardParticleMaterial {
            unlit: true,
            ..Default::default()
        };

        assert!(!lit.to_standard_material(asset_server, &[]).unlit);
        assert!(unlit.to_standard_material(asset_server, &[]).unlit);
    }

    /// Task 22's material-rebuild question, pinned at the exact seam
    /// `sync_particle_material` (`spawning.rs`) reads: it compares
    /// `StandardParticleMaterial::cache_key()`, not `FxSettings::cache_key()`
    /// directly, every frame, for every emitter, unconditionally (no
    /// `Changed<>` filter -- it re-derives `new_material` from the asset
    /// fresh each tick and only SKIPS the rebuild when the two keys already
    /// match). `FxSettings`'s own tests already pin that ITS `cache_key`
    /// changes with a field; this test pins that the fold at line ~628
    /// (`self.fx.cache_key().hash(&mut hasher)`) actually carries that
    /// change up into the key `sync_particle_material` looks at, so an
    /// fx-only edit is never mistaken for "nothing changed" the way Task
    /// 21's light bug mistook a spawn-only read for a live one.
    #[test]
    fn an_fx_only_difference_changes_the_materials_own_cache_key() {
        let a = StandardParticleMaterial::default();
        let b = StandardParticleMaterial {
            fx: FxSettings {
                scroll: bevy::math::Vec2::new(0.0, 0.4),
                ..Default::default()
            },
            ..Default::default()
        };
        assert_ne!(a.cache_key(), b.cache_key());
    }
}
