//! The Lights outliner section: an add/select/delete list (this module) plus
//! the inspector content for the currently selected one (`inspector::light`).
//!
//! Mirrors `variables.rs` deliberately, not `data_panel.rs`'s
//! Emitters/Colliders list: `DriveTarget::Light { index, .. }` is positional
//! into `ParticlesAsset::lights` in exactly the way Task 18's `VariableId` is
//! positional into `ParticlesAsset::variables`, and `validate_drives` rejects
//! an out-of-range light index at load. So deleting a light must both drop
//! any drive that targeted it AND renumber every drive above it, or the
//! editor would happily write a file it cannot reopen itself.
//! [`remove_light`] is the one place that does this, and every deletion path
//! this list offers routes through it. `data_panel.rs`'s generic
//! Duplicate/Delete/rename flow never sees a light at all -- lights are not
//! `InspectableItem`s there -- but its `Inspectable`-exhaustive matches still
//! carry a `Light` arm apiece (unreachable, kept correct anyway), the same
//! way it already carries one for `Variable`.
//!
//! An add always appends (`asset.lights.push`), which is positionally safe
//! and needs no renumbering -- unlike a hypothetical duplicate-at-arbitrary-
//! index, which this module deliberately does not offer (see Task 18's own
//! note in `data_panel.rs` about why a generic Duplicate path is never safe
//! for a positionally-addressed list).

use bevy::prelude::*;
use bevy_sprinkles::asset::{DriveTarget, LightData};
use bevy_sprinkles::prelude::*;

use crate::state::{DirtyState, EditorState, Inspectable, Inspecting};
use crate::ui::components::data_panel::EditorDataPanel;
use crate::ui::icons::ICON_CLOSE;
use crate::ui::widgets::button::{
    ButtonClickEvent, ButtonProps, ButtonVariant, IconButtonProps, button, icon_button,
};
use crate::ui::widgets::panel_section::{PanelSectionProps, panel_section};
use crate::viewport::RespawnLightsEvent;

pub fn plugin(app: &mut App) {
    app.add_observer(on_add_light)
        .add_observer(on_select_light_click)
        .add_observer(on_delete_light_click)
        .add_systems(Update, (setup_lights_section, rebuild_light_list));
}

/// Removes the light at `index`, keeping the file loadable.
///
/// `DriveTarget::Light`'s index is positional (see the module doc), so
/// removing an entry shifts every index above it down by one. Two things
/// follow, both required: a drive that targeted exactly this light is
/// dropped (it would otherwise point at whatever now occupies this slot, or
/// at nothing), and every surviving light-targeted drive above it is
/// renumbered to match the shift. Skipping either leaves an asset that
/// `validate_drives` rejects on the very next load -- the author sees no
/// error until they reopen the file.
pub fn remove_light(asset: &mut ParticlesAsset, index: usize) {
    if index >= asset.lights.len() {
        return;
    }
    asset.lights.remove(index);
    asset.drives.retain(|d| match &d.target {
        DriveTarget::Light { index: i, .. } => *i as usize != index,
        _ => true,
    });
    for drive in asset.drives.iter_mut() {
        if let DriveTarget::Light { index: i, .. } = &mut drive.target {
            if *i as usize > index {
                *i -= 1;
            }
        }
    }
}

// --- Outliner list ---------------------------------------------------

#[derive(Component)]
struct LightsSection;

#[derive(Component)]
struct LightRowList;

#[derive(Component, Clone, Copy)]
struct LightSelectButton(u8);

#[derive(Component, Clone, Copy)]
struct LightDeleteButton(u8);

#[derive(Event)]
struct AddLightEvent;

fn setup_lights_section(mut commands: Commands, panels: Query<Entity, Added<EditorDataPanel>>) {
    for panel_entity in &panels {
        commands
            .spawn_scene(panel_section(
                PanelSectionProps::new("Lights").with_add_button(),
            ))
            .insert((LightsSection, ChildOf(panel_entity)))
            .observe(on_add_light_click);
    }
}

fn on_add_light_click(_event: On<ButtonClickEvent>, mut commands: Commands) {
    commands.trigger(AddLightEvent);
}

fn on_add_light(
    _event: On<AddLightEvent>,
    mut commands: Commands,
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

    let existing: Vec<&str> = asset.lights.iter().map(|l| l.name.as_str()).collect();
    let name = next_unique_light_name(&existing);
    let new_index = asset.lights.len() as u8;
    // Appending is the only mutation site besides `remove_light` that
    // touches `asset.lights`, and it is positionally safe: nothing shifts,
    // so no drive needs renumbering (see the module doc).
    asset.lights.push(LightData {
        name,
        ..Default::default()
    });

    dirty_state.has_unsaved_changes = true;
    editor_state.inspecting = Some(Inspecting {
        kind: Inspectable::Light,
        index: new_index,
    });
    commands.trigger(RespawnLightsEvent);
}

fn next_unique_light_name(existing: &[&str]) -> String {
    if !existing.contains(&"Light") {
        return "Light".to_string();
    }
    let mut n = 2;
    loop {
        let candidate = format!("Light {n}");
        if !existing.iter().any(|name| *name == candidate) {
            return candidate;
        }
        n += 1;
    }
}

fn on_select_light_click(
    trigger: On<ButtonClickEvent>,
    buttons: Query<&LightSelectButton>,
    mut editor_state: ResMut<EditorState>,
) {
    let Ok(button) = buttons.get(trigger.entity) else {
        return;
    };
    editor_state.inspecting = Some(Inspecting {
        kind: Inspectable::Light,
        index: button.0,
    });
}

fn on_delete_light_click(
    trigger: On<ButtonClickEvent>,
    buttons: Query<&LightDeleteButton>,
    mut commands: Commands,
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
    if index >= asset.lights.len() {
        return;
    }

    remove_light(&mut asset, index);
    dirty_state.has_unsaved_changes = true;
    let new_len = asset.lights.len();

    if let Some(current) = editor_state.inspecting {
        if current.kind == Inspectable::Light {
            if current.index as usize == index {
                editor_state.inspecting = if new_len > 0 {
                    Some(Inspecting {
                        kind: Inspectable::Light,
                        index: 0,
                    })
                } else {
                    None
                };
            } else if current.index as usize > index {
                editor_state.inspecting = Some(Inspecting {
                    kind: Inspectable::Light,
                    index: current.index - 1,
                });
            }
        }
    }

    commands.trigger(RespawnLightsEvent);
}

/// Rebuilds the whole row list whenever anything that could change it
/// changes: the project/selection (`EditorState`) or a committed edit
/// (`DirtyState`, which a rename via the inspector's own Name field also
/// sets). Mirrors `variables::rebuild_variable_list`'s reasoning exactly:
/// simpler and safer than tracking which specific mutation happened, at the
/// cost of a full rebuild on any edit -- fine for a list this small.
fn rebuild_light_list(
    mut commands: Commands,
    editor_state: Res<EditorState>,
    dirty_state: Res<DirtyState>,
    assets: Res<Assets<ParticlesAsset>>,
    section: Query<Entity, With<LightsSection>>,
    new_sections: Query<Entity, Added<LightsSection>>,
    existing_rows: Query<Entity, With<LightRowList>>,
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
            LightRowList,
            Node {
                width: percent(100),
                flex_direction: FlexDirection::Column,
                row_gap: px(6.0),
                ..default()
            },
        ))
        .id();
    commands.entity(section_entity).add_child(list_entity);

    for (i, light) in asset.lights.iter().enumerate() {
        let index = i as u8;
        let is_active = editor_state
            .inspecting
            .map(|ins| ins.kind == Inspectable::Light && ins.index == index)
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
                ButtonProps::new(&light.name)
                    .with_variant(variant)
                    .align_left(),
            ))
            .insert(LightSelectButton(index))
            .id();
        commands.entity(select_wrapper).add_child(select_entity);

        let delete_entity = commands
            .spawn_scene(icon_button(
                IconButtonProps::new(ICON_CLOSE).variant(ButtonVariant::Ghost),
            ))
            .insert(LightDeleteButton(index))
            .id();
        commands.entity(row).add_child(delete_entity);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy_sprinkles::asset::drive::validate_drives;
    use bevy_sprinkles::asset::{Drive, DriveOp, LightProp, VariableDecl, VariableId};

    fn asset_with(lights: Vec<LightData>, drives: Vec<Drive>) -> ParticlesAsset {
        let mut a = ParticlesAsset::new(
            "t".into(),
            ParticlesDimension::D3,
            Default::default(),
            vec![EmitterData::default()],
            vec![],
            false,
            ParticlesAuthors::default(),
        );
        // A drive always names a variable too; declaring one keeps
        // `validate_drives` focused on the light-index question these tests
        // pin, rather than failing on an unrelated undeclared-variable error.
        a.variables = vec![VariableDecl {
            name: "v".into(),
            ..Default::default()
        }];
        a.lights = lights;
        a.drives = drives;
        a
    }

    fn light_drive_on(light_index: u8) -> Drive {
        Drive {
            variable: VariableId(0),
            target: DriveTarget::Light {
                index: light_index,
                prop: LightProp::Intensity,
            },
            curve: CurveTexture::default(),
            output: ParticleRange { min: 0.0, max: 1.0 },
            op: DriveOp::Multiply,
            muted: false,
        }
    }

    #[test]
    fn deleting_a_light_renumbers_the_drives_above_it() {
        let mut asset = asset_with(vec![LightData::default(); 3], vec![light_drive_on(2)]);
        remove_light(&mut asset, 1);
        assert!(matches!(
            asset.drives[0].target,
            DriveTarget::Light { index: 1, .. }
        ));
    }

    #[test]
    fn deleting_a_light_removes_the_drives_that_targeted_it() {
        let mut asset = asset_with(vec![LightData::default()], vec![light_drive_on(0)]);
        remove_light(&mut asset, 0);
        assert!(asset.drives.is_empty());
    }

    /// The direct pin on the property this whole module exists to protect:
    /// the editor must never write a file that fails its own load
    /// validation. Runs the delete through the REAL `validate_drives`,
    /// mirroring `variables.rs`'s own equivalent test.
    #[test]
    fn a_delete_that_renumbers_still_leaves_the_asset_loadable() {
        let mut asset = asset_with(vec![LightData::default(); 3], vec![light_drive_on(2)]);
        remove_light(&mut asset, 1);
        assert!(validate_drives(&asset).is_ok());
    }

    #[test]
    fn deleting_an_out_of_range_index_is_a_no_op() {
        let mut asset = asset_with(vec![LightData::default()], vec![]);
        remove_light(&mut asset, 5);
        assert_eq!(asset.lights.len(), 1);
    }

    #[test]
    fn a_drive_on_a_light_below_the_deleted_one_is_left_untouched() {
        // The shift must be one-directional: a drive whose index is BELOW
        // the deleted light must not move, only ones above it.
        let mut asset = asset_with(
            vec![LightData::default(); 3],
            vec![light_drive_on(0), light_drive_on(2)],
        );
        remove_light(&mut asset, 1);
        assert!(matches!(
            asset.drives[0].target,
            DriveTarget::Light { index: 0, .. }
        ));
        assert!(matches!(
            asset.drives[1].target,
            DriveTarget::Light { index: 1, .. }
        ));
    }
}
