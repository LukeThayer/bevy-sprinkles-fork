//! The inspector content for a selected effect-owned light.
//!
//! `LightData` reuses `InitialTransform`/`EmitterTime`/`CurveTexture` --
//! exactly the types an emitter authors -- so most fields here are ordinary
//! `FieldBinding`-bound rows, wired through the same generic machinery
//! `binding::mod`'s `Inspectable::Light` arm resolves (see
//! `get_inspected_data`/`get_inspected_data_mut`). The transform and time
//! sections are declared purely declaratively (`InspectorSection::from_fields`)
//! exactly the way `inspector::transform`/`inspector::time` do for an
//! emitter, since every one of their fields is a plain `InspectorFieldProps`
//! row with nothing special about it.
//!
//! Two sections ("Light" and "Color") are instead built by hand, mirroring
//! `inspector::variable`'s and `inspector::collider_properties`'s dynamic
//! content, because three fields cannot go through the generic
//! `InspectorItem`/`spawn_inspector_field` dispatch at all:
//!
//! - `name` needs a plain, non-numeric text field. `spawn_inspector_field`'s
//!   generic fallback calls `.numeric_f32()` on every field kind it does not
//!   otherwise recognize, which would silently reject letters -- the same
//!   reason `inspector::variable`'s own Name field is hand-spawned rather
//!   than declared.
//! - `color` needs the `color_picker` widget. `spawn_inspector_field` has no
//!   `FieldKind::Color` arm at all (only `variant_edit.rs`'s dispatcher
//!   does), so a declared `Color` field would fall through to that same
//!   wrong numeric fallback.
//! - `intensity`/`range` each need Task 19's drive button beside them,
//!   targeting `DriveTarget::Light` (via `DriveButtonProps::new_light`)
//!   rather than `DriveTarget::Emitter` -- there is no declarative
//!   `InspectorItem` variant for a light-targeted drive, so these are wired
//!   the same way `setup_inspector_section_fields`'s own `Driven` arm wires
//!   an emitter one, just called directly instead of through that dispatch.
//!
//! `kind` (`FxLightKind::Point`/`Spot`) IS spawned through the generic
//! dispatch despite living in the hand-built "Light" section: unlike
//! `collider_properties.rs`'s shape combobox, `FxLightKind` carries no
//! per-variant data, so the plain reflect-based `set_enum_by_name` commit
//! path (`binding::commit::handle_combobox_change`) applies cleanly and
//! needs no dedicated observer -- `spawn_inspector_field` handles a
//! `FieldKind::ComboBox` row correctly, so there is no reason to hand-roll it
//! the way the collider's Box/Sphere swap must.
//!
//! Only three `LightProp` variants exist besides `Intensity`/`Range`:
//! `Hue`/`Saturation`/`Value` have no authored field on `LightData` to hang a
//! drive button off -- exactly `EmitterProp::SpawnProbability`'s situation --
//! and stay reachable only through `drives.rs`'s target picker (Task 20).
//!
//! **Respawn, not live-sync, for the structural fields.** `sync_effect_lights`
//! (`bevy_sprinkles::lights`) re-reads `intensity`/`range`/`color`/
//! `intensity_over_life`/`time` from the live asset every frame, so those
//! fields reach the preview with no extra wiring here. `kind`, `transform`,
//! `shadows` and `enabled` (the last painted by the shared title-bar
//! checkbox, not by this module) are read only once, at spawn, by
//! `setup_effect_lights` -- editing them needs a full light respawn, which
//! `binding::commit`'s `RESPAWN_FIELD_PATHS`/`mark_change` fires
//! automatically for exactly those paths. This module adds no respawn logic
//! of its own.

use bevy::prelude::*;
use bevy_sprinkles::asset::{FxLightKind, LightData, LightProp};
use bevy_sprinkles::prelude::*;

use crate::state::{EditorState, Inspectable};
use crate::ui::components::binding::{BindingTarget, FieldBinding};
use crate::ui::components::inspector::FieldKind;
use crate::ui::icons::ICON_TIME;
use crate::ui::widgets::color_picker::{ColorPickerProps, color_picker};
use crate::ui::widgets::inspector_field::{InspectorFieldProps, fields_row, spawn_inspector_field};
use crate::ui::widgets::text_edit::{TextEditProps, text_edit};
use crate::ui::widgets::vector_edit::VectorSuffixes;

use super::drive_button::{DriveButtonProps, drive_button};
use super::utils::combobox_options_from_reflect;
use super::{DynamicSectionContent, InspectorSection, section_needs_setup};

pub fn plugin(app: &mut App) {
    app.add_systems(
        Update,
        (setup_light_properties_content, setup_light_color_content)
            .after(super::update_inspected_light_tracker),
    );
}

// --- "Light" section: name, kind, shadows -------------------------------
//
// Hand-built for the `name` field's sake (see the module doc); `kind` and
// `shadows` are spawned through the generic dispatch anyway, just called
// directly rather than declared as `InspectorSection` rows, so the three
// live in one deterministic order instead of split across two mechanisms.

#[derive(Component)]
struct LightPropertiesSection;

#[derive(Component)]
struct LightPropertiesContent;

pub fn light_section() -> (impl Bundle, InspectorSection) {
    (
        LightPropertiesSection,
        InspectorSection::new("Light", vec![]),
    )
}

fn setup_light_properties_content(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    editor_state: Res<EditorState>,
    assets: Res<Assets<ParticlesAsset>>,
    sections: Query<(Entity, &InspectorSection), With<LightPropertiesSection>>,
    existing: Query<Entity, With<LightPropertiesContent>>,
) {
    let Some(entity) = section_needs_setup(&sections, &existing) else {
        return;
    };

    let name = current_light(&editor_state, &assets)
        .map(|l| l.name.clone())
        .unwrap_or_default();

    let content = commands
        .spawn((
            LightPropertiesContent,
            Node {
                width: percent(100),
                flex_direction: FlexDirection::Column,
                row_gap: px(12.0),
                ..default()
            },
        ))
        .with_children(|parent| {
            parent.spawn(fields_row()).with_children(|row| {
                let row_target = row.target_entity();
                row.commands()
                    .spawn_scene(text_edit(
                        TextEditProps::default()
                            .with_label("Name")
                            .with_default_value(&name),
                    ))
                    .insert(FieldBinding::emitter("name", FieldKind::String))
                    .insert(ChildOf(row_target));
            });

            parent.spawn(fields_row()).with_children(|row| {
                spawn_inspector_field(
                    row,
                    InspectorFieldProps::new("kind")
                        .combobox(combobox_options_from_reflect::<FxLightKind>()),
                    &asset_server,
                );
            });

            parent.spawn(fields_row()).with_children(|row| {
                spawn_inspector_field(
                    row,
                    InspectorFieldProps::new("shadows").bool(),
                    &asset_server,
                );
            });
        })
        .id();

    commands.entity(entity).add_child(content);
}

// --- Transform section: pure declarative, no custom code needed ----------

pub fn light_transform_section() -> (impl Bundle, InspectorSection) {
    let field = |path: &'static str| {
        InspectorFieldProps::new(path).with_target(BindingTarget::Inspected)
    };
    (
        (),
        InspectorSection::from_fields(
            "Transform",
            vec![
                field("transform.translation")
                    .vector(VectorSuffixes::XYZ)
                    .into(),
                field("transform.rotation")
                    .vector(VectorSuffixes::RollPitchYaw)
                    .with_suffix("\u{b0}")
                    .into(),
                field("transform.scale").vector(VectorSuffixes::XYZ).into(),
            ],
        ),
    )
}

// --- Time section: mirrors inspector/time.rs, minus the Lifetime drive
// button -- `LightProp` has no `Lifetime` variant to attach one to, since
// every light drive is ECS-stage, not spawn-stage. Pure declarative. --------

pub fn light_time_section() -> (impl Bundle, InspectorSection) {
    (
        (),
        InspectorSection::new(
            "Time",
            vec![
                vec![
                    InspectorFieldProps::new("time.lifetime")
                        .with_icon(ICON_TIME)
                        .with_suffix("s")
                        .into(),
                    InspectorFieldProps::new("time.lifetime_randomness")
                        .percent()
                        .with_icon(ICON_TIME)
                        .into(),
                ],
                vec![
                    InspectorFieldProps::new("time.delay")
                        .with_min(0.)
                        .with_icon(ICON_TIME)
                        .with_suffix("s")
                        .into(),
                ],
                vec![
                    InspectorFieldProps::new("time.explosiveness")
                        .percent()
                        .into(),
                    InspectorFieldProps::new("time.spawn_time_randomness")
                        .percent()
                        .into(),
                ],
                vec![
                    InspectorFieldProps::new("time.fixed_fps")
                        .u32_or_empty()
                        .with_placeholder("Unlimited")
                        .into(),
                    InspectorFieldProps::new("time.fixed_seed")
                        .optional_u32()
                        .with_placeholder("Random")
                        .into(),
                ],
                vec![InspectorFieldProps::new("time.one_shot").bool().into()],
            ],
        ),
    )
}

// --- Color section: color, intensity, range, intensity_over_life --------
//
// Hand-built for `color`'s sake (see the module doc); `intensity`/`range`
// are spawned through the generic dispatch too, each paired with a
// light-targeted drive button, and `intensity_over_life` is a plain curve
// field -- all three called directly rather than declared, for the same
// one-deterministic-order reason as the "Light" section above.

#[derive(Component)]
struct LightColorSection;

#[derive(Component)]
struct LightColorContent;

pub fn light_color_section() -> (impl Bundle, InspectorSection) {
    (LightColorSection, InspectorSection::new("Color", vec![]))
}

fn setup_light_color_content(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    sections: Query<(Entity, &InspectorSection), With<LightColorSection>>,
    existing: Query<Entity, With<LightColorContent>>,
) {
    let Some(entity) = section_needs_setup(&sections, &existing) else {
        return;
    };

    let content = commands
        .spawn((
            LightColorContent,
            DynamicSectionContent,
            Node {
                width: percent(100),
                flex_direction: FlexDirection::Column,
                row_gap: px(12.0),
                ..default()
            },
        ))
        .with_children(|parent| {
            parent.spawn(fields_row()).with_children(|row| {
                let row_target = row.target_entity();
                row.commands()
                    .spawn_scene(color_picker(ColorPickerProps::new()))
                    .insert(FieldBinding::emitter("color", FieldKind::Color))
                    .insert(ChildOf(row_target));
            });

            parent.spawn(fields_row()).with_children(|row| {
                spawn_inspector_field(
                    row,
                    InspectorFieldProps::new("intensity").with_min(0.0),
                    &asset_server,
                );
                let row_target = row.target_entity();
                row.commands()
                    .spawn_scene(drive_button(DriveButtonProps::new_light(
                        LightProp::Intensity,
                    )))
                    .insert(ChildOf(row_target));
            });

            parent.spawn(fields_row()).with_children(|row| {
                spawn_inspector_field(
                    row,
                    InspectorFieldProps::new("range")
                        .with_min(0.0)
                        .with_suffix("m"),
                    &asset_server,
                );
                let row_target = row.target_entity();
                row.commands()
                    .spawn_scene(drive_button(DriveButtonProps::new_light(LightProp::Range)))
                    .insert(ChildOf(row_target));
            });

            parent.spawn(fields_row()).with_children(|row| {
                spawn_inspector_field(
                    row,
                    InspectorFieldProps::new("intensity_over_life").curve(),
                    &asset_server,
                );
            });
        })
        .id();

    commands.entity(entity).add_child(content);
}

fn current_light<'a>(
    editor_state: &EditorState,
    assets: &'a Assets<ParticlesAsset>,
) -> Option<&'a LightData> {
    let inspecting = editor_state
        .inspecting
        .as_ref()
        .filter(|i| i.kind == Inspectable::Light)?;
    let handle = editor_state.current_project.as_ref()?;
    let asset = assets.get(handle)?;
    asset.lights.get(inspecting.index as usize)
}
