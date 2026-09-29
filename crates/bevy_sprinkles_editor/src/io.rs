use std::env;
use std::fs::File;
use std::io::Write;
use std::path::PathBuf;

use bevy::prelude::*;
use bevy::tasks::IoTaskPool;
use serde::{Deserialize, Serialize};

pub fn plugin(app: &mut App) {
    ensure_data_dirs();
    crate::assets::extract_examples(&examples_dir());
    let editor_data = load_editor_data();
    app.register_type::<EditorSettings>()
        .register_type::<EditorTonemapping>()
        .register_type::<EditorBloom>()
        .register_type::<EditorSmaaPreset>()
        .insert_resource(editor_data);
}

#[derive(Resource, Serialize, Deserialize, Default)]
pub struct EditorData {
    pub cache: EditorCache,
    #[serde(default)]
    pub settings: EditorSettings,
}

#[derive(Serialize, Deserialize, Reflect, Clone)]
pub struct EditorSettings {
    #[serde(default = "default_show_fps")]
    pub show_fps: bool,
    #[serde(default = "default_vsync")]
    pub vsync: bool,
    #[serde(default = "default_tonemapping")]
    pub tonemapping: Option<EditorTonemapping>,
    #[serde(default = "default_bloom")]
    pub bloom: Option<EditorBloom>,
    #[serde(default = "default_anti_aliasing")]
    pub anti_aliasing: Option<EditorSmaaPreset>,
    #[serde(default = "default_show_aabb_gizmos")]
    pub show_aabb_gizmos: bool,
    #[serde(default = "default_frustum_culling")]
    pub frustum_culling: bool,
}

fn default_show_fps() -> bool {
    true
}

fn default_vsync() -> bool {
    true
}

fn default_tonemapping() -> Option<EditorTonemapping> {
    // No display transform by default, so the viewport shows what a renderer
    // WITHOUT one shows: an emissive above 1.0 clips per channel and stays
    // saturated, rather than being rolled off and desaturated toward white by
    // a filmic shoulder. Bevy's own default is `TonyMcMapface`
    // (`bevy_core_pipeline-0.19.0/src/tonemapping/mod.rs:119-158`), which made
    // this editor preview every effect through a curve the host game may not
    // have -- authoring against a shoulder and shipping into a clip is how an
    // emissive that read as soft white here arrived as blown-out orange there.
    // A host that DOES tonemap can turn it back on; the honest default is the
    // one that adds nothing.
    None
}

fn default_bloom() -> Option<EditorBloom> {
    // Off for the same reason as `default_tonemapping`: bloom spreads a bright
    // core into a halo the host may never draw, so an effect tuned to look
    // right with it is over-bright without it.
    None
}

fn default_anti_aliasing() -> Option<EditorSmaaPreset> {
    Some(EditorSmaaPreset::High)
}

fn default_show_aabb_gizmos() -> bool {
    true
}

fn default_frustum_culling() -> bool {
    true
}

impl Default for EditorSettings {
    fn default() -> Self {
        Self {
            show_fps: default_show_fps(),
            vsync: default_vsync(),
            tonemapping: default_tonemapping(),
            bloom: default_bloom(),
            anti_aliasing: default_anti_aliasing(),
            show_aabb_gizmos: default_show_aabb_gizmos(),
            frustum_culling: default_frustum_culling(),
        }
    }
}

#[derive(Serialize, Deserialize, Reflect, Clone, Default, PartialEq)]
pub enum EditorTonemapping {
    #[default]
    Reinhard,
    ReinhardLuminance,
    AcesFitted,
    AgX,
    SomewhatBoringDisplayTransform,
    TonyMcMapface,
    BlenderFilmic,
}

#[derive(Serialize, Deserialize, Reflect, Clone, Default, PartialEq)]
pub enum EditorBloom {
    #[default]
    Natural,
    Anamorphic,
    OldSchool,
    ScreenBlur,
}

#[derive(Serialize, Deserialize, Reflect, Clone, Default, PartialEq)]
pub enum EditorSmaaPreset {
    Low,
    Medium,
    #[default]
    High,
    Ultra,
}

#[derive(Serialize, Deserialize, Default)]
pub struct EditorCache {
    pub last_opened_project: Option<String>,
    pub recent_projects: Vec<String>,
}

impl EditorCache {
    const MAX_RECENT_PROJECTS: usize = 10;

    pub fn add_recent_project(&mut self, path: String) {
        let new_canonical = canonicalize_path(&path);
        self.recent_projects
            .retain(|p| canonicalize_path(p) != new_canonical);
        self.recent_projects.insert(0, path.clone());
        self.recent_projects.truncate(Self::MAX_RECENT_PROJECTS);
        self.last_opened_project = Some(path);
    }

    pub fn remove_recent_project(&mut self, path: &str) {
        let canonical = canonicalize_path(path);
        self.recent_projects
            .retain(|p| canonicalize_path(p) != canonical);
    }
}

pub fn data_dir() -> PathBuf {
    #[cfg(unix)]
    let home = env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    #[cfg(not(unix))]
    let home = env::var_os("USERPROFILE")
        .map(PathBuf::from)
        .unwrap_or_default();
    home.join(".sprinkles")
}

pub fn projects_dir() -> PathBuf {
    data_dir().join("projects")
}

pub fn examples_dir() -> PathBuf {
    data_dir().join("examples")
}

pub fn is_example_path(path: &std::path::Path) -> bool {
    path.starts_with(examples_dir())
}

pub fn working_dir() -> PathBuf {
    env::current_dir().unwrap_or_default()
}

fn ensure_data_dirs() {
    let _ = std::fs::create_dir_all(projects_dir());
    let _ = std::fs::create_dir_all(examples_dir());
}

fn canonicalize_path(path: &str) -> PathBuf {
    let stripped = path
        .strip_prefix("./")
        .or_else(|| path.strip_prefix(".\\"))
        .unwrap_or(path);
    let path_buf = project_path(stripped);
    path_buf.canonicalize().unwrap_or(path_buf)
}

fn editor_data_path() -> PathBuf {
    data_dir().join("editor.ron")
}

pub fn project_path(relative_path: &str) -> PathBuf {
    if relative_path.starts_with("~/") {
        #[cfg(unix)]
        let home = env::var_os("HOME").map(PathBuf::from);
        #[cfg(not(unix))]
        let home = env::var_os("USERPROFILE").map(PathBuf::from);
        home.map(|h| h.join(&relative_path[2..]))
            .unwrap_or_else(|| PathBuf::from(relative_path))
    } else {
        PathBuf::from(relative_path)
    }
}

pub fn load_editor_data() -> EditorData {
    let path = editor_data_path();
    if path.exists() {
        std::fs::read_to_string(&path)
            .ok()
            .and_then(|contents| ron::from_str(&contents).ok())
            .unwrap_or_default()
    } else {
        EditorData::default()
    }
}

pub fn save_editor_data(data: &EditorData) {
    let path = editor_data_path();
    let Ok(contents) = ron::ser::to_string_pretty(data, ron::ser::PrettyConfig::default()) else {
        return;
    };

    IoTaskPool::get()
        .spawn(async move {
            let mut file = File::create(&path).expect("failed to create editor data file");
            file.write_all(contents.as_bytes())
                .expect("failed to write editor data");
        })
        .detach();
}

#[cfg(test)]
mod default_viewport_tests {
    use super::*;

    /// The viewport must add no display transform of its own.
    ///
    /// This is an authoring-honesty guarantee, not a preference: an effect is
    /// tuned by eye in this viewport and then rendered by a host that may apply
    /// no tonemapping and no bloom at all. Under a filmic shoulder a bright
    /// emissive rolls off and desaturates toward white; under none it clips per
    /// channel and stays saturated. Authoring against the former and shipping
    /// into the latter is how the same asset reads as soft white here and
    /// blown-out orange there -- a real report, and the reason these defaults
    /// moved off bevy's own.
    ///
    /// A host that genuinely tonemaps can switch both back on; the default is
    /// the one that shows the author their own values rather than a curve's
    /// opinion of them.
    #[test]
    fn the_viewport_adds_no_tonemapping_or_bloom_by_default() {
        assert!(
            default_tonemapping().is_none(),
            "a default display transform makes every preview a lie for a host without one"
        );
        assert!(
            default_bloom().is_none(),
            "default bloom makes an over-bright effect look correct while authoring"
        );
    }

    /// An explicit setting in `editor.ron` must still win, or a user who wants
    /// a filmic preview silently loses it on the next launch.
    #[test]
    fn an_explicitly_authored_setting_still_overrides_the_default() {
        let ron = r#"(
            show_fps: true,
            vsync: true,
            tonemapping: Some(TonyMcMapface),
            bloom: Some(Natural),
        )"#;
        let parsed: EditorSettings = ron::from_str(ron).expect("settings parse");
        assert!(
            matches!(parsed.tonemapping, Some(EditorTonemapping::TonyMcMapface)),
            "serde(default) must only fill an ABSENT field, never replace an authored one"
        );
        assert!(matches!(parsed.bloom, Some(EditorBloom::Natural)));
    }
}
