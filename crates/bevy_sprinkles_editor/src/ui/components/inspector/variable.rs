//! The inspector content for a selected variable declaration.
//!
//! Name/default/range are ordinary asset-bound fields, wired through the
//! same generic `FieldBinding` machinery every other inspector field uses
//! (see `binding::mod`'s `Inspectable::Variable` arms) -- editing them marks
//! the project dirty exactly like an emitter or collider field would.
//!
//! **The preview-value control is NOT here.** It used to be, as a numeric
//! "Preview value" field at the bottom of this section, which meant a
//! variable had to be SELECTED before its knob could be turned -- and
//! turning knobs is the whole point of a preview, so the one interaction
//! the feature exists for sat behind an extra click. It now lives on every
//! row of the Variables list (`components::variables`), as the same widget
//! with the same "writes only to `VariableScrub`, never to the asset"
//! contract. It is not duplicated here: two live editors of one value is
//! precisely the desync that `VariableScrub::set_clamped` exists to end,
//! and the scrub value has no dirty flag for two surfaces to rebuild on.

use bevy::prelude::*;
use bevy_sprinkles::prelude::*;

use crate::state::{EditorState, Inspectable};
use crate::ui::components::binding::FieldBinding;
use crate::ui::components::inspector::FieldKind;
use crate::ui::icons::ICON_HASHTAG;
use crate::ui::widgets::inspector_field::{InspectorFieldProps, fields_row, spawn_inspector_field};
use crate::ui::widgets::text_edit::{TextEditProps, text_edit};
use crate::ui::widgets::vector_edit::VectorSuffixes;

use super::{DynamicSectionContent, InspectorSection, section_needs_setup};

#[derive(Component)]
struct VariableSection;

#[derive(Component)]
struct VariableContent;

pub fn plugin(app: &mut App) {
    app.add_systems(Update, setup_variable_content);
}

pub fn variable_section() -> (impl Bundle, InspectorSection) {
    (VariableSection, InspectorSection::new("Variable", vec![]))
}

fn setup_variable_content(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    editor_state: Res<EditorState>,
    assets: Res<Assets<ParticlesAsset>>,
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
        })
        .id();

    commands.entity(entity).add_child(content);
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::asset::AssetPlugin;

    use crate::io::EditorData;
    use crate::state::{DirtyState, Inspecting};
    use crate::ui::components::inspector::{InspectedEmitterTracker, update_inspected_emitter_tracker};
    use crate::ui::widgets::color_picker::CheckerboardMaterial;
    use crate::ui::widgets::gradient_edit::GradientMaterial;
    use crate::ui::widgets::text_edit::{EditorTextEdit, TextEditCommitEvent};

    /// A minimal App carrying the REAL commit path
    /// (`binding::plugin` -- `propagate_bindings` finding a `FieldBinding`
    /// ancestor, then `commit::handle_text_commit` dirtying on a change),
    /// not a hand-rolled stand-in for it. `binding`'s `commit`/`sync`/
    /// `swatch` submodules are private, so `binding::plugin` is the only
    /// externally reachable entry point -- there is no way to pull in just
    /// `propagate_bindings` and `handle_text_commit` without dragging in
    /// the swatch systems' asset types too, which is why
    /// `CheckerboardMaterial`/`GradientMaterial` are registered here despite
    /// having nothing to do with variables: `setup_variant_swatch` takes
    /// `ResMut<Assets<CheckerboardMaterial>>` unconditionally and would
    /// panic on a missing resource otherwise. `init_asset` alone (not the
    /// full `UiMaterialPlugin`) is enough -- these systems only read/write
    /// the `Assets<T>` storage, never the render/shader half.
    fn test_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_plugins(AssetPlugin::default());
        app.init_asset::<ParticlesAsset>();
        app.init_asset::<CheckerboardMaterial>();
        app.init_asset::<GradientMaterial>();

        app.init_resource::<DirtyState>();
        app.insert_resource(EditorData::default());
        app.init_resource::<InspectedEmitterTracker>();
        app.add_systems(Update, update_inspected_emitter_tracker);
        crate::ui::components::binding::plugin(&mut app);

        let handle = {
            let mut assets = app.world_mut().resource_mut::<Assets<ParticlesAsset>>();
            assets.add(ParticlesAsset::new(
                "t".into(),
                ParticlesDimension::D3,
                Default::default(),
                vec![EmitterData::default()],
                vec![],
                false,
                ParticlesAuthors::default(),
            ))
        };

        app.insert_resource(EditorState {
            current_project: Some(handle),
            current_project_path: None,
            inspecting: Some(Inspecting {
                kind: Inspectable::Emitter,
                index: 0,
            }),
        });

        app
    }

    /// The bound half of the variable section: an ordinary
    /// `FieldBinding`-bearing field (the Name field's own shape,
    /// `FieldBinding::emitter("name", ..)`) must dirty on commit.
    ///
    /// This is the surviving half of a pair. The other half lived here when
    /// the scrub control did -- it drove the same real
    /// `propagate_bindings` + `handle_text_commit` chain to prove a scrub
    /// commit never reaches it -- and moved with the control to
    /// `components::variables`. Keeping this one here is what stops that
    /// one from passing for the wrong reason (commits broken entirely)
    /// rather than the right one (scrub rows are excluded from the binding
    /// graph), so the two are a pair across two modules by necessity, not
    /// by oversight.
    #[test]
    fn editing_a_bound_field_still_dirties_the_project() {
        let mut app = test_app();

        let root = app
            .world_mut()
            .spawn(FieldBinding::emitter("name", FieldKind::String))
            .id();
        let leaf = app.world_mut().spawn((EditorTextEdit, ChildOf(root))).id();

        app.update();

        app.world_mut().trigger(TextEditCommitEvent {
            entity: leaf,
            text: "renamed".into(),
        });
        app.update();

        assert!(
            app.world().resource::<DirtyState>().has_unsaved_changes,
            "an ordinary bound field must still dirty on commit"
        );
    }
}
