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
use crate::viewport::EditorParticlePreview;

pub fn plugin(app: &mut App) {
    app.init_resource::<VariableScrub>()
        .add_observer(on_add_variable)
        .add_observer(on_select_variable_click)
        .add_observer(on_delete_variable_click)
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

#[derive(Event)]
struct AddVariableEvent;

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
fn rebuild_variable_list(
    mut commands: Commands,
    editor_state: Res<EditorState>,
    dirty_state: Res<DirtyState>,
    assets: Res<Assets<ParticlesAsset>>,
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
/// inserting the component if it is not there yet. The slider IS the
/// preview mechanism -- there is no separate preview concept.
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
        // The slider IS the preview mechanism -- there is no separate preview
        // concept -- so this wiring is the feature, not a convenience.
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
}
