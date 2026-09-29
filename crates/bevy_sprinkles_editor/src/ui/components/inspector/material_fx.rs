//! The material FX inspector section (Task 22): the editing surface for
//! Phase 3's stylized-look material features -- scroll, flow-map churn,
//! erosion dissolve, fresnel, soft particles, gradient remap -- all carried
//! on [`FxSettings`] (`crates/bevy_sprinkles/src/asset/fx.rs`).
//!
//! **Where the fields live drives how they're read/written.** `fx:
//! FxSettings` sits on `StandardParticleMaterial`, which itself is the tuple
//! payload of `DrawPassMaterial::Standard` -- so `fx.scroll` is TWO hops
//! inside an enum (`draw_pass.material` is the enum; `.fx` unwraps its tuple
//! field to a struct; `.scroll` reads a field of THAT struct). Every field
//! here is therefore authored through `FieldBinding::emitter_variant_field`
//! with a base path of `"draw_pass.material"` and a dotted `field_name`
//! (`"fx.scroll"`, `"fx.flow_texture"`, ...) -- the same accessor
//! `sub_emitter.rs`/`draw_pass.rs`'s mask-cutoff row already use for a
//! single hop, widened by Task 22 (see `binding::resolve_variant_field_ref`'s
//! doc) to also cross a plain struct boundary, not just another enum.
//!
//! **No section has a real on/off `bool`.** Unlike `turbulence.enabled`,
//! there is no backing field for "is Scroll on" -- `FxSettings::scroll_enabled`
//! etc. are DERIVED from whether the feature's fields differ from their
//! inert defaults (`fx.rs`'s doc: every default must cost nothing and change
//! nothing, exactly so a struct added to an existing asset never restyles
//! it). So instead of a checkbox, each section gets a "Reset to default"
//! button that calls this module's `clear_*` function for that feature --
//! the one-directional half of "toggling a feature off" the brief asks for
//! (turning one ON is just editing any of its fields, which the individual
//! widgets already do).
//!
//! **The two tooltips are permanent [`AlertVariant::Info`] notes**, not a
//! hover tooltip -- this codebase has no tooltip widget. `alert()` is
//! already the established way to put a fixed explanatory note inside an
//! inspector section (`draw_pass.rs`'s trail-mesh alert, `colors.rs`'s
//! alpha-disabled alert), just used here unconditionally rather than shown
//! only in some states.
//!
//! **The six sections are hidden, not merely inert, when the material isn't
//! `Standard`.** `FxSettings` only exists on `DrawPassMaterial::Standard`;
//! selecting `CustomShader` leaves every `fx.*` chained lookup resolving to
//! `None` (safe -- reads show stale values, writes silently no-op -- but
//! gives an author who edits a field no signal that nothing happened). Task
//! 22's fix round matched this to the existing precedent for exactly this
//! situation: `draw_pass.rs`'s mask-cutoff row is despawned, not left
//! dangling, when its own `let SerializableAlphaMode::Mask { .. } = ..`
//! guard fails. `sync_fx_sections_visibility` does the section-level
//! equivalent with `Display` rather than despawn/respawn (six sections
//! toggling every emitter-selection change would otherwise tear down and
//! rebuild the WHOLE FX surface, including any open drive popovers, purely
//! because the author looked at a different emitter of the same material
//! kind) -- `set_display_visible` already short-circuits a no-op write, so
//! this costs nothing when nothing changed.
use bevy::prelude::*;
use bevy_sprinkles::asset::{EmitterProp, FxSettings};
use bevy_sprinkles::prelude::*;

use crate::state::EditorState;
use crate::ui::components::binding::{EmitterWriter, get_inspecting_emitter};
use crate::ui::widgets::alert::{AlertSpan, AlertVariant, alert};
use crate::ui::widgets::button::{ButtonClickEvent, ButtonProps, button};
use crate::ui::widgets::inspector_field::fields_row;
use crate::ui::widgets::vector_edit::VectorSuffixes;

use super::{InspectorItem, InspectorSection, VariantField};

/// The one base path every FX field is authored through. See the module doc.
const FX_MATERIAL_PATH: &str = "draw_pass.material";

pub fn plugin(app: &mut App) {
    app.add_observer(handle_fx_reset_click).add_systems(
        Update,
        (setup_fx_section_extras, sync_fx_sections_visibility)
            .after(super::update_inspected_emitter_tracker),
    );
}

/// Which FX feature a section is for. Doubles as the section's own marker
/// component (inserted as the `impl Bundle` half of each `*_section()`
/// return, exactly like `TurbulenceSection` etc.) and as the reset button's
/// payload, so `setup_fx_section_extras` needs only ONE `Added<FxFeature>`
/// query to catch all six sections rather than a marker-plus-observer pair
/// per feature.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
enum FxFeature {
    Scroll,
    Flow,
    Erosion,
    Fresnel,
    Soft,
    Gradient,
}

impl FxFeature {
    /// Resets exactly the fields the matching `FxSettings::*_enabled`
    /// predicate reads. Deliberately dispatches to a separate top-level
    /// `clear_*` function per feature rather than inlining the field
    /// writes here: the `tests` module below mutation-verifies each one
    /// against `fx.rs`'s own predicate, and a bug in the dispatch itself
    /// (calling the wrong arm) would show up as an obviously-wrong test
    /// failure rather than a subtle one.
    fn clear(self, fx: &mut FxSettings) {
        match self {
            Self::Scroll => clear_scroll(fx),
            Self::Flow => clear_flow(fx),
            Self::Erosion => clear_erosion(fx),
            Self::Fresnel => clear_fresnel(fx),
            Self::Soft => clear_soft(fx),
            Self::Gradient => clear_gradient(fx),
        }
    }

    /// The permanent note that prevents a wrong conclusion -- see the
    /// module doc. `None` for every feature that doesn't need one.
    fn tooltip(self) -> Option<&'static str> {
        match self {
            Self::Fresnel => Some(
                "Near-uniform on camera-facing billboards -- use on mesh \
                 particles (cone, sphere, tube).",
            ),
            Self::Soft => Some("Requires a depth prepass on the camera."),
            Self::Flow | Self::Erosion | Self::Scroll | Self::Gradient => None,
        }
    }
}

fn material_field(field: VariantField, drives: Vec<EmitterProp>) -> InspectorItem {
    InspectorItem::MaterialField {
        base_path: FX_MATERIAL_PATH,
        field,
        drives,
    }
}

pub fn scroll_section() -> (impl Bundle, InspectorSection) {
    (
        FxFeature::Scroll,
        InspectorSection::new(
            "FX \u{b7} Scroll",
            vec![
                vec![material_field(
                    VariantField::vector("fx.scroll", VectorSuffixes::XY),
                    vec![EmitterProp::ScrollU, EmitterProp::ScrollV],
                )],
                vec![material_field(
                    VariantField::vector("fx.tiling", VectorSuffixes::XY),
                    vec![],
                )],
            ],
        ),
    )
}

pub fn flow_section() -> (impl Bundle, InspectorSection) {
    (
        FxFeature::Flow,
        InspectorSection::new(
            "FX \u{b7} Flow",
            vec![
                vec![material_field(
                    VariantField::texture_ref("fx.flow_texture"),
                    vec![],
                )],
                vec![material_field(
                    VariantField::f32("fx.flow_strength"),
                    vec![EmitterProp::FlowStrength],
                )],
                vec![material_field(
                    VariantField::vector("fx.flow_scroll", VectorSuffixes::XY),
                    vec![],
                )],
            ],
        ),
    )
}

pub fn erosion_section() -> (impl Bundle, InspectorSection) {
    (
        FxFeature::Erosion,
        InspectorSection::new(
            "FX \u{b7} Erosion",
            vec![
                vec![material_field(
                    VariantField::texture_ref("fx.erosion_texture"),
                    vec![],
                )],
                vec![material_field(
                    VariantField::f32("fx.erosion_threshold")
                        .with_min(0.0)
                        .with_max(1.0),
                    vec![EmitterProp::ErosionThreshold],
                )],
                vec![material_field(
                    VariantField::f32("fx.erosion_edge")
                        .with_min(0.0)
                        .with_max(1.0),
                    vec![],
                )],
                vec![material_field(
                    VariantField::color("fx.erosion_edge_color"),
                    vec![],
                )],
            ],
        ),
    )
}

pub fn fresnel_section() -> (impl Bundle, InspectorSection) {
    (
        FxFeature::Fresnel,
        InspectorSection::new(
            "FX \u{b7} Fresnel",
            vec![
                vec![material_field(
                    VariantField::f32("fx.fresnel_power").with_min(0.0),
                    vec![EmitterProp::FresnelPower],
                )],
                vec![material_field(
                    VariantField::f32("fx.fresnel_boost").with_min(0.0),
                    vec![],
                )],
            ],
        ),
    )
}

pub fn soft_section() -> (impl Bundle, InspectorSection) {
    (
        FxFeature::Soft,
        InspectorSection::new(
            "FX \u{b7} Soft",
            vec![vec![material_field(
                VariantField::f32("fx.soft_fade").with_min(0.0),
                vec![],
            )]],
        ),
    )
}

pub fn gradient_remap_section() -> (impl Bundle, InspectorSection) {
    (
        FxFeature::Gradient,
        InspectorSection::new(
            "FX \u{b7} Gradient remap",
            vec![vec![material_field(
                VariantField::gradient("fx.gradient_remap"),
                vec![],
            )]],
        ),
    )
}

#[derive(Component, Clone, Copy)]
struct ResetFxButton(FxFeature);

/// Appends the tooltip (if any) and the "Reset to default" button to a
/// freshly-spawned FX section, once each -- `Added<FxFeature>` fires
/// exactly once per section entity, so unlike the `section_needs_setup`
/// helper other sections use (which exists to wait on `InspectorSection`'s
/// OWN field rows plus asset state), this content is static and needs no
/// "already set up" query at all.
fn setup_fx_section_extras(
    mut commands: Commands,
    new_sections: Query<(Entity, &FxFeature), Added<FxFeature>>,
) {
    for (entity, feature) in &new_sections {
        let mut extra_children = Vec::new();

        if let Some(note) = feature.tooltip() {
            let alert_entity = commands
                .spawn_scene(alert(AlertVariant::Info, vec![AlertSpan::Text(note.into())]))
                .id();
            extra_children.push(alert_entity);
        }

        let reset_row = commands.spawn(fields_row()).id();
        commands
            .spawn_scene(button(ButtonProps::new("Reset to default").align_left()))
            .insert(ResetFxButton(*feature))
            .insert(ChildOf(reset_row));
        extra_children.push(reset_row);

        commands.entity(entity).add_children(&extra_children);
    }
}

/// Hides all six FX sections when the currently inspected emitter's
/// material isn't `Standard`; shows them otherwise. See the module doc's
/// note on why this is `Display`-toggling rather than despawn/respawn, and
/// why it matches `draw_pass.rs`'s mask-cutoff row precedent rather than
/// leaving the sections merely inert.
fn sync_fx_sections_visibility(
    editor_state: Res<EditorState>,
    assets: Res<Assets<ParticlesAsset>>,
    mut sections: Query<&mut Node, With<FxFeature>>,
) {
    if !editor_state.is_changed() && !assets.is_changed() {
        return;
    }

    let is_standard = get_inspecting_emitter(&editor_state, &assets)
        .map(|(_, emitter)| matches!(emitter.draw_pass.material, DrawPassMaterial::Standard(_)))
        .unwrap_or(false);

    for mut node in &mut sections {
        super::set_display_visible(&mut node, is_standard);
    }
}

fn handle_fx_reset_click(
    trigger: On<ButtonClickEvent>,
    buttons: Query<&ResetFxButton>,
    mut ew: EmitterWriter,
) {
    let Ok(reset) = buttons.get(trigger.entity) else {
        return;
    };
    let feature = reset.0;

    ew.modify_emitter(|emitter| {
        let DrawPassMaterial::Standard(mat) = &mut emitter.draw_pass.material else {
            return false;
        };
        let before = mat.fx.clone();
        feature.clear(&mut mat.fx);
        mat.fx != before
    });
}

// --- Pure reset functions -------------------------------------------------
//
// Each resets exactly the fields the matching `FxSettings::*_enabled`
// predicate (`asset/fx.rs`) reads, to their value in `FxSettings::default()`.
// Kept field-by-field rather than e.g. `*fx = FxSettings::default()`
// (which would also stomp every OTHER feature's fields) -- clearing Fresnel
// must not silently also clear Scroll.

fn clear_scroll(fx: &mut FxSettings) {
    fx.scroll = Vec2::ZERO;
    fx.tiling = Vec2::ONE;
}

fn clear_flow(fx: &mut FxSettings) {
    fx.flow_texture = None;
    fx.flow_strength = 0.0;
    fx.flow_scroll = Vec2::ZERO;
}

fn clear_erosion(fx: &mut FxSettings) {
    fx.erosion_texture = None;
    fx.erosion_threshold = 0.0;
    fx.erosion_edge = 0.0;
    fx.erosion_edge_color = FxSettings::default().erosion_edge_color;
}

fn clear_fresnel(fx: &mut FxSettings) {
    fx.fresnel_power = 0.0;
    // `0.0` is NOT the default here: `fresnel_boost` defaults to the neutral
    // `1.0`, because it is a multiplier inside a feature `fresnel_power`
    // already gates off. Hardcoding zero would leave residue that silently
    // cancels the next fresnel the author switches on -- exactly the defect
    // this branch's I8 fixed. Same shape as `clear_erosion`'s edge-colour
    // line, for the same reason.
    fx.fresnel_boost = FxSettings::default().fresnel_boost;
}

fn clear_soft(fx: &mut FxSettings) {
    fx.soft_fade = 0.0;
}

fn clear_gradient(fx: &mut FxSettings) {
    fx.gradient_remap = None;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toggling_a_feature_off_restores_the_inert_default_exactly() {
        // Otherwise a disabled feature leaves residue that silently
        // re-enables itself via the *_enabled() predicates on the next load.
        let mut fx = FxSettings {
            fresnel_power: 2.0,
            fresnel_boost: 1.0,
            ..Default::default()
        };
        clear_fresnel(&mut fx);
        assert_eq!(fx, FxSettings::default());
    }

    #[test]
    fn clear_scroll_restores_the_inert_default() {
        let mut fx = FxSettings {
            scroll: Vec2::new(0.0, 0.3),
            tiling: Vec2::new(2.0, 2.0),
            ..Default::default()
        };
        clear_scroll(&mut fx);
        assert_eq!(fx, FxSettings::default());
        assert!(!fx.scroll_enabled());
    }

    #[test]
    fn clear_flow_restores_the_inert_default() {
        let mut fx = FxSettings {
            flow_texture: Some(TextureRef::Asset("flow.png".into())),
            flow_strength: 0.5,
            flow_scroll: Vec2::new(0.1, 0.2),
            ..Default::default()
        };
        clear_flow(&mut fx);
        assert_eq!(fx, FxSettings::default());
        assert!(!fx.flow_enabled());
    }

    #[test]
    fn clear_erosion_restores_the_inert_default() {
        let mut fx = FxSettings {
            erosion_texture: Some(TextureRef::Asset("erosion.png".into())),
            erosion_threshold: 0.4,
            erosion_edge: 0.2,
            erosion_edge_color: [0.0, 1.0, 0.0, 1.0],
            ..Default::default()
        };
        clear_erosion(&mut fx);
        assert_eq!(fx, FxSettings::default());
        assert!(!fx.erosion_enabled());
    }

    #[test]
    fn clear_soft_restores_the_inert_default() {
        let mut fx = FxSettings {
            soft_fade: 3.0,
            ..Default::default()
        };
        clear_soft(&mut fx);
        assert_eq!(fx, FxSettings::default());
        assert!(!fx.soft_enabled());
    }

    #[test]
    fn clear_gradient_restores_the_inert_default() {
        let mut fx = FxSettings {
            gradient_remap: Some(ParticleGradient::white()),
            ..Default::default()
        };
        clear_gradient(&mut fx);
        assert_eq!(fx, FxSettings::default());
        assert!(!fx.gradient_enabled());
    }

    /// Mutation check for the dispatch itself, not just the six functions
    /// it calls: if `FxFeature::clear`'s match ever called the wrong arm
    /// (e.g. `Self::Fresnel => clear_soft(fx)`), a test that only exercises
    /// `clear_fresnel` directly would stay green. This one goes through
    /// `FxFeature::Fresnel.clear(..)` instead.
    #[test]
    fn fx_feature_clear_dispatches_to_the_matching_function() {
        let mut fx = FxSettings {
            fresnel_power: 5.0,
            soft_fade: 5.0,
            ..Default::default()
        };
        FxFeature::Fresnel.clear(&mut fx);
        assert!(!fx.fresnel_enabled(), "Fresnel must be cleared");
        assert!(fx.soft_enabled(), "clearing Fresnel must not touch Soft");
    }

    #[test]
    fn fresnel_and_soft_carry_the_two_required_tooltips() {
        assert!(FxFeature::Fresnel.tooltip().unwrap().contains("billboards"));
        assert!(FxFeature::Soft.tooltip().unwrap().contains("depth prepass"));
    }

    #[test]
    fn every_other_feature_has_no_tooltip() {
        for feature in [
            FxFeature::Scroll,
            FxFeature::Flow,
            FxFeature::Erosion,
            FxFeature::Gradient,
        ] {
            assert!(feature.tooltip().is_none());
        }
    }

    // --- Fix round: section visibility and the reset-click observer -------

    use bevy::asset::AssetPlugin;
    use crate::state::{DirtyState, Inspectable, Inspecting};

    fn asset_with_material(material: DrawPassMaterial) -> ParticlesAsset {
        let mut emitter = EmitterData::default();
        emitter.draw_pass.material = material;
        ParticlesAsset::new(
            "t".into(),
            ParticlesDimension::D3,
            Default::default(),
            vec![emitter],
            vec![],
            false,
            ParticlesAuthors::default(),
        )
    }

    /// A section entity in the shape `setup_inspector_panel` actually
    /// spawns it: `FxFeature` plus a real `Node` (the `panel_section()`
    /// scene root always carries one). Seeded at the OPPOSITE `Display`
    /// from what each test expects the system to leave it at, so a no-op
    /// system would fail the assertion rather than passing by coincidence.
    fn app_with_section(material: DrawPassMaterial, start_hidden: bool) -> (App, Entity) {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_plugins(AssetPlugin::default());
        app.init_asset::<ParticlesAsset>();
        app.add_systems(Update, sync_fx_sections_visibility);

        let handle = {
            let mut assets = app.world_mut().resource_mut::<Assets<ParticlesAsset>>();
            assets.add(asset_with_material(material))
        };
        app.insert_resource(EditorState {
            current_project: Some(handle),
            current_project_path: None,
            inspecting: Some(Inspecting {
                kind: Inspectable::Emitter,
                index: 0,
            }),
        });

        let display = if start_hidden {
            Display::None
        } else {
            Display::Flex
        };
        let section = app
            .world_mut()
            .spawn((FxFeature::Scroll, Node { display, ..default() }))
            .id();

        app.update();
        (app, section)
    }

    #[test]
    fn fx_sections_are_shown_when_the_material_is_standard() {
        let (app, section) = app_with_section(
            DrawPassMaterial::Standard(StandardParticleMaterial::default()),
            true, // starts hidden -- the system must flip it to Flex
        );
        assert_eq!(app.world().get::<Node>(section).unwrap().display, Display::Flex);
    }

    #[test]
    fn fx_sections_are_hidden_when_the_material_is_customshader() {
        let (app, section) = app_with_section(
            DrawPassMaterial::CustomShader {
                vertex_shader: None,
                fragment_shader: None,
            },
            false, // starts visible -- the system must flip it to None
        );
        assert_eq!(app.world().get::<Node>(section).unwrap().display, Display::None);
    }

    fn app_for_reset(fresnel_power: f32) -> (App, Entity) {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_plugins(AssetPlugin::default());
        app.init_asset::<ParticlesAsset>();
        app.init_resource::<DirtyState>();
        app.add_observer(handle_fx_reset_click);

        let mut emitter = EmitterData::default();
        let DrawPassMaterial::Standard(mat) = &mut emitter.draw_pass.material else {
            unreachable!("EmitterData::default() carries a Standard material");
        };
        mat.fx.fresnel_power = fresnel_power;

        let asset = ParticlesAsset::new(
            "t".into(),
            ParticlesDimension::D3,
            Default::default(),
            vec![emitter],
            vec![],
            false,
            ParticlesAuthors::default(),
        );
        let handle = {
            let mut assets = app.world_mut().resource_mut::<Assets<ParticlesAsset>>();
            assets.add(asset)
        };
        app.insert_resource(EditorState {
            current_project: Some(handle),
            current_project_path: None,
            inspecting: Some(Inspecting {
                kind: Inspectable::Emitter,
                index: 0,
            }),
        });

        let button_entity = app.world_mut().spawn(ResetFxButton(FxFeature::Fresnel)).id();
        (app, button_entity)
    }

    fn fresnel_power_of(app: &App) -> f32 {
        let handle = app
            .world()
            .resource::<EditorState>()
            .current_project
            .clone()
            .unwrap();
        let asset = app
            .world()
            .resource::<Assets<ParticlesAsset>>()
            .get(&handle)
            .unwrap();
        let DrawPassMaterial::Standard(mat) = &asset.emitters[0].draw_pass.material else {
            unreachable!()
        };
        mat.fx.fresnel_power
    }

    /// Mirrors `drive_button.rs`'s `muting_a_drive_dirties_the_project`:
    /// every sibling button-click observer in this area gets an ECS-level
    /// test, not just its pure helper.
    #[test]
    fn clicking_reset_on_a_modified_section_restores_the_default_and_dirties() {
        let (mut app, button_entity) = app_for_reset(3.0);

        app.world_mut().trigger(ButtonClickEvent { entity: button_entity });

        assert!(
            app.world().resource::<DirtyState>().has_unsaved_changes,
            "resetting a modified section is a real edit and must dirty"
        );
        assert_eq!(fresnel_power_of(&app), 0.0);
    }

    /// The read-compare-write half of the same property (mirrors
    /// `drive_button.rs`'s `re_committing_the_same_mute_state_does_not_
    /// spuriously_dirty`): clicking Reset on an already-default section
    /// must not dirty, or every idle click on a clean section would look
    /// like an edit.
    #[test]
    fn clicking_reset_on_an_already_default_section_does_not_dirty() {
        let (mut app, button_entity) = app_for_reset(0.0);

        app.world_mut().trigger(ButtonClickEvent { entity: button_entity });

        assert!(
            !app.world().resource::<DirtyState>().has_unsaved_changes,
            "resetting an already-default section must not dirty"
        );
    }
}
