//! Persisted configuration.
//!
//! Everything the pipeline touches is a setting, because every path in this repo has moved at
//! least once: output lives in `alpimaps_mbtiles/`, sources in `data/sources`, elevation in
//! `elevation_tiles/`, and the planetiler jar version has drifted three times (`env.sh` still
//! says 0.5 and 0.7 while the real jar is 0.10.3). Hard-coding any of them would bake in a lie.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// One buildable area. `name` is both the output subdirectory and planetiler's `--area`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AreaConfig {
    pub name: String,
    /// `--polygon`, clipping the build to a shape rather than the extract's bbox.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub poly: Option<PathBuf>,
}

/// Every field carries `#[serde(default)]` so a settings file written by an older build still
/// loads after new fields land - the alternative is an app that refuses to start after upgrade.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Root of the data-generator checkout; the default for every other path hangs off it.
    pub repo_root: PathBuf,
    /// Where areas and their artifacts live. One subdirectory per area.
    pub output_root: PathBuf,
    /// Planetiler's source downloads (`data/sources`).
    pub data_dir: PathBuf,
    /// Parent for per-run temp dirs. Never shared between two concurrent runs.
    pub tmp_dir: PathBuf,
    pub elevation_tiles_dir: PathBuf,
    pub sources_json: PathBuf,
    /// Override for JRE discovery; `None` means probe `$JAVA_HOME` then `$PATH`.
    pub java_home: Option<PathBuf>,
    pub planetiler_jar: Option<PathBuf>,
    pub valhalla_bin_dir: Option<PathBuf>,
    pub heap_mb: u32,
    /// Passed as `--loginterval`. Planetiler's own default is `10s`, which yields about six
    /// progress lines for a one-minute build - far too coarse to drive a progress bar.
    pub log_interval: String,
    pub areas: Vec<AreaConfig>,
}

impl Default for Settings {
    fn default() -> Self {
        Self::for_repo(PathBuf::from("."))
    }
}

impl Settings {
    /// Defaults matching this repository's actual layout.
    pub fn for_repo(repo_root: PathBuf) -> Self {
        Self {
            output_root: repo_root.join("alpimaps_mbtiles"),
            data_dir: repo_root.join("data/sources"),
            tmp_dir: repo_root.join("data/tmp"),
            elevation_tiles_dir: repo_root.join("elevation_tiles"),
            sources_json: repo_root.join("sources.json"),
            java_home: None,
            planetiler_jar: None,
            valhalla_bin_dir: Some(repo_root.join("valhalla/build")),
            heap_mb: 12288,
            log_interval: "1s".into(),
            areas: Vec::new(),
            repo_root,
        }
    }

    /// Load from disk, falling back to defaults when the file does not exist yet.
    pub fn load_or_default(path: &Path, repo_root: PathBuf) -> Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(text) => serde_json::from_str(&text)
                .with_context(|| format!("parsing settings at {}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::for_repo(repo_root)),
            Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
        }
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, serde_json::to_string_pretty(self)?)
            .with_context(|| format!("writing {}", path.display()))
    }

    /// Per-run temp directory. Two planetiler processes sharing one delete each other's sort
    /// chunks, so this is keyed by area and never handed out twice for concurrent runs.
    pub fn run_tmp_dir(&self, area: &str) -> PathBuf {
        self.tmp_dir.join(area)
    }

    pub fn area_dir(&self, area: &str) -> PathBuf {
        self.output_root.join(area)
    }

    pub fn area(&self, name: &str) -> Option<&AreaConfig> {
        self.areas.iter().find(|a| a.name == name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_follow_the_repo_layout() {
        let s = Settings::for_repo(PathBuf::from("/repo"));
        assert_eq!(s.output_root, PathBuf::from("/repo/alpimaps_mbtiles"));
        assert_eq!(s.data_dir, PathBuf::from("/repo/data/sources"));
        assert_eq!(s.area_dir("rhone-alpes"), PathBuf::from("/repo/alpimaps_mbtiles/rhone-alpes"));
    }

    #[test]
    fn missing_file_yields_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let s = Settings::load_or_default(&dir.path().join("nope.json"), "/repo".into()).unwrap();
        assert_eq!(s, Settings::for_repo("/repo".into()));
    }

    #[test]
    fn round_trips_through_disk() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let mut s = Settings::for_repo("/repo".into());
        s.areas.push(AreaConfig { name: "rhone-alpes".into(), poly: Some("/repo/ra.poly".into()) });
        s.heap_mb = 32768;
        s.save(&path).unwrap();
        assert_eq!(Settings::load_or_default(&path, "/other".into()).unwrap(), s);
    }

    /// A settings file from an older build must still load once new fields are added, or every
    /// upgrade bricks the app.
    #[test]
    fn tolerates_a_partial_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(&path, r#"{"heap_mb": 4096}"#).unwrap();
        let s = Settings::load_or_default(&path, "/repo".into()).unwrap();
        assert_eq!(s.heap_mb, 4096);
        assert_eq!(s.log_interval, "1s", "unspecified fields fall back to defaults");
    }
}
