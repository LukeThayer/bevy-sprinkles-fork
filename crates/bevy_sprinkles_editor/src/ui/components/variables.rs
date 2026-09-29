//! The Variables panel: an outliner list (this module) plus an inspector
//! section (`inspector::variable`) for the currently selected declaration.
//!
//! `VariableId` is positional -- it indexes into `ParticlesAsset::variables`
//! -- and `validate_drives` rejects a `Drive` naming an out-of-range id at
//! load. So deleting a declaration must both drop any drive that targeted it
//! AND renumber every drive above it, or the editor would happily write a
//! file it cannot reopen itself. `remove_variable` is the one place that
//! does this; every deletion path (the list's delete button, and the
//! generic outliner delete-confirmation flow in `data_panel.rs`, kept
//! exhaustive but otherwise unreachable for variables) routes through it.
//!
//! **The scrub control sits on every row of this list**, not in the
//! inspector. It was a "Preview value" field inside
//! `inspector::variable`'s section, which meant selecting a variable before
//! its knob could be turned -- and turning knobs is what a preview is FOR,
//! so the one interaction the feature exists to support was the one behind
//! an extra click. It is the same widget, moved: a plain numeric
//! `text_edit` (this codebase has no general-purpose slider; the only one
//! is the bespoke shader-backed hue/alpha strip in `widgets::color_picker`),
//! writing only to [`VariableScrub`] and never to the asset.
//!
//! There is deliberately only ONE of it. The inspector's copy is gone
//! rather than kept in sync, because two live editors of one value is the
//! desync class the range-clamp fix next door was about -- and unlike the
//! Drives list and its popover, which agree by both rebuilding from the
//! asset, the scrub value lives in a resource that nothing dirties, so a
//! second editor would have no rebuild signal to share.

use std::collections::HashMap;

use bevy::prelude::*;
use bevy_sprinkles::asset::{VariableDecl, VariableId};
use bevy_sprinkles::prelude::*;

use crate::state::{DirtyState, EditorState, Inspectable, Inspecting};
use crate::ui::components::data_panel::EditorDataPanel;
use crate::ui::icons::ICON_CLOSE;
use crate::ui::widgets::button::{
    ButtonClickEvent, ButtonProps, ButtonVariant, IconButtonProps, button, icon_button,
};
use crate::ui::widgets::panel_section::{PanelSectionProps, panel_section};
use crate::ui::widgets::text_edit::{TextEditCommitEvent, TextEditProps, text_edit};
use crate::ui::widgets::utils::find_ancestor;
use crate::viewport::EditorParticlePreview;

pub fn plugin(app: &mut App) {
    app.init_resource::<VariableScrub>()
        .add_observer(on_add_variable)
        .add_observer(on_select_variable_click)
        .add_observer(on_delete_variable_click)
        .add_observer(on_row_scrub_commit)
        .add_systems(
            Update,
            (
                setup_variables_section,
                rebuild_variable_list,
                apply_variable_scrub,
            ),
        );
}

/// The editor's live knob values for the previewed effect.
///
/// Separate from the asset because these are a VIEWING state, not authored
/// content: scrubbing temperature to 0.9 to see what happens must never dirty
/// the project or end up saved. The authored value is `VariableDecl::default`.
#[derive(Resource, Default)]
pub struct VariableScrub(HashMap<String, f32>);

impl VariableScrub {
    pub fn set(&mut self, name: &str, v: f32) {
        self.0.insert(name.to_string(), v);
    }
    pub fn get(&self, name: &str) -> Option<f32> {
        self.0.get(name).copied()
    }

    /// Stores a scrubbed value clamped to the variable's range **as the
    /// asset declares it right now**, rather than to whatever the range was
    /// when the control was built.
    ///
    /// The bounds are looked up here, per commit, on purpose. The editor's
    /// range field and its scrub control are two separate surfaces onto the
    /// same variable, and the scrub control used to carry its own copy of
    /// `decl.range` captured at setup time -- so widening the range in the
    /// inspector left the scrub still clamping to the old one, with nothing
    /// on screen to explain why the number would not go past 1.0. Deriving
    /// the bounds from the asset is the fix that cannot be forgotten;
    /// re-syncing a stored copy is the one that already was.
    ///
    /// An undeclared name is ignored rather than stored unclamped:
    /// [`retain_declared`](Self::retain_declared) would drop the entry on
    /// the next frame anyway, and storing it would flash a ghost value in
    /// between.
    pub fn set_clamped(&mut self, name: &str, value: f32, decls: &[VariableDecl]) {
        let Some(decl) = decls.iter().find(|d| d.name == name) else {
            return;
        };
        let (lo, hi) = (
            decl.range.min.min(decl.range.max),
            decl.range.min.max(decl.range.max),
        );
        self.set(name, value.clamp(lo, hi));
    }

    pub fn value_or_default(&self, name: &str, decls: &[VariableDecl]) -> f32 {
        self.get(name).unwrap_or_else(|| {
            decls
                .iter()
                .find(|d| d.name == name)
                .map(|d| d.default)
                .unwrap_or(0.0)
        })
    }

    /// Drops entries for variables that no longer exist, so a rename or a
    /// delete cannot leave a ghost that shows a stale number and drives
    /// nothing.
    pub fn retain_declared(&mut self, decls: &[VariableDecl]) {
        self.0.retain(|k, _| decls.iter().any(|d| &d.name == k));
    }

    pub fn apply_to(&self, vars: &mut ParticleVariables) {
        for (k, v) in &self.0 {
            vars.set(k, *v);
        }
    }
}

/// Removes the declaration at `index`, keeping the file loadable.
///
/// `VariableId` is positional (see the module doc), so removing an entry
/// shifts every id above it down by one. Two things follow, both required:
/// a drive that named exactly this variable is dropped (it would otherwise
/// point at whatever now occupies this slot, or at nothing), and every
/// surviving drive above it is renumbered to match the shift. Skipping
/// either leaves an asset that `validate_drives` rejects on the very next
/// load -- the author sees no error until they reopen the file.
pub fn remove_variable(asset: &mut ParticlesAsset, index: usize) {
    if index >= asset.variables.len() {
        return;
    }
    asset.variables.remove(index);
    asset.drives.retain(|d| d.variable.0 as usize != index);
    for drive in asset.drives.iter_mut() {
        if drive.variable.0 as usize > index {
            drive.variable = VariableId(drive.variable.0 - 1);
        }
    }
}

// --- Outliner list ---------------------------------------------------

#[derive(Component)]
struct VariablesSection;

#[derive(Component)]
struct VariableRowList;

#[derive(Component, Clone, Copy)]
struct VariableSelectButton(u8);

#[derive(Component, Clone, Copy)]
struct VariableDeleteButton(u8);

/// Marks one row's scrub field with the variable it previews.
///
/// The NAME is all it stores, on purpose: [`VariableScrub`] is keyed by
/// name, and a row's index would go stale the moment a variable above it
/// was deleted. It is deliberately not a `FieldBinding` either -- that is
/// the component `binding::propagate_bindings` walks up to find, and
/// carrying one would route this commit through `handle_text_commit`,
/// which dirties the project. Scrubbing is a viewing state; see
/// [`VariableScrub`]'s own doc.
#[derive(Component)]
struct VariableScrubRow {
    name: String,
}

#[derive(Event)]
struct AddVariableEvent;

/// Width of a row's scrub field. Wide enough for a signed three-decimal
/// value without the name button beside it collapsing; the name has the
/// row's remaining space and shrinks, the scrub does not.
const SCRUB_FIELD_WIDTH: f32 = 84.0;

/// Renders a scrub value the way the numeric `text_edit` expects to parse
/// it back -- an integral float keeps its `.0` rather than reading as an
/// integer field.
fn format_f32(v: f32) -> String {
    let mut text = v.to_string();
    if !text.contains('.') {
        text.push_str(".0");
    }
    text
}

/// A row's scrub commit: clamp to the variable's CURRENT declared range and
/// store it in [`VariableScrub`], touching neither the asset nor
/// `DirtyState`.
///
/// It walks up from the text input to its `VariableScrubRow` ancestor the
/// same way `binding::propagate_bindings` walks up to a `FieldBinding` --
/// deliberately outside that system, so scrubbing can never dirty the
/// project.
fn on_row_scrub_commit(
    trigger: On<TextEditCommitEvent>,
    rows: Query<&VariableScrubRow>,
    parents: Query<&ChildOf>,
    editor_state: Res<EditorState>,
    assets: Res<Assets<ParticlesAsset>>,
    mut scrub: ResMut<VariableScrub>,
) {
    let Some((_, row)) = find_ancestor(trigger.entity, &rows, &parents) else {
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
    scrub.set_clamped(&row.name, value, &asset.variables);
}

fn setup_variables_section(mut commands: Commands, panels: Query<Entity, Added<EditorDataPanel>>) {
    for panel_entity in &panels {
        commands
            .spawn_scene(panel_section(
                PanelSectionProps::new("Variables").with_add_button(),
            ))
            .insert((VariablesSection, ChildOf(panel_entity)))
            .observe(on_add_variable_click);
    }
}

fn on_add_variable_click(_event: On<ButtonClickEvent>, mut commands: Commands) {
    commands.trigger(AddVariableEvent);
}

fn on_add_variable(
    _event: On<AddVariableEvent>,
    mut editor_state: ResMut<EditorState>,
    mut assets: ResMut<Assets<ParticlesAsset>>,
    mut dirty_state: ResMut<DirtyState>,
) {
    let Some(handle) = &editor_state.current_project else {
        return;
    };
    let Some(mut asset) = assets.get_mut(handle) else {
        return;
    };

    let existing: Vec<&str> = asset.variables.iter().map(|v| v.name.as_str()).collect();
    let name = next_unique_variable_name(&existing);
    let new_index = asset.variables.len() as u8;
    asset.variables.push(VariableDecl {
        name,
        ..Default::default()
    });

    dirty_state.has_unsaved_changes = true;
    editor_state.inspecting = Some(Inspecting {
        kind: Inspectable::Variable,
        index: new_index,
    });
}

fn next_unique_variable_name(existing: &[&str]) -> String {
    let mut n = 1;
    loop {
        let candidate = format!("variable_{n}");
        if !existing.iter().any(|name| *name == candidate) {
            return candidate;
        }
        n += 1;
    }
}

fn on_select_variable_click(
    trigger: On<ButtonClickEvent>,
    buttons: Query<&VariableSelectButton>,
    mut editor_state: ResMut<EditorState>,
) {
    let Ok(button) = buttons.get(trigger.entity) else {
        return;
    };
    editor_state.inspecting = Some(Inspecting {
        kind: Inspectable::Variable,
        index: button.0,
    });
}

fn on_delete_variable_click(
    trigger: On<ButtonClickEvent>,
    buttons: Query<&VariableDeleteButton>,
    mut editor_state: ResMut<EditorState>,
    mut assets: ResMut<Assets<ParticlesAsset>>,
    mut dirty_state: ResMut<DirtyState>,
) {
    let Ok(button) = buttons.get(trigger.entity) else {
        return;
    };
    let index = button.0 as usize;

    let Some(handle) = &editor_state.current_project else {
        return;
    };
    let Some(mut asset) = assets.get_mut(handle) else {
        return;
    };
    if index >= asset.variables.len() {
        return;
    }

    remove_variable(&mut asset, index);
    dirty_state.has_unsaved_changes = true;
    let new_len = asset.variables.len();

    if let Some(current) = editor_state.inspecting {
        if current.kind == Inspectable::Variable {
            if current.index as usize == index {
                editor_state.inspecting = if new_len > 0 {
                    Some(Inspecting {
                        kind: Inspectable::Variable,
                        index: 0,
                    })
                } else {
                    None
                };
            } else if current.index as usize > index {
                editor_state.inspecting = Some(Inspecting {
                    kind: Inspectable::Variable,
                    index: current.index - 1,
                });
            }
        }
    }
}

/// Rebuilds the whole row list whenever anything that could change it
/// changes: the project/selection (`EditorState`) or a committed edit
/// (`DirtyState`, which a rename via the inspector's own Name field also
/// sets). This is simpler and safer than tracking which specific mutation
/// happened, at the cost of a full rebuild on any edit -- fine for a list
/// this small.
///
/// That rebuild is also what keeps each row's scrub field honest: the
/// widget clamps typed input against a `NumericRange` component captured
/// when it was spawned (`widgets::text_edit`), so a range widened in the
/// inspector would otherwise leave the field refusing the new maximum even
/// though [`VariableScrub::set_clamped`] would accept it. Editing a range
/// dirties, a dirty rebuilds this list, and the respawned field carries
/// the new bounds.
///
/// [`VariableScrub`] is read but NOT watched for change. It is rewritten
/// every frame by `apply_variable_scrub` (`retain_declared` takes `&mut
/// self` unconditionally), so watching it would rebuild this list on every
/// frame -- and would tear down the very field the author is typing into.
fn rebuild_variable_list(
    mut commands: Commands,
    editor_state: Res<EditorState>,
    dirty_state: Res<DirtyState>,
    assets: Res<Assets<ParticlesAsset>>,
    scrub: Res<VariableScrub>,
    section: Query<Entity, With<VariablesSection>>,
    new_sections: Query<Entity, Added<VariablesSection>>,
    existing_rows: Query<Entity, With<VariableRowList>>,
) {
    let should_rebuild =
        !new_sections.is_empty() || editor_state.is_changed() || dirty_state.is_changed();
    if !should_rebuild {
        return;
    }

    let Ok(section_entity) = section.single() else {
        return;
    };

    for entity in &existing_rows {
        commands.entity(entity).despawn();
    }

    let Some(handle) = &editor_state.current_project else {
        return;
    };
    let Some(asset) = assets.get(handle) else {
        return;
    };

    let list_entity = commands
        .spawn((
            VariableRowList,
            Node {
                width: percent(100),
                flex_direction: FlexDirection::Column,
                row_gap: px(6.0),
                ..default()
            },
        ))
        .id();
    commands.entity(section_entity).add_child(list_entity);

    for (i, variable) in asset.variables.iter().enumerate() {
        let index = i as u8;
        let is_active = editor_state
            .inspecting
            .map(|ins| ins.kind == Inspectable::Variable && ins.index == index)
            .unwrap_or(false);
        let variant = if is_active {
            ButtonVariant::Active
        } else {
            ButtonVariant::Ghost
        };

        let row = commands
            .spawn(Node {
                width: percent(100),
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::Center,
                column_gap: px(4.0),
                ..default()
            })
            .id();
        commands.entity(list_entity).add_child(row);

        let select_wrapper = commands
            .spawn(Node {
                flex_grow: 1.0,
                flex_shrink: 1.0,
                flex_basis: px(0.0),
                ..default()
            })
            .id();
        commands.entity(row).add_child(select_wrapper);

        let select_entity = commands
            .spawn_scene(button(
                ButtonProps::new(&variable.name)
                    .with_variant(variant)
                    .align_left(),
            ))
            .insert(VariableSelectButton(index))
            .id();
        commands.entity(select_wrapper).add_child(select_entity);

        // The scrub field, on the row rather than behind a selection. Its
        // bounds and its shown value are both read from the asset and the
        // resource right here, every rebuild -- nothing is cached on the
        // row itself but the name.
        let scrub_wrapper = commands
            .spawn(Node {
                width: px(SCRUB_FIELD_WIDTH),
                flex_shrink: 0.0,
                ..default()
            })
            .id();
        commands.entity(row).add_child(scrub_wrapper);

        let scrub_value = scrub.value_or_default(&variable.name, &asset.variables);
        let scrub_entity = commands
            .spawn_scene(text_edit(
                TextEditProps::default()
                    .with_default_value(format_f32(scrub_value))
                    .numeric_f32()
                    .with_min(variable.range.min as f64)
                    .with_max(variable.range.max as f64),
            ))
            .insert(VariableScrubRow {
                name: variable.name.clone(),
            })
            .id();
        commands.entity(scrub_wrapper).add_child(scrub_entity);

        let delete_entity = commands
            .spawn_scene(icon_button(
                IconButtonProps::new(ICON_CLOSE).variant(ButtonVariant::Ghost),
            ))
            .insert(VariableDeleteButton(index))
            .id();
        commands.entity(row).add_child(delete_entity);
    }
}

// --- Live preview wiring ----------------------------------------------

/// Every frame: drops stale scrub entries (a renamed/deleted variable) and
/// pushes the surviving ones onto the previewed entity's `ParticleVariables`,
/// inserting the component if it is not there yet. The row's scrub field IS
/// the preview mechanism -- there is no separate preview concept, and no
/// second surface driving these values since the inspector's copy was
/// folded into the list (see the module doc).
fn apply_variable_scrub(
    mut commands: Commands,
    editor_state: Res<EditorState>,
    assets: Res<Assets<ParticlesAsset>>,
    mut scrub: ResMut<VariableScrub>,
    preview: Query<Entity, With<EditorParticlePreview>>,
    mut existing_vars: Query<&mut ParticleVariables>,
) {
    let Some(handle) = &editor_state.current_project else {
        return;
    };
    let Some(asset) = assets.get(handle) else {
        return;
    };
    scrub.retain_declared(&asset.variables);

    let Ok(entity) = preview.single() else {
        return;
    };

    if let Ok(mut vars) = existing_vars.get_mut(entity) {
        scrub.apply_to(&mut vars);
    } else {
        let mut vars = ParticleVariables::default();
        scrub.apply_to(&mut vars);
        commands.entity(entity).insert(vars);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy_sprinkles::asset::drive::validate_drives;
    use bevy_sprinkles::asset::{Drive, DriveOp, DriveTarget, EmitterProp};

    #[test]
    fn scrubbing_a_variable_reaches_the_previewed_entitys_particle_variables() {
        // The row's scrub field IS the preview mechanism -- there is no
        // separate preview concept -- so this wiring is the feature, not a
        // convenience.
        let mut scrub = VariableScrub::default();
        scrub.set("temperature", 0.75);
        let mut vars = ParticleVariables::default();
        scrub.apply_to(&mut vars);
        assert_eq!(vars.get("temperature"), Some(0.75));
    }

    #[test]
    fn a_renamed_variable_drops_its_stale_scrub_value() {
        // Otherwise a rename leaves a ghost entry that silently drives nothing
        // and shows a stale number next to the new name.
        let mut scrub = VariableScrub::default();
        scrub.set("heat", 0.9);
        scrub.retain_declared(&[VariableDecl {
            name: "temperature".into(),
            ..Default::default()
        }]);
        assert_eq!(scrub.get("heat"), None);
    }

    #[test]
    fn an_undeclared_variable_scrubs_to_its_declared_default() {
        let scrub = VariableScrub::default();
        let decls = vec![VariableDecl {
            name: "t".into(),
            default: 0.3,
            ..Default::default()
        }];
        assert_eq!(scrub.value_or_default("t", &decls), 0.3);
    }

    fn asset_with(variables: Vec<VariableDecl>, drives: Vec<Drive>) -> ParticlesAsset {
        let mut a = ParticlesAsset::new(
            "t".into(),
            ParticlesDimension::D3,
            Default::default(),
            vec![EmitterData::default()],
            vec![],
            false,
            ParticlesAuthors::default(),
        );
        a.variables = variables;
        a.drives = drives;
        a
    }

    fn var(name: &str) -> VariableDecl {
        VariableDecl {
            name: name.into(),
            ..Default::default()
        }
    }

    fn drive_on(variable: u16) -> Drive {
        Drive {
            variable: VariableId(variable),
            target: DriveTarget::Emitter {
                index: 0,
                prop: EmitterProp::Tint,
            },
            curve: CurveTexture::default(),
            output: ParticleRange { min: 0.0, max: 1.0 },
            op: DriveOp::Multiply,
            muted: false,
        }
    }

    #[test]
    fn deleting_a_variable_renumbers_the_drives_above_it() {
        let mut asset = asset_with(vec![var("a"), var("b"), var("c")], vec![drive_on(2)]);
        remove_variable(&mut asset, 1);
        assert_eq!(
            asset.drives[0].variable,
            VariableId(1),
            "ids are positional and must shift"
        );
    }

    #[test]
    fn deleting_a_variable_removes_the_drives_that_used_it() {
        let mut asset = asset_with(vec![var("a"), var("b")], vec![drive_on(0)]);
        remove_variable(&mut asset, 0);
        assert!(
            asset.drives.is_empty(),
            "a drive with no variable would fail validation on load"
        );
    }

    #[test]
    fn a_delete_that_renumbers_still_leaves_the_asset_loadable() {
        // The direct pin on the property this whole module exists to protect:
        // the editor must never write a file that fails its own load
        // validation.
        let mut asset = asset_with(vec![var("a"), var("b"), var("c")], vec![drive_on(2)]);
        remove_variable(&mut asset, 1);
        assert!(validate_drives(&asset).is_ok());
    }

    #[test]
    fn deleting_an_out_of_range_index_is_a_no_op() {
        let mut asset = asset_with(vec![var("a")], vec![]);
        remove_variable(&mut asset, 5);
        assert_eq!(asset.variables.len(), 1);
    }

    // --- The row's scrub control -------------------------------------
    //
    // These moved here with the control itself, from
    // `inspector::variable`. Its `editing_a_bound_field_still_dirties_the_
    // project` stayed behind on purpose and is the other half of the first
    // test below: without it, "a scrub never dirties" could pass because
    // commits are broken outright rather than because scrub rows are
    // excluded from the binding graph.

    /// A minimal App carrying the REAL commit path
    /// (`binding::plugin` -- `propagate_bindings` finding a `FieldBinding`
    /// ancestor, then `commit::handle_text_commit` dirtying on a change),
    /// not a hand-rolled stand-in for it. `inspector::variable`'s copy of
    /// this harness carries the full explanation of why
    /// `CheckerboardMaterial`/`GradientMaterial` are registered in a test
    /// about variables (short version: `binding::plugin` is the only
    /// reachable entry point and it drags the swatch systems in, which
    /// take those `Assets<T>` unconditionally).
    fn scrub_app(range: ParticleRange) -> (App, Handle<ParticlesAsset>) {
        use crate::io::EditorData;
        use crate::ui::components::inspector::{
            InspectedEmitterTracker, update_inspected_emitter_tracker,
        };
        use crate::ui::widgets::color_picker::CheckerboardMaterial;
        use crate::ui::widgets::gradient_edit::GradientMaterial;

        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_plugins(bevy::asset::AssetPlugin::default());
        app.init_asset::<ParticlesAsset>();
        app.init_asset::<CheckerboardMaterial>();
        app.init_asset::<GradientMaterial>();
        app.init_resource::<DirtyState>();
        app.init_resource::<VariableScrub>();
        app.insert_resource(EditorData::default());
        app.init_resource::<InspectedEmitterTracker>();
        app.add_systems(Update, update_inspected_emitter_tracker);
        crate::ui::components::binding::plugin(&mut app);
        app.add_observer(on_row_scrub_commit);

        let mut asset = ParticlesAsset::new(
            "t".into(),
            ParticlesDimension::D3,
            Default::default(),
            vec![EmitterData::default()],
            vec![],
            false,
            ParticlesAuthors::default(),
        );
        asset.variables = vec![VariableDecl {
            name: "temperature".into(),
            range,
            ..Default::default()
        }];

        let handle = {
            let mut assets = app.world_mut().resource_mut::<Assets<ParticlesAsset>>();
            assets.add(asset)
        };
        app.insert_resource(EditorState {
            current_project: Some(handle.clone()),
            current_project_path: None,
            inspecting: None,
        });
        (app, handle)
    }

    /// Drives a commit through the row marker the way a typed value does:
    /// the text input is a DESCENDANT, so this also exercises the
    /// `find_ancestor` walk rather than triggering on the marker itself.
    fn commit_scrub(app: &mut App, text: &str) {
        use crate::ui::widgets::text_edit::EditorTextEdit;

        let root = app
            .world_mut()
            .spawn(VariableScrubRow {
                name: "temperature".into(),
            })
            .id();
        let leaf = app.world_mut().spawn((EditorTextEdit, ChildOf(root))).id();
        // Let `propagate_bindings` process the `Added<EditorTextEdit>` leaf
        // first -- it walks up to `root`, finds no `FieldBinding` there
        // (only `VariableScrubRow`), and attaches no `BoundTo`.
        app.update();
        app.world_mut().trigger(TextEditCommitEvent {
            entity: leaf,
            text: text.into(),
        });
        app.update();
    }

    /// The property the whole `VariableScrubRow`/`FieldBinding` split
    /// exists for: a scrub commit must never reach
    /// `commit::handle_text_commit`, because that system is what dirties
    /// the project. It drives the real chain end to end, so a future change
    /// to either (`propagate_bindings` widening its ancestor search, or
    /// this row accidentally growing a `FieldBinding`) is caught here
    /// rather than reported as the editor silently marking sessions dirty.
    #[test]
    fn scrubbing_a_preview_value_never_dirties_the_project() {
        let (mut app, _) = scrub_app(ParticleRange { min: 0.0, max: 1.0 });
        commit_scrub(&mut app, "0.5");
        assert!(
            !app.world().resource::<DirtyState>().has_unsaved_changes,
            "a scrub commit must never dirty the project"
        );
        assert_eq!(
            app.world().resource::<VariableScrub>().get("temperature"),
            Some(0.5),
            "and it must still have reached the scrub, or the test above is \
             passing because nothing happened at all"
        );
    }

    /// Editing a variable's range must move what the scrub accepts. The
    /// control used to clamp against a copy of `decl.range` captured when
    /// it was built, so widening the range did nothing: it went on refusing
    /// anything past the old maximum with nothing on screen to blame.
    ///
    /// The two commits are the same text against two different ranges. The
    /// first pins that clamping still happens at all, so the second cannot
    /// pass merely because the clamp was deleted.
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

    /// The point of moving the control: EVERY declared variable carries
    /// one, with nothing selected. Before this, the scrub lived in the
    /// inspector's Variable section, so it existed only for whatever
    /// variable happened to be inspected -- exactly one, and only after a
    /// click.
    ///
    /// This drives the real `rebuild_variable_list`, scene spawning and
    /// all, rather than asserting about the code that calls it.
    #[test]
    fn every_row_carries_a_scrub_field_with_nothing_selected() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_plugins(bevy::asset::AssetPlugin::default())
            .add_plugins(bevy::scene::ScenePlugin);
        app.init_asset::<ParticlesAsset>();
        app.init_asset::<Image>();
        app.init_asset::<bevy::text::Font>();
        app.init_resource::<DirtyState>();
        app.init_resource::<VariableScrub>();
        app.add_systems(Update, rebuild_variable_list);

        let mut asset = ParticlesAsset::new(
            "t".into(),
            ParticlesDimension::D3,
            Default::default(),
            vec![EmitterData::default()],
            vec![],
            false,
            ParticlesAuthors::default(),
        );
        asset.variables = vec![var("heat"), var("wind"), var("wetness")];
        let handle = {
            let mut assets = app.world_mut().resource_mut::<Assets<ParticlesAsset>>();
            assets.add(asset)
        };
        app.insert_resource(EditorState {
            current_project: Some(handle),
            current_project_path: None,
            inspecting: None,
        });
        app.world_mut().spawn(VariablesSection);

        app.update();
        app.update();

        let mut names: Vec<String> = app
            .world_mut()
            .query::<&VariableScrubRow>()
            .iter(app.world())
            .map(|row| row.name.clone())
            .collect();
        names.sort();
        assert_eq!(
            names,
            vec![
                "heat".to_string(),
                "wetness".to_string(),
                "wind".to_string()
            ],
            "one scrub field per declared variable, addressed by name"
        );
    }

    /// A scrub on a name the asset no longer declares is dropped rather
    /// than stored unclamped -- `retain_declared` would evict it on the
    /// next frame anyway, and storing it flashes a ghost value in between.
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
