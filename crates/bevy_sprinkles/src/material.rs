use bevy::{
    mesh::MeshVertexBufferLayoutRef,
    pbr::{MaterialExtension, MaterialExtensionKey, MaterialExtensionPipeline, MeshPipelineKey},
    prelude::*,
    render::{
        render_resource::{
            AsBindGroup, CompareFunction, RenderPipelineDescriptor, ShaderType,
            SpecializedMeshPipelineError,
        },
        storage::ShaderBuffer,
    },
    shader::ShaderRef,
};

use crate::asset::{DRIVE_SLOT_COUNT, FxUniform};

const SHADER_ASSET_PATH: &str = "embedded://bevy_sprinkles/shaders/particle_material.wgsl";

/// Number of samples in the baked trail thickness curve LUT.
pub const TRAIL_THICKNESS_CURVE_SAMPLES: usize = 16;

/// GPU-side per-emitter uniforms passed to the particle material shader.
// LAYOUT-LOCKSTEP: fields (order+types) must match ParticleEmitterUniforms in
// shaders/common.wgsl — same GPU buffer.
#[derive(Clone, Copy, ShaderType)]
pub struct ParticleEmitterUniforms {
    /// World-space transform matrix for the emitter.
    pub emitter_transform: Mat4,
    /// Maximum number of particles this emitter can hold.
    pub max_particles: u32,
    /// Particle behavior flags (see [`ParticleFlags`](crate::ParticleFlags)).
    pub particle_flags: u32,
    /// Whether particles are simulated in local coordinates rather than world coordinates.
    pub use_local_coords: u32,
    /// Number of trail segments per particle.
    pub trail_size: u32,
    /// Transform alignment mode for particles.
    ///
    /// - `0`: Disabled
    /// - `1`: Billboard
    /// - `2`: Y to velocity
    /// - `3`: Billboard Y to velocity
    /// - `4`: Billboard fixed Y
    pub transform_align: u32,
    /// Baked trail thickness curve samples.
    pub trail_thickness_curve: [f32; TRAIL_THICKNESS_CURVE_SAMPLES],
    /// Render-stage drive values, indexed by `EmitterProp::slot()`.
    ///
    /// A slot array rather than a named field per property: adding a drivable
    /// property then changes one integer here and one in `common.wgsl`, not
    /// the shape of two structs that must match field for field. `NaN` never
    /// reaches here — `drives::sample` checks finiteness at the boundary.
    ///
    /// Sentinel: a slot no drive touched carries the identity for its consumer
    /// (1.0 for multipliers, which is every current slot), written by
    /// `write_emitter_uniforms`.
    pub drive_slots: [f32; DRIVE_SLOT_COUNT],
}

impl Default for ParticleEmitterUniforms {
    fn default() -> Self {
        Self {
            emitter_transform: Mat4::IDENTITY,
            max_particles: 0,
            particle_flags: 0,
            use_local_coords: 0,
            trail_size: 1,
            transform_align: 0,
            trail_thickness_curve: [1.0; TRAIL_THICKNESS_CURVE_SAMPLES],
            drive_slots: [1.0; DRIVE_SLOT_COUNT],
        }
    }
}

/// A material extension that binds particle data buffers for GPU particle rendering.
///
/// This extension provides the sorted particle buffer and per-emitter uniforms
/// to the vertex shader so it can read per-particle state (position, color,
/// scale, etc.) and transform each instanced mesh accordingly. It also carries
/// the stylized-FX uniform and its textures (see [`crate::asset::FxSettings`]).
#[derive(Asset, AsBindGroup, Reflect, Debug, Clone)]
#[bind_group_data(FxDefs)]
pub struct ParticleMaterialExtension {
    /// Handle to the sorted particle data buffer, read by the vertex shader.
    #[storage(100, read_only)]
    pub sorted_particles: Handle<ShaderBuffer>,
    /// Handle to the per-emitter uniforms buffer (transform, flags, etc.).
    #[storage(101, read_only)]
    pub emitter_uniforms: Handle<ShaderBuffer>,
    /// Stylized-FX parameters. See [`FxUniform`].
    #[uniform(102)]
    pub fx: FxUniform,
    /// Flow-texture handle, sampled to offset the base UV. Only read by the
    /// shader when `FX_FLOW` is compiled in.
    #[texture(103)]
    #[sampler(104)]
    pub flow_texture: Option<Handle<Image>>,
    /// Erosion-noise texture handle. Only read when `FX_EROSION` is compiled in.
    #[texture(105)]
    #[sampler(106)]
    pub erosion_texture: Option<Handle<Image>>,
    /// Baked gradient-remap texture handle. Only read when `FX_GRADIENT` is
    /// compiled in.
    #[texture(107)]
    #[sampler(108)]
    pub gradient_texture: Option<Handle<Image>>,
    /// Which FX shader-def blocks this material's fragment compiles in. Not a
    /// GPU binding -- consumed only by `bind_group_data`/`specialize` to pick
    /// the pipeline variant.
    #[reflect(ignore)]
    pub defs: FxDefs,
}

/// Which FX blocks a [`ParticleMaterialExtension`] wants its shader to
/// compile in. Each `true` flag pushes exactly one shader def in `specialize`
/// below; a feature whose flag is never set costs nothing at draw time
/// because the corresponding `#ifdef` block never compiles at all.
///
/// This is [`ParticleMaterialExtension`]'s [`AsBindGroup::Data`] (via
/// `#[bind_group_data(FxDefs)]`), so pipeline specialization keys on it: each
/// distinct combination of flags compiles once and is cached, rather than
/// recompiling per material instance.
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq, Hash, Reflect)]
pub struct FxDefs {
    /// Pushes `FX_SCROLL`.
    pub scroll: bool,
    /// Pushes `FX_FLOW`.
    pub flow: bool,
    /// Pushes `FX_EROSION`.
    pub erosion: bool,
    /// Pushes `FX_FRESNEL`.
    pub fresnel: bool,
    /// Pushes `FX_SOFT`.
    pub soft: bool,
    /// Pushes `FX_GRADIENT`.
    pub gradient: bool,
    /// Pushes `FX_LIT`. No [`crate::asset::FxSettings`] field drives this
    /// yet -- reserved for a later Phase 3 task, always `false` for now.
    pub lit: bool,
}

impl From<&ParticleMaterialExtension> for FxDefs {
    fn from(ext: &ParticleMaterialExtension) -> Self {
        ext.defs
    }
}

impl MaterialExtension for ParticleMaterialExtension {
    fn vertex_shader() -> ShaderRef {
        SHADER_ASSET_PATH.into()
    }

    fn fragment_shader() -> ShaderRef {
        SHADER_ASSET_PATH.into()
    }

    fn prepass_vertex_shader() -> ShaderRef {
        SHADER_ASSET_PATH.into()
    }

    fn prepass_fragment_shader() -> ShaderRef {
        SHADER_ASSET_PATH.into()
    }

    fn specialize(
        _pipeline: &MaterialExtensionPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        key: MaterialExtensionKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        let is_transparent = key.mesh_key.contains(MeshPipelineKey::BLEND_ALPHA)
            || key
                .mesh_key
                .contains(MeshPipelineKey::BLEND_PREMULTIPLIED_ALPHA)
            || key.mesh_key.contains(MeshPipelineKey::BLEND_MULTIPLY)
            || key
                .mesh_key
                .contains(MeshPipelineKey::BLEND_ALPHA_TO_COVERAGE);

        if let Some(depth_stencil) = &mut descriptor.depth_stencil {
            depth_stencil.depth_write_enabled = Some(!is_transparent);
            depth_stencil.depth_compare = Some(CompareFunction::GreaterEqual);
        }

        // disable backface culling so trail tubes render both sides
        descriptor.primitive.cull_mode = None;

        // Each flag pushes exactly one shader def, in both stages that share
        // this shader source. An effect using none of them compiles the same
        // lean fragment it did before FX existed.
        let defs = &key.bind_group_data;
        let mut push = |on: bool, name: &str| {
            if on {
                descriptor.vertex.shader_defs.push(name.into());
                if let Some(f) = descriptor.fragment.as_mut() {
                    f.shader_defs.push(name.into());
                }
            }
        };
        push(defs.scroll, "FX_SCROLL");
        push(defs.flow, "FX_FLOW");
        push(defs.erosion, "FX_EROSION");
        push(defs.fresnel, "FX_FRESNEL");
        push(defs.soft, "FX_SOFT");
        push(defs.gradient, "FX_GRADIENT");
        push(defs.lit, "FX_LIT");

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The WGSL array length is a literal — nothing links it to Rust's
    /// `DRIVE_SLOT_COUNT`. Because `drive_slots` is the struct's LAST field, a
    /// Rust-side bump without a WGSL-side one would compile clean, test clean,
    /// and then silently never read the new slot. This test is the only thing
    /// standing between that change and a bug with no error message.
    #[test]
    fn the_wgsl_drive_slots_array_matches_the_rust_constant() {
        let src = include_str!("shaders/common.wgsl");
        let decl = src
            .lines()
            .find(|l| l.contains("drive_slots"))
            .expect("common.wgsl must declare drive_slots");
        let want = format!("array<f32, {DRIVE_SLOT_COUNT}>");
        assert!(
            decl.contains(&want),
            "common.wgsl declares `{}` but DRIVE_SLOT_COUNT is {DRIVE_SLOT_COUNT}; \
             these share one GPU buffer and must match",
            decl.trim(),
        );
    }

    /// `EmitterProp::EmissiveIntensity` (slot 3, `DRIVE_SLOT_EMISSIVE`) is
    /// resolved and folded into the uniform by `fold_render_slots` /
    /// `write_emitter_uniforms`, but nothing forces anything to ever read it
    /// back out again. Slots 0-2 were wired when the slot array itself was
    /// introduced and slots 4-8 belong to later FX features; slot 3 fell
    /// through the gap between them. That is the same dead-dial defect that
    /// got `EmitterProp::Drag` deleted -- a slot written but never read is
    /// invisible to every other test in this suite, so this one greps the
    /// shader source directly rather than trusting a runtime effect.
    #[test]
    fn the_emissive_drive_slot_is_actually_read_by_the_fragment_shader() {
        let src = include_str!("shaders/particle_material.wgsl");
        assert!(
            src.contains("drive_slots[DRIVE_SLOT_EMISSIVE]"),
            "particle_material.wgsl must read \
             emitter_uniforms.drive_slots[DRIVE_SLOT_EMISSIVE] somewhere in the \
             fragment shader, or the emissive drive is silently dead",
        );
    }

    /// `ParticleMaterialExtension::flow_texture` is loaded and bound
    /// (`build_extension` in spawning.rs, `#[texture(103)]`/`#[sampler(104)]`
    /// above), but nothing in Rust forces the shader to ever sample it or
    /// read `DRIVE_SLOT_FLOW` back out. A texture binding the shader declares
    /// but never samples is a feature wired to nothing -- the same
    /// dead-dial/dead-slot defect class the emissive test above guards,
    /// which this branch has hit four separate times. This greps the shader
    /// source directly rather than trusting a runtime effect.
    #[test]
    fn the_flow_texture_and_drive_slot_are_actually_read_by_the_fragment_shader() {
        let src = include_str!("shaders/particle_material.wgsl");
        assert!(
            src.contains("textureSample(flow_texture, flow_sampler"),
            "particle_material.wgsl must sample flow_texture through \
             flow_sampler in the fragment shader, or the flow-map binding is \
             wired to nothing",
        );
        assert!(
            src.contains("drive_slots[DRIVE_SLOT_FLOW]"),
            "particle_material.wgsl must read \
             emitter_uniforms.drive_slots[DRIVE_SLOT_FLOW] somewhere in the \
             fragment shader, or the flow drive is silently dead",
        );
    }
}
