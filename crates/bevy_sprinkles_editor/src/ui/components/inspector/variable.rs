//! The inspector content for a selected variable declaration.
//!
//! Name/default/range are ordinary asset-bound fields, wired through the
//! same generic `FieldBinding` machinery every other inspector field uses
//! (see `binding::mod`'s `Inspectable::Variable` arms) -- editing them marks
//! the project dirty exactly like an emitter or collider field would.
//!
//! The "Preview value" field is deliberately NOT asset-bound: it is the
//! slider called for in the brief, built from a plain numeric `text_edit`
//! rather than a real drag-slider widget, because this codebase has no
//! general-purpose slider (the only slider is the bespoke, shader-backed hue
//! and alpha strip inside `widgets::color_picker`, which is not reusable for
//! an arbitrary numeric range). Its commit writes only to `VariableScrub`,
//! found by walking up from the text input to a `ScrubField` ancestor the
//! same way `binding::propagate_bindings` walks up to a `FieldBinding` --
//! deliberately outside that system, so scrubbing can never dirty the asset.

use bevy::prelude::*;
use bevy_sprinkles::prelude::*;

use crate::state::{EditorState, Inspectable};
use crate::ui::components::binding::FieldBinding;
use crate::ui::components::inspector::FieldKind;
use crate::ui::components::variables::VariableScrub;
use crate::ui::icons::ICON_HASHTAG;
use crate::ui::widgets::inspector_field::{InspectorFieldProps, fields_row, spawn_inspector_field};
use crate::ui::widgets::text_edit::{TextEditCommitEvent, TextEditProps, text_edit};
use crate::ui::widgets::utils::find_ancestor;
use crate::ui::widgets::vector_edit::VectorSuffixes;

use super::{DynamicSectionContent, InspectorSection, section_needs_setup};

#[derive(Component)]
struct VariableSection;

#[derive(Component)]
struct VariableContent;

/// Marks the scrub text field's root entity with the variable name and
/// declared range it previews -- deliberately not a `FieldBinding`, so the
/// generic commit path never touches the asset for this field.
#[derive(Component)]
struct ScrubField {
    name: String,
    min: f32,
    max: f32,
}

pub fn plugin(app: &mut App) {
    app.add_observer(on_scrub_commit)
        .add_systems(Update, setup_variable_content);
}

pub fn variable_section() -> (impl Bundle, InspectorSection) {
    (VariableSection, InspectorSection::new("Variable", vec![]))
}

fn setup_variable_content(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    editor_state: Res<EditorState>,
    assets: Res<Assets<ParticlesAsset>>,
    scrub: Res<VariableScrub>,
    sections: Query<(Entity, &InspectorSection), With<VariableSection>>,
    existing: Query<Entity, With<VariableContent>>,
) {
    let Some(entity) = section_needs_setup(&sections, &existing) else {
        return;
    };

    let inspecting = editor_state
        .inspecting
        .as_ref()
        .filter(|i| i.kind == Inspectable::Variable);
    let Some(inspecting) = inspecting else {
        return;
    };

    let decl = editor_state
        .current_project
        .as_ref()
        .and_then(|h| assets.get(h))
        .and_then(|a| a.variables.get(inspecting.index as usize));
    let Some(decl) = decl else {
        return;
    };

    let scrub_value = scrub.value_or_default(&decl.name, std::slice::from_ref(decl));

    let content = commands
        .spawn((
            VariableContent,
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
                    .spawn_scene(text_edit(
                        TextEditProps::default()
                            .with_label("Name")
                            .with_default_value(&decl.name),
                    ))
                    .insert(FieldBinding::emitter("name", FieldKind::String))
                    .insert(ChildOf(row_target));
            });

            parent.spawn(fields_row()).with_children(|row| {
                spawn_inspector_field(
                    row,
                    InspectorFieldProps::new("default").with_icon(ICON_HASHTAG),
                    &asset_server,
                );
            });

            parent.spawn(fields_row()).with_children(|row| {
                spawn_inspector_field(
                    row,
                    InspectorFieldProps::new("range").vector(VectorSuffixes::Range),
                    &asset_server,
                );
            });

            parent.spawn(fields_row()).with_children(|row| {
                let row_target = row.target_entity();
                row.commands()
                    .spawn_scene(text_edit(
                        TextEditProps::default()
                            .with_label("Preview value")
                            .with_default_value(&format_f32(scrub_value))
                            .numeric_f32()
                            .with_min(decl.range.min as f64)
                            .with_max(decl.range.max as f64),
                    ))
                    .insert(ScrubField {
                        name: decl.name.clone(),
                        min: decl.range.min,
                        max: decl.range.max,
                    })
                    .insert(ChildOf(row_target));
            });
        })
        .id();

    commands.entity(entity).add_child(content);
}

fn format_f32(v: f32) -> String {
    let mut text = v.to_string();
    if !text.contains('.') {
        text.push_str(".0");
    }
    text
}

fn on_scrub_commit(
    trigger: On<TextEditCommitEvent>,
    fields: Query<&ScrubField>,
    parents: Query<&ChildOf>,
    mut scrub: ResMut<VariableScrub>,
) {
    let Some((_, field)) = find_ancestor(trigger.entity, &fields, &parents) else {
        return;
    };
    let Ok(value) = trigger.text.trim().parse::<f32>() else {
        return;
    };
    let (lo, hi) = (field.min.min(field.max), field.min.max(field.max));
    scrub.set(&field.name, value.clamp(lo, hi));
}
