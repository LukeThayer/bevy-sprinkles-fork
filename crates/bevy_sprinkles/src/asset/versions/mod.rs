mod v0_1;

use serde::Deserialize;
use thiserror::Error;

use super::ParticlesAsset;

const CURRENT_FORMAT_VERSION: &str = "0.4";

/// Returns the current asset format version string.
pub fn current_format_version() -> &'static str {
    CURRENT_FORMAT_VERSION
}

#[derive(Deserialize)]
struct VersionProbe {
    sprinkles_version: String,
}

/// Errors that can occur during asset version migration.
#[derive(Debug, Error)]
pub enum MigrationError {
    /// The asset file contained invalid RON syntax.
    #[error("Could not parse RON: {0}")]
    Ron(#[from] ron::error::SpannedError),
    /// The asset file has an unrecognized format version.
    #[error("Unknown sprinkles_version \"{0}\". You may need a newer version of Sprinkles.")]
    UnknownVersion(String),
    /// The asset parsed but broke an invariant that would make it render
    /// nothing with no error at runtime.
    #[error("{0}")]
    Invalid(String),
}

/// The result of a [`migrate`] call.
#[derive(Debug)]
pub struct MigrationResult {
    /// The particle system asset in the current format version.
    pub asset: ParticlesAsset,
    /// Whether the asset was migrated from an older version.
    pub was_migrated: bool,
}

/// Runs [`validate_drives`](crate::asset::drive::validate_drives) on every
/// successful parse, STAMPS the current format version, and packages the
/// result -- so no arm of [`migrate`] can forget either step before handing an
/// asset back to the loader.
///
/// The stamp matters because a migrated asset gets re-serialized the next time
/// the editor saves it. Without it, a `"0.3"` file edited and saved went back
/// to disk as 0.4 content still LABELLED `"0.3"`, and the next load ran the
/// 0.3 arm over it. That is harmless only for as long as the 0.3 and 0.4 arms
/// stay identical, which is exactly why nobody noticed -- and it stops being
/// harmless the moment either arm gains a real transformation.
///
/// `sprinkles_version` is private to `super`; `versions` is a child module, so
/// the write is reachable here and nowhere outside `asset`.
fn finish(
    mut asset: ParticlesAsset,
    was_migrated: bool,
) -> Result<MigrationResult, MigrationError> {
    crate::asset::drive::validate_drives(&asset).map_err(MigrationError::Invalid)?;
    asset.sprinkles_version = current_format_version().to_string();
    Ok(MigrationResult { asset, was_migrated })
}

/// Migrates a RON-encoded particle system asset to the current format version.
pub fn migrate(bytes: &[u8]) -> Result<MigrationResult, MigrationError> {
    let probe: VersionProbe = ron::de::from_bytes(bytes)?;
    let current = current_format_version();

    match probe.sprinkles_version.as_str() {
        v if v == current => {
            let asset: ParticlesAsset = ron::de::from_bytes(bytes)?;
            finish(asset, false)
        }
        "0.3" => {
            let asset: ParticlesAsset = ron::de::from_bytes(bytes)?;
            finish(asset, true)
        }
        "0.2" => {
            let asset: ParticlesAsset = ron::de::from_bytes(bytes)?;
            finish(asset, true)
        }
        "0.1" => {
            let old: v0_1::ParticlesAsset = ron::de::from_bytes(bytes)?;
            let asset: ParticlesAsset = old.into();
            finish(asset, true)
        }
        unknown => Err(MigrationError::UnknownVersion(unknown.to_string())),
    }
}

/// Migrates a RON-encoded particle system asset from a string.
pub fn migrate_str(ron: &str) -> Result<MigrationResult, MigrationError> {
    migrate(ron.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_v0_3_file_without_the_new_fields_still_loads() {
        let ron = r#"(
            sprinkles_version: "0.3",
            name: "old",
            dimension: D3,
            emitters: [],
        )"#;
        let r = migrate_str(ron).expect("a 0.3 file must keep loading");
        assert!(r.was_migrated);
        assert!(r.asset.variables.is_empty());
        assert!(r.asset.drives.is_empty());
        assert!(r.asset.lights.is_empty());
    }

    #[test]
    fn a_file_whose_drive_names_a_missing_variable_fails_to_load() {
        // The brief's inline RON literal for a `Drive` (`curve: (points: [])`)
        // does not match `CurveTexture`'s real serialized shape (it has
        // `name`/`x`/`y`/`z`, not a bare `points` list — confirmed by running
        // this test with that literal: `MissingStructField { field: "x" }`).
        // Building the asset in Rust and serializing it with `ron::ser` keeps
        // the test honest about what a real save produces.
        use crate::asset::{
            CurveTexture, Drive, DriveOp, DriveTarget, EmitterData, EmitterProp,
            ParticlesAuthors, ParticlesDimension, Range, VariableId,
        };

        let mut asset = ParticlesAsset::new(
            "broken".into(),
            ParticlesDimension::D3,
            Default::default(),
            vec![EmitterData::default()],
            vec![],
            false,
            ParticlesAuthors::default(),
        );
        asset.drives = vec![Drive {
            variable: VariableId(0),
            target: DriveTarget::Emitter { index: 0, prop: EmitterProp::Tint },
            curve: CurveTexture::default(),
            output: Range { min: 0.0, max: 1.0 },
            op: DriveOp::Multiply,
            muted: false,
        }];
        let ron = ron::ser::to_string(&asset).expect("serialize");

        let err = migrate_str(&ron).expect_err("must not load");
        assert!(matches!(err, MigrationError::Invalid(_)), "got {err:?}");
    }

    /// I7: the editor re-serializes whatever `migrate` handed it, so an
    /// unstamped migration writes new-format content under an old version
    /// label. Round-trips through `ron::ser` rather than reading the field
    /// directly, because the FILE's label is the thing that misleads the next
    /// load -- reading the in-memory field would pass even if serialization
    /// dropped it.
    #[test]
    fn a_migrated_asset_is_reserialized_carrying_the_current_version() {
        let ron = r#"(
            sprinkles_version: "0.3",
            name: "old",
            dimension: D3,
            emitters: [],
        )"#;
        let r = migrate_str(ron).expect("a 0.3 file must keep loading");
        assert!(r.was_migrated);

        let written = ron::ser::to_string(&r.asset).expect("serialize");
        let probe: VersionProbe = ron::de::from_bytes(written.as_bytes()).expect("reprobe");
        assert_eq!(
            probe.sprinkles_version,
            current_format_version(),
            "a migrated asset saved back out must not claim the version it came from"
        );
    }

    /// The stamp must not be limited to the migrating arms: an asset already
    /// at the current version passes through `finish` too, and must come out
    /// labelled the same way it went in.
    #[test]
    fn a_current_version_file_keeps_its_version() {
        let ron = format!(
            r#"(
                sprinkles_version: "{}",
                name: "new",
                dimension: D3,
                emitters: [],
            )"#,
            current_format_version()
        );
        let r = migrate_str(&ron).expect("a current-version file must load");
        assert!(!r.was_migrated);

        let written = ron::ser::to_string(&r.asset).expect("serialize");
        let probe: VersionProbe = ron::de::from_bytes(written.as_bytes()).expect("reprobe");
        assert_eq!(probe.sprinkles_version, current_format_version());
    }
}
