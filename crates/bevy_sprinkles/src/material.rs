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
    /// [`EmitterDrawPass::max_screen_size`](crate::asset::EmitterDrawPass::max_screen_size):
    /// the largest fraction of the view's height one camera-facing particle
    /// may span. `0.0` is off. LAYOUT-LOCKSTEP with `common.wgsl`.
    pub max_screen_size: f32,
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
            max_screen_size: 0.0,
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

    /// `FxUniform`'s Rust and WGSL declarations share one GPU buffer, and its
    /// four `vec4`s carry unrelated quantities packed by POSITION -- `scroll`
    /// in `scroll_tiling.xy`, `fresnel_power` in `flow_fresnel.w`,
    /// `fresnel_boost` in `erosion_soft.z`. Reorder or rename one side and
    /// every lane after it reads someone else's number: no compile error, no
    /// validation error, just an effect whose fresnel is driven by its soft
    /// fade. Both declarations carried a "LAYOUT-LOCKSTEP" comment and nothing
    /// checked it -- the only one of the three struct pairs here in that state.
    ///
    /// The Rust side comes from `#[derive(Reflect)]`'s own field metadata,
    /// generated from the struct definition, so this compares WGSL against the
    /// struct ITSELF rather than against a second hand-written list that could
    /// drift alongside it. Same mechanism as
    /// `asset::drive::tests::emitter_prop_all_matches_the_reflected_enum_exactly`.
    #[test]
    fn the_wgsl_fx_uniform_declares_the_same_fields_in_the_same_order() {
        let bevy::reflect::TypeInfo::Struct(info) =
            <FxUniform as bevy::reflect::Typed>::type_info()
        else {
            panic!("FxUniform must be a reflected struct");
        };
        let rust: Vec<&str> = (0..info.field_len())
            .filter_map(|i| info.field_at(i))
            .map(|f| f.name())
            .collect();

        let src = include_str!("shaders/common.wgsl");
        let body = src
            .split_once("struct FxUniform {")
            .expect("common.wgsl must declare struct FxUniform")
            .1
            .split_once('}')
            .expect("the FxUniform declaration must be closed")
            .0;
        let wgsl: Vec<(&str, &str)> = body
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with("//"))
            .map(|l| {
                let l = l.trim_end_matches(',');
                let (name, ty) = l.split_once(':').expect("a WGSL field is `name: type`");
                (name.trim(), ty.trim())
            })
            .collect();

        let wgsl_names: Vec<&str> = wgsl.iter().map(|(n, _)| *n).collect();
        assert_eq!(
            wgsl_names, rust,
            "common.wgsl's FxUniform must declare exactly the Rust struct's \
             fields, in the same ORDER -- they are the same GPU buffer, and \
             every field is a vec4, so a reorder mismatches silently"
        );
        for (name, ty) in wgsl {
            assert_eq!(
                ty, "vec4<f32>",
                "FxUniform packs everything into vec4s so the WGSL struct needs \
                 no padding fields; `{name}` is declared `{ty}`, and a non-vec4 \
                 field here changes the layout on one side only"
            );
        }
    }

    /// `particle_material.wgsl` has two fragment functions -- the deferred-
    /// prepass fragment and the forward fragment -- which must stay
    /// byte-identical wherever a drive or texture read matters: a feature
    /// living in only one path works in some render configurations (e.g.
    /// normal-prepass-enabled vs. not) and silently not in others, which is
    /// a nasty bug to chase with no compiler or naga check to catch it.
    /// `str::contains` is satisfied by a single occurrence, so it stays
    /// green even if a future edit deletes one of the two blocks -- asserting
    /// an exact count of 2 is what actually proves both paths still read it.
    fn assert_occurs_in_both_fragments(src: &str, needle: &str, defect: &str) {
        let count = src.matches(needle).count();
        assert_eq!(
            count, 2,
            "expected `{needle}` to appear exactly twice in \
             particle_material.wgsl (once in the deferred-prepass fragment, \
             once in the forward fragment) -- {defect}. Found {count} \
             occurrence(s): 0 means neither fragment reads it, 1 means only \
             one of the two fragment functions does, anything above 2 means \
             a third fragment path appeared and also needs covering.",
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
        assert_occurs_in_both_fragments(
            src,
            "drive_slots[DRIVE_SLOT_EMISSIVE]",
            "the emissive drive is silently dead in whichever fragment lost it",
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
        assert_occurs_in_both_fragments(
            src,
            "textureSample(flow_texture, flow_sampler",
            "the flow-map binding is wired to nothing in whichever fragment lost it",
        );
        assert_occurs_in_both_fragments(
            src,
            "drive_slots[DRIVE_SLOT_FLOW]",
            "the flow drive is silently dead in whichever fragment lost it",
        );
    }

    /// `ParticleMaterialExtension::erosion_texture` is loaded and bound
    /// (`build_extension` in spawning.rs, `#[texture(105)]`/`#[sampler(106)]`
    /// above), but nothing in Rust forces the shader to ever sample it,
    /// discard on `DRIVE_SLOT_EROSION`, or paint the rim color back out. A
    /// texture binding the shader declares but never samples -- or a discard
    /// that lives in only one of the two fragments -- is the same
    /// dead-dial/dead-slot defect class the flow and emissive tests above
    /// guard. This greps the shader source directly rather than trusting a
    /// runtime effect.
    #[test]
    fn the_erosion_texture_and_drive_slot_are_actually_read_by_the_fragment_shader() {
        let src = include_str!("shaders/particle_material.wgsl");
        assert_occurs_in_both_fragments(
            src,
            "textureSample(erosion_texture, erosion_sampler",
            "the erosion-noise binding is wired to nothing in whichever fragment lost it",
        );
        assert_occurs_in_both_fragments(
            src,
            "drive_slots[DRIVE_SLOT_EROSION]",
            "the erosion drive is silently dead in whichever fragment lost it",
        );
        assert_occurs_in_both_fragments(
            src,
            "fx.erosion_edge_color.a",
            "the erosion rim color is silently dead in whichever fragment lost it",
        );
    }

    /// `DRIVE_SLOT_FRESNEL` is resolved into `emitter_uniforms.drive_slots`
    /// (`asset/drive.rs`) but nothing forces the shader to ever read it back.
    /// Fresnel follows the established both-fragments shape (deferred-prepass
    /// and forward) exactly like scroll/flow/erosion, so it gets the same
    /// exactly-2 guard as those.
    #[test]
    fn the_fresnel_drive_slot_is_actually_read_by_the_fragment_shader() {
        let src = include_str!("shaders/particle_material.wgsl");
        assert_occurs_in_both_fragments(
            src,
            "drive_slots[DRIVE_SLOT_FRESNEL]",
            "the fresnel drive is silently dead in whichever fragment lost it",
        );
    }

    /// Unlike every other FX block, soft particles deliberately do NOT follow
    /// the both-fragments shape: `prepass_depth` reads the depth texture the
    /// depth prepass *produces*, so it can only be sampled coherently from a
    /// fragment that runs in a LATER pass than the one writing that texture.
    /// Of this file's three fragment functions, only the forward fragment
    /// (`#ifndef PREPASS_PIPELINE`) qualifies -- the depth-only prepass
    /// fragment and the deferred/normal/motion prepass fragment both execute
    /// DURING the depth prepass itself. `assert_occurs_in_both_fragments`
    /// would be the WRONG guard here: it would demand a second, structurally
    /// invalid copy of this block. This asserts an exact count of 1 instead,
    /// which is the guard that actually matches the constraint -- see the
    /// task-14 report for the vendored bevy_pbr citations this rests on.
    fn assert_occurs_only_in_forward_fragment(src: &str, needle: &str, defect: &str) {
        let count = src.matches(needle).count();
        assert_eq!(
            count, 1,
            "expected `{needle}` to appear exactly once in \
             particle_material.wgsl (in the forward fragment only -- \
             prepass_depth cannot be read coherently from a fragment that \
             executes during the same depth prepass that produces it) -- \
             {defect}. Found {count} occurrence(s): 0 means the forward \
             fragment lost it, anything above 1 means it leaked into a \
             prepass-stage fragment where the depth texture it reads is not \
             yet complete.",
        );
    }

    /// `ParticleMaterialExtension::gradient_texture` is baked and bound
    /// (`build_extension` in spawning.rs, through the same
    /// `GradientTextureCache` the emitter colour-gradient path already uses,
    /// `#[texture(107)]`/`#[sampler(108)]` above), but nothing in Rust forces
    /// the shader to ever sample it. Gradient remap has no prepass-ordering
    /// constraint like soft particles, so it follows the established
    /// both-fragments shape (deferred-prepass and forward) exactly like
    /// scroll/flow/erosion/fresnel, and gets the same exactly-2 guard.
    #[test]
    fn the_gradient_texture_is_actually_read_by_the_fragment_shader() {
        let src = include_str!("shaders/particle_material.wgsl");
        assert_occurs_in_both_fragments(
            src,
            "textureSample(gradient_texture, gradient_sampler",
            "the gradient-remap binding is wired to nothing in whichever fragment lost it",
        );
    }

    #[test]
    fn soft_particle_fade_is_read_only_by_the_forward_fragment() {
        let src = include_str!("shaders/particle_material.wgsl");
        assert_occurs_only_in_forward_fragment(
            src,
            "prepass_depth(in.position, 0u)",
            "the soft-particle depth sample is silently dead, or leaked into a prepass fragment",
        );
        assert_occurs_only_in_forward_fragment(
            src,
            "fx.erosion_soft.w",
            "the soft-fade distance drive is silently dead, or leaked into a prepass fragment",
        );
    }

    /// Lit particles are a runtime branch, not a shader def -- there is no
    /// `FX_LIT` any more (removed alongside `FxDefs.lit`, which pushed a def
    /// no `#ifdef` in this file ever read). This is the only thing
    /// protecting that branch: a tidy-up that called `apply_pbr_lighting`
    /// unconditionally -- exactly the bug a sibling project shipped in its
    /// own particle-ish material -- would compile clean and only show up as
    /// "unlit particles are lit anyway" in play.
    ///
    /// Restricted to the forward fragment for the same structural reason as
    /// `soft_particle_fade_is_read_only_by_the_forward_fragment` above,
    /// though the constraint here is different: the deferred-prepass
    /// fragment writes a gbuffer via `deferred_output` and does not light at
    /// all (lighting happens later, from the gbuffer, outside this shader),
    /// and the depth-only prepass fragment only discards -- neither has a
    /// lighting decision to branch on in the first place.
    #[test]
    fn the_unlit_branch_is_read_only_by_the_forward_fragment() {
        let src = include_str!("shaders/particle_material.wgsl");
        assert_occurs_only_in_forward_fragment(
            src,
            "pbr_input.material.flags & STANDARD_MATERIAL_FLAGS_UNLIT_BIT",
            "the unlit runtime branch is silently dead, or leaked into a prepass fragment",
        );
    }
}
