//! What the Build form holds, remembered per area across restarts.
//!
//! Options are not a preference, they are part of what an area *is*: rhone-alpes is built at a
//! different maxzoom from europe, with a different clip shape and a different set of steps.
//! Keeping them only in the form meant retyping all of it after every launch, and - worse -
//! meant a rebuild silently using whatever the default preset said rather than what the area was
//! last built with.
//!
//! Stored beside `settings.json` rather than inside it: this grows one entry per area and is
//! written on every keystroke in the form, while settings.json is edited by hand.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::steps::StepId;

/// One area's form state, in the shape the front end sends it.
///
/// camelCase because the only writers are the UI and this file; every field defaults, so an
/// entry written before a field existed still loads.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct AreaBuildConfig {
    /// The steps ticked by hand. Dependencies are re-derived on load rather than stored, so a
    /// change to the graph reaches a remembered selection.
    pub steps: Vec<StepId>,
    /// Per step, the option values the form holds. Only keys that were actually set.
    pub values: BTreeMap<StepId, BTreeMap<String, serde_json::Value>>,
    /// Per step, the free-text tool arguments the form has no field for.
    pub extra_args: BTreeMap<StepId, String>,
    /// Steps armed to rebuild even though their output is on disk.
    ///
    /// Deliberately *not* restored as armed by the front end: see `force_all`.
    pub force: Vec<StepId>,
    pub force_all: bool,
    /// `bundled` or `yaml`.
    pub schema_mode: Option<String>,
    pub schema_yaml: Option<String>,
    /// An explicitly chosen planetiler jar. Empty means "whatever the app finds".
    pub jar: Option<String>,
}

/// Every area's form state, keyed by area name.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct BuildConfigStore {
    pub areas: BTreeMap<String, AreaBuildConfig>,
}

impl BuildConfigStore {
    /// Load from disk. A missing file is an empty store, and so is an unreadable one: losing
    /// remembered options is annoying, refusing to open the Build tab because of them is worse.
    pub fn load_or_default(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(text) => serde_json::from_str(&text).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, serde_json::to_string_pretty(self)?)
            .with_context(|| format!("writing {}", path.display()))
    }

    pub fn get(&self, area: &str) -> Option<&AreaBuildConfig> {
        self.areas.get(area)
    }

    pub fn set(&mut self, area: &str, config: AreaBuildConfig) {
        self.areas.insert(area.to_string(), config);
    }

    /// Forget one area, for when its output is deleted.
    pub fn remove(&mut self, area: &str) -> bool {
        self.areas.remove(area).is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> AreaBuildConfig {
        AreaBuildConfig {
            steps: vec![StepId::Basemap, StepId::TerrainRgb],
            values: BTreeMap::from([(
                StepId::TerrainRgb,
                BTreeMap::from([("maxzoom".to_string(), serde_json::json!(12))]),
            )]),
            extra_args: BTreeMap::from([(StepId::Basemap, "--max-point-buffer=4".to_string())]),
            force: vec![],
            force_all: false,
            schema_mode: Some("bundled".into()),
            schema_yaml: None,
            jar: None,
        }
    }

    #[test]
    fn round_trips_through_disk() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("build-config.json");
        let mut store = BuildConfigStore::default();
        store.set("rhone-alpes", sample());
        store.save(&path).unwrap();
        assert_eq!(BuildConfigStore::load_or_default(&path), store);
    }

    /// Two areas are two independent sets of options - the whole point of the file.
    #[test]
    fn areas_do_not_share_values() {
        let mut store = BuildConfigStore::default();
        store.set("rhone-alpes", sample());
        store.set("europe", AreaBuildConfig { steps: vec![StepId::DownloadOsm], ..sample() });
        assert_eq!(store.get("rhone-alpes").unwrap().steps.len(), 2);
        assert_eq!(store.get("europe").unwrap().steps, vec![StepId::DownloadOsm]);
        assert!(store.get("nowhere").is_none());
    }

    /// A file from an older build, or a corrupted one, must not stop the Build tab opening.
    #[test]
    fn unreadable_or_partial_files_load_as_empty_or_partial() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("build-config.json");
        std::fs::write(&path, "not json at all").unwrap();
        assert_eq!(BuildConfigStore::load_or_default(&path), BuildConfigStore::default());

        std::fs::write(&path, r#"{"areas":{"a":{"steps":["basemap"]}}}"#).unwrap();
        let store = BuildConfigStore::load_or_default(&path);
        assert_eq!(store.get("a").unwrap().steps, vec![StepId::Basemap]);
        assert!(store.get("a").unwrap().values.is_empty());
    }
}
