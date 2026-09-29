use std::fs::File;
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use bevy::prelude::*;
use bevy::tasks::IoTaskPool;
use bevy_sprinkles::prelude::*;
use inflector::Inflector;

use crate::io::{EditorData, is_example_path, project_path, projects_dir, save_editor_data};
use crate::state::{DirtyState, EditorState, Inspectable, Inspecting};
use crate::ui::components::toasts::ToastEvent;
use crate::utils::{MAX_DISPLAY_PATH_LEN, simplify_path, truncate_path};

pub fn plugin(app: &mut App) {
    app.add_observer(on_open_project_event)
        .add_observer(on_browse_open_project_event)
        .add_observer(on_save_project_event)
        .add_observer(on_save_project_as_event)
        .add_systems(
            Update,
            (
                handle_save_keyboard_shortcut,
                poll_browse_open_result,
                poll_save_as_result,
                poll_save_result,
            ),
        );
}

#[derive(Event)]
pub struct OpenProjectEvent(pub String);

#[derive(Event)]
pub struct BrowseOpenProjectEvent;

#[derive(Event)]
pub struct SaveProjectEvent;

#[derive(Event)]
pub struct SaveProjectAsEvent;

#[derive(Resource, Clone)]
pub struct BrowseOpenResult(pub Arc<Mutex<Option<PathBuf>>>);

#[derive(Resource, Clone)]
pub struct SaveAsResult(pub Arc<Mutex<Option<PathBuf>>>);

#[derive(Clone)]
pub enum SaveResultStatus {
    Success(String),
    SerializationError,
    WriteError(String),
    CreateError,
    /// The asset broke an invariant `validate_drives` enforces at LOAD, so
    /// writing it would produce a file the editor could not reopen. Carries
    /// `validate_drives`' own message, which names the offender.
    Invalid(String),
}

#[derive(Resource, Clone)]
pub struct SaveResult(pub Arc<Mutex<Option<SaveResultStatus>>>);

pub(crate) enum LoadProjectError {
    /// The file could not be read from disk.
    Read(std::io::Error),
    /// The file contents could not be parsed as a valid project.
    Parse,
    /// The file has an unrecognized format version.
    UnknownVersion,
}

pub(crate) fn load_project_from_path(
    path: &std::path::Path,
) -> Result<bevy_sprinkles::asset::versions::MigrationResult, LoadProjectError> {
    let contents = std::fs::read_to_string(path).map_err(|err| {
        error!("Failed to read project file: {path:?}");
        error!("{err}");
        LoadProjectError::Read(err)
    })?;

    bevy_sprinkles::asset::versions::migrate_str(&contents).map_err(|err| {
        error!("Failed to parse project file: {path:?}");
        error!("{err}");
        match err {
            bevy_sprinkles::asset::versions::MigrationError::UnknownVersion(_) => {
                LoadProjectError::UnknownVersion
            }
            _ => LoadProjectError::Parse,
        }
    })
}

fn on_open_project_event(
    event: On<OpenProjectEvent>,
    mut editor_state: ResMut<EditorState>,
    mut editor_data: ResMut<EditorData>,
    mut assets: ResMut<Assets<ParticlesAsset>>,
    mut dirty_state: ResMut<DirtyState>,
    mut commands: Commands,
) {
    let location = &event.0;
    let path = project_path(location);
    let is_example = is_example_path(&path);

    let result = match load_project_from_path(&path) {
        Ok(result) => result,
        Err(err) => {
            let display = truncate_path(location, MAX_DISPLAY_PATH_LEN);
            let message = match &err {
                LoadProjectError::Read(io_err) => match io_err.kind() {
                    std::io::ErrorKind::NotFound => {
                        format!("Project file not found: \"{display}\"")
                    }
                    std::io::ErrorKind::PermissionDenied => {
                        format!("Permission denied: \"{display}\"")
                    }
                    _ => format!("Could not read project file: \"{display}\""),
                },
                LoadProjectError::Parse => {
                    format!("Project \"{display}\" is corrupted or invalid")
                }
                LoadProjectError::UnknownVersion => {
                    format!(
                        "Project \"{display}\" has an unknown version. You may need to update Sprinkles."
                    )
                }
            };
            commands.trigger(ToastEvent::error(message));
            return;
        }
    };

    let asset = result.asset;

    let filename = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| location.clone());

    if result.was_migrated {
        let current = bevy_sprinkles::asset::versions::current_format_version();
        dirty_state.has_unsaved_changes = true;
        commands.trigger(ToastEvent::success(format!(
            "Project \"{filename}\" will be updated to {current} on the next save"
        )));
    }

    let has_emitters = !asset.emitters.is_empty();
    let handle = assets.add(asset);
    editor_state.open_project(handle, path, &mut dirty_state);
    editor_state.inspecting = if has_emitters {
        Some(Inspecting {
            kind: Inspectable::Emitter,
            index: 0,
        })
    } else {
        None
    };

    if !is_example {
        editor_data.cache.add_recent_project(location.clone());
        save_editor_data(&editor_data);
    }
}

fn on_browse_open_project_event(_event: On<BrowseOpenProjectEvent>, mut commands: Commands) {
    let projects_dir = projects_dir();

    let path_result = Arc::new(Mutex::new(None));
    let path_result_clone = path_result.clone();

    let task = rfd::AsyncFileDialog::new()
        .set_title("Open Project")
        .set_directory(&projects_dir)
        .add_filter("RON files", &["ron"])
        .pick_file();

    IoTaskPool::get()
        .spawn(async move {
            if let Some(file_handle) = task.await {
                let path = file_handle.path().to_path_buf();
                if let Ok(mut guard) = path_result_clone.lock() {
                    *guard = Some(path);
                }
            }
        })
        .detach();

    commands.insert_resource(BrowseOpenResult(path_result));
}

fn poll_browse_open_result(result: Option<Res<BrowseOpenResult>>, mut commands: Commands) {
    let Some(result) = result else {
        return;
    };

    let path = {
        let Ok(mut guard) = result.0.lock() else {
            return;
        };
        guard.take()
    };

    if let Some(path) = path {
        commands.trigger(OpenProjectEvent(simplify_path(&path)));
        commands.remove_resource::<BrowseOpenResult>();
    }
}

/// The write barrier: validates, then serializes, returning the bytes a save
/// may put on disk.
///
/// `validate_drives` is the loader's own gate (`asset::versions::finish` runs
/// it on every successful parse), and until this existed nothing ran it on the
/// way OUT. Two one-keystroke paths through the Variables Name field produced
/// a file the editor could not reopen: clearing the name, and renaming to a
/// duplicate. Both are legal intermediate states while editing, which is why
/// the editor does not forbid them -- it refuses to WRITE them, and says why.
///
/// This is a write barrier, not an edit check. It also closes the whole class
/// of positional-renumbering defects the delete/duplicate helpers each guard
/// individually (`data_panel::remove_emitter` and friends), including any
/// instance nobody has found: no path through this editor can serialize an
/// asset the loader would reject, wherever the breakage came from.
pub(crate) fn prepare_save_contents(
    asset: &bevy_sprinkles::asset::ParticlesAsset,
) -> Result<String, SaveResultStatus> {
    bevy_sprinkles::asset::drive::validate_drives(asset)
        .map_err(SaveResultStatus::Invalid)?;
    ron::ser::to_string_pretty(asset, ron::ser::PrettyConfig::default())
        .map_err(|_| SaveResultStatus::SerializationError)
}

/// Writes `asset` to `path`, or refuses and reports why.
///
/// Returns whether a write was actually dispatched, so a caller can leave the
/// project dirty on a refusal instead of marking it saved.
pub fn save_project_to_path(
    path: PathBuf,
    asset: &bevy_sprinkles::asset::ParticlesAsset,
    result: Arc<Mutex<Option<SaveResultStatus>>>,
) -> bool {
    let contents = match prepare_save_contents(asset) {
        Ok(contents) => contents,
        Err(status) => {
            if let Ok(mut guard) = result.lock() {
                *guard = Some(status);
            }
            return false;
        }
    };

    IoTaskPool::get()
        .spawn(async move {
            let status = match File::create(&path) {
                Ok(mut file) => {
                    let filename = path
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_else(|| "".to_string());
                    if file.write_all(contents.as_bytes()).is_ok() {
                        SaveResultStatus::Success(filename)
                    } else {
                        SaveResultStatus::WriteError(filename)
                    }
                }
                Err(_) => SaveResultStatus::CreateError,
            };
            if let Ok(mut guard) = result.lock() {
                *guard = Some(status);
            }
        })
        .detach();
    true
}

fn on_save_project_event(
    _event: On<SaveProjectEvent>,
    editor_state: Res<EditorState>,
    assets: Res<Assets<ParticlesAsset>>,
    mut dirty_state: ResMut<DirtyState>,
    mut commands: Commands,
) {
    let Some(handle) = &editor_state.current_project else {
        return;
    };
    let Some(asset) = assets.get(handle) else {
        return;
    };

    if let Some(path) = &editor_state.current_project_path {
        let result = Arc::new(Mutex::new(None));
        let wrote = save_project_to_path(path.clone(), asset, result.clone());
        commands.insert_resource(SaveResult(result));
        // A refused save leaves the project dirty. Clearing the flag on a
        // refusal would tell the author their work is on disk when the file
        // was never touched -- and the next thing many authors do after Ctrl+S
        // is close the window.
        if wrote {
            dirty_state.has_unsaved_changes = false;
        }
    } else {
        commands.trigger(SaveProjectAsEvent);
    }
}

fn on_save_project_as_event(
    _event: On<SaveProjectAsEvent>,
    editor_state: Res<EditorState>,
    assets: Res<Assets<ParticlesAsset>>,
    mut commands: Commands,
) {
    let Some(handle) = &editor_state.current_project else {
        return;
    };
    let Some(asset) = assets.get(handle) else {
        return;
    };

    // Checked BEFORE the file dialog opens, not only inside the write: asking
    // an author where to put a file and then refusing to write it is a worse
    // way to deliver the same refusal. `save_project_to_path` still re-checks
    // -- it is the barrier, this is only the early exit.
    if let Err(SaveResultStatus::Invalid(message)) = prepare_save_contents(asset) {
        commands.trigger(ToastEvent::error(format!("Cannot save: {message}")));
        return;
    }

    let projects_dir = projects_dir();
    let default_name = format!("{}.ron", asset.name.to_kebab_case());
    let asset_clone = asset.clone();

    let path_result = Arc::new(Mutex::new(None));
    let path_result_clone = path_result.clone();

    let save_result = Arc::new(Mutex::new(None));
    let save_result_clone = save_result.clone();

    let task = rfd::AsyncFileDialog::new()
        .set_title("Save Project As")
        .set_directory(&projects_dir)
        .set_file_name(&default_name)
        .add_filter("RON files", &["ron"])
        .save_file();

    IoTaskPool::get()
        .spawn(async move {
            if let Some(file_handle) = task.await {
                let path = file_handle.path().to_path_buf();
                save_project_to_path(path.clone(), &asset_clone, save_result_clone);
                if let Ok(mut guard) = path_result_clone.lock() {
                    *guard = Some(path);
                }
            }
        })
        .detach();

    commands.insert_resource(SaveAsResult(path_result));
    commands.insert_resource(SaveResult(save_result));
}

fn poll_save_as_result(
    result: Option<Res<SaveAsResult>>,
    mut editor_state: ResMut<EditorState>,
    mut editor_data: ResMut<EditorData>,
    mut dirty_state: ResMut<DirtyState>,
    mut commands: Commands,
) {
    let Some(result) = result else {
        return;
    };

    let path = {
        let Ok(mut guard) = result.0.lock() else {
            return;
        };
        guard.take()
    };

    if let Some(path) = path {
        editor_state.current_project_path = Some(path.clone());

        editor_data.cache.add_recent_project(simplify_path(&path));
        save_editor_data(&editor_data);
        dirty_state.has_unsaved_changes = false;
        commands.remove_resource::<SaveAsResult>();
    }
}

fn poll_save_result(result: Option<Res<SaveResult>>, mut commands: Commands) {
    let Some(result) = result else {
        return;
    };

    let status = {
        let Ok(mut guard) = result.0.lock() else {
            return;
        };
        guard.take()
    };

    if let Some(status) = status {
        match status {
            SaveResultStatus::Success(filename) => {
                commands.trigger(ToastEvent::success(format!("Saved \"{filename}\"")));
            }
            SaveResultStatus::SerializationError => {
                commands.trigger(ToastEvent::error("Cannot save project with invalid data"));
            }
            SaveResultStatus::WriteError(filename) => {
                commands.trigger(ToastEvent::error(format!(
                    "Failed to write to \"{filename}\""
                )));
            }
            SaveResultStatus::CreateError => {
                commands.trigger(ToastEvent::error("Failed to create project file"));
            }
            // `validate_drives`' own wording, verbatim: it names the offender
            // ("duplicate variable name \"heat\""), which a generic "invalid
            // project" would throw away.
            SaveResultStatus::Invalid(message) => {
                commands.trigger(ToastEvent::error(format!("Cannot save: {message}")));
            }
        }
        commands.remove_resource::<SaveResult>();
    }
}

fn handle_save_keyboard_shortcut(keyboard: Res<ButtonInput<KeyCode>>, mut commands: Commands) {
    let ctrl_or_cmd = keyboard.pressed(KeyCode::SuperLeft)
        || keyboard.pressed(KeyCode::SuperRight)
        || keyboard.pressed(KeyCode::ControlLeft)
        || keyboard.pressed(KeyCode::ControlRight);

    if ctrl_or_cmd && keyboard.just_pressed(KeyCode::KeyS) {
        commands.trigger(SaveProjectEvent);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy_sprinkles::asset::{
        EmitterData, ParticlesAsset, ParticlesAuthors, ParticlesDimension, VariableDecl,
    };

    fn asset_with(variables: Vec<VariableDecl>) -> ParticlesAsset {
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
        a
    }

    fn named(name: &str) -> VariableDecl {
        VariableDecl {
            name: name.into(),
            ..Default::default()
        }
    }

    /// One keystroke in the Variables Name field: select all, delete.
    #[test]
    fn a_variable_with_no_name_refuses_to_save() {
        let asset = asset_with(vec![named("")]);
        match prepare_save_contents(&asset) {
            Err(SaveResultStatus::Invalid(message)) => {
                assert!(message.contains("empty"), "must say what is wrong: {message}");
            }
            Err(_) => panic!("must be refused as Invalid, not as a serialization failure"),
            Ok(_) => panic!("the loader rejects this file, so the editor must not write it"),
        }
    }

    /// The other one-keystroke path: rename a variable onto its neighbour.
    #[test]
    fn two_variables_sharing_a_name_refuse_to_save() {
        let asset = asset_with(vec![named("heat"), named("heat")]);
        match prepare_save_contents(&asset) {
            Err(SaveResultStatus::Invalid(message)) => {
                assert!(
                    message.contains("heat"),
                    "must name the offender so the author can find it: {message}"
                );
            }
            Err(_) => panic!("must be refused as Invalid, not as a serialization failure"),
            Ok(_) => panic!("the loader rejects this file, so the editor must not write it"),
        }
    }

    /// The barrier must not be a wall. Round-trips the bytes back through the
    /// REAL loader (`migrate_str`, which runs `validate_drives` itself), so
    /// this pins the actual contract -- "what the editor writes, the editor
    /// reopens" -- rather than merely that `Ok` came back.
    #[test]
    fn a_valid_asset_still_saves_and_the_bytes_reload() {
        let asset = asset_with(vec![named("heat")]);
        let contents = prepare_save_contents(&asset).unwrap_or_else(|_| {
            panic!("a valid asset must still serialize");
        });
        bevy_sprinkles::asset::versions::migrate_str(&contents)
            .expect("the loader must accept what the editor just wrote");
    }

    /// The refusal reaches the real write path, and nothing lands on disk.
    /// `IoTaskPool` is deliberately never initialised here: if the barrier
    /// ever stopped short-circuiting, `save_project_to_path` would reach
    /// `IoTaskPool::get()` and panic, which is a louder failure than a missing
    /// file and a fine second signal.
    #[test]
    fn a_refused_save_writes_no_file_and_reports_why() {
        let dir = std::env::temp_dir().join(format!(
            "sprinkles-save-barrier-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("scratch dir");
        let path = dir.join("refused.ron");
        let _ = std::fs::remove_file(&path);

        let asset = asset_with(vec![named("heat"), named("heat")]);
        let result = Arc::new(Mutex::new(None));
        let wrote = save_project_to_path(path.clone(), &asset, result.clone());

        assert!(!wrote, "a refused save must report that it did not write");
        assert!(!path.exists(), "nothing may reach disk");
        assert!(
            matches!(
                result.lock().unwrap().as_ref(),
                Some(SaveResultStatus::Invalid(_))
            ),
            "the refusal must be surfaced, not swallowed"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
