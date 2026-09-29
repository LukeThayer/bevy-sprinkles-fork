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

/// Marks the scrub text field's root entity with the variable it previews --
/// deliberately not a `FieldBinding`, so the generic commit path never
/// touches the asset for this field.
///
/// The NAME is all it stores. It used to carry a copy of `decl.range` as
/// well, captured when this section was built, and `on_scrub_commit` clamped
/// against that copy -- but this section is built ONCE
/// (`section_needs_setup` fires only while no content exists) while the
/// `range` field three rows above it edits the asset, so widening a range
/// left the scrub clamping to the old bounds with nothing on screen to
/// explain why the number would not go past the old maximum. The bounds are
/// read from the asset per commit now; see [`VariableScrub::set_clamped`].
#[derive(Component)]
struct ScrubField {
    name: String,
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
    editor_state: Res<EditorState>,
    assets: Res<Assets<ParticlesAsset>>,
    mut scrub: ResMut<VariableScrub>,
) {
    let Some((_, field)) = find_ancestor(trigger.entity, &fields, &parents) else {
        return;
    };
    let Ok(value) = trigger.text.trim().parse::<f32>() else {
        return;
    };
    let Some(asset) = editor_state
        .current_project
        .as_ref()
        .and_then(|h| assets.get(h))
    else {
        return;
    };
    scrub.set_clamped(&field.name, value, &asset.variables);
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::asset::AssetPlugin;
    use bevy_sprinkles::asset::VariableDecl;

    use crate::io::EditorData;
    use crate::state::{DirtyState, Inspecting};
    use crate::ui::components::inspector::{InspectedEmitterTracker, update_inspected_emitter_tracker};
    use crate::ui::widgets::color_picker::CheckerboardMaterial;
    use crate::ui::widgets::gradient_edit::GradientMaterial;
    use crate::ui::widgets::text_edit::EditorTextEdit;

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

    /// The property the whole `ScrubField`/`FieldBinding` split exists for:
    /// a scrub commit must never reach `commit::handle_text_commit`, because
    /// that system is what dirties the project. This does not merely check
    /// that `on_scrub_commit` behaves -- it drives the REAL
    /// `propagate_bindings` + `handle_text_commit` chain end to end, so a
    /// future change to either (e.g. `propagate_bindings` starting to widen
    /// its ancestor search, or `ScrubField` accidentally growing a
    /// `FieldBinding`) would be caught here, not discovered as a live bug
    /// report about the editor silently marking sessions dirty.
    #[test]
    fn scrubbing_a_preview_value_never_dirties_the_project() {
        let mut app = test_app();

        let root = app
            .world_mut()
            .spawn(ScrubField {
                name: "temperature".into(),
            })
            .id();
        let leaf = app.world_mut().spawn((EditorTextEdit, ChildOf(root))).id();

        // Let `propagate_bindings` process the `Added<EditorTextEdit>` leaf
        // first -- it will walk up to `root`, find no `FieldBinding` there
        // (only `ScrubField`), and attach no `BoundTo`.
        app.update();

        app.world_mut().trigger(TextEditCommitEvent {
            entity: leaf,
            text: "0.5".into(),
        });
        app.update();

        assert!(
            !app.world().resource::<DirtyState>().has_unsaved_changes,
            "a scrub commit must never dirty the project"
        );
    }

    /// The other half of the same property: this is not a test that commits
    /// silently do nothing. An ordinary `FieldBinding`-bearing field (the
    /// Name field's own shape, `FieldBinding::emitter("name", ..)`) must
    /// still dirty -- otherwise the first test above would pass for the
    /// wrong reason (commits broken entirely) rather than the right one
    /// (scrub fields specifically are excluded from the binding graph).
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

    /// An app carrying the real `on_scrub_commit` observer and one declared
    /// variable, so a commit can be driven end to end against a range the
    /// test then edits out from under the control.
    fn scrub_app(range: ParticleRange) -> (App, Handle<ParticlesAsset>) {
        let mut app = test_app();
        app.init_resource::<VariableScrub>();
        app.add_observer(on_scrub_commit);

        let handle = app
            .world()
            .resource::<EditorState>()
            .current_project
            .clone()
            .unwrap();
        {
            let mut assets = app.world_mut().resource_mut::<Assets<ParticlesAsset>>();
            let mut asset = assets.get_mut(&handle).unwrap();
            asset.variables = vec![VariableDecl {
                name: "temperature".into(),
                range,
                ..Default::default()
            }];
        }
        (app, handle)
    }

    fn commit_scrub(app: &mut App, text: &str) {
        let root = app
            .world_mut()
            .spawn(ScrubField {
                name: "temperature".into(),
            })
            .id();
        let leaf = app.world_mut().spawn((EditorTextEdit, ChildOf(root))).id();
        app.update();
        app.world_mut().trigger(TextEditCommitEvent {
            entity: leaf,
            text: text.into(),
        });
        app.update();
    }

    /// The desync this whole change exists for: the range field and the
    /// scrub control are two editors of the same variable, and the scrub
    /// used to clamp against a copy of the range captured when its section
    /// was built. Widening the range then did nothing -- the control went on
    /// refusing anything past the old maximum, with no error and nothing on
    /// screen to blame.
    ///
    /// The two commits below are the same text against two different ranges.
    /// The first pins that clamping still happens at all (so the second is
    /// not passing merely because the clamp was deleted).
    #[test]
    fn widening_a_variables_range_widens_what_the_scrub_accepts() {
        let (mut app, handle) = scrub_app(ParticleRange { min: 0.0, max: 1.0 });

        commit_scrub(&mut app, "5.0");
        assert_eq!(
            app.world().resource::<VariableScrub>().get("temperature"),
            Some(1.0),
            "5.0 must clamp to the declared maximum of 1.0"
        );

        {
            let mut assets = app.world_mut().resource_mut::<Assets<ParticlesAsset>>();
            let mut asset = assets.get_mut(&handle).unwrap();
            asset.variables[0].range = ParticleRange {
                min: 0.0,
                max: 10.0,
            };
        }

        commit_scrub(&mut app, "5.0");
        assert_eq!(
            app.world().resource::<VariableScrub>().get("temperature"),
            Some(5.0),
            "the clamp must follow the range the asset declares NOW, not the \
             one captured when the control was built"
        );
    }

    /// A scrub on a name the asset no longer declares is dropped rather than
    /// stored unclamped -- `retain_declared` would evict it on the next
    /// frame anyway, and storing it flashes a ghost value in between.
    #[test]
    fn a_scrub_on_an_undeclared_variable_stores_nothing() {
        let (mut app, handle) = scrub_app(ParticleRange { min: 0.0, max: 1.0 });
        {
            let mut assets = app.world_mut().resource_mut::<Assets<ParticlesAsset>>();
            assets.get_mut(&handle).unwrap().variables.clear();
        }
        commit_scrub(&mut app, "0.5");
        assert_eq!(
            app.world().resource::<VariableScrub>().get("temperature"),
            None
        );
    }
}
