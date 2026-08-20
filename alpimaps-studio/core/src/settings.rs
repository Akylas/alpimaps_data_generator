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
    /// Where to fetch the planetiler jar when there is none, so a released app need not carry
    /// 89 MB it may never use.
    ///
    /// This pipeline runs a *fork* of planetiler - the route and landcover layers are not
    /// upstream - so this cannot default to planetiler's own releases: a jar from there would
    /// build a different schema without saying so. It stays empty until the fork publishes one.
    #[serde(default)]
    pub planetiler_jar_url: Option<String>,
    pub valhalla_bin_dir: Option<PathBuf>,
    /// The `valhalla.json` used as the template for routing. The embedded router validates the
    /// whole document, so a hand-written stub is not enough - this points at the real one.
    /// `None` means `repo_root/valhalla.json`.
    #[serde(default)]
    pub valhalla_config: Option<PathBuf>,
    pub heap_mb: u32,
    /// Passed as `--loginterval`. Planetiler's own default is `10s`, which yields about six
    /// progress lines for a one-minute build - far too coarse to drive a progress bar.
    pub log_interval: String,
    pub areas: Vec<AreaConfig>,
    /// Where the front end's own bundled files live: the planetiler jar, valhalla.json, the
    /// Valhalla binaries.
    ///
    /// Discovered at run time, never stored - a packaged app's resource directory moves with
    /// the app, and a stale path written into settings.json would outlive an update. A packaged
    /// build has no repository to fall back on, which is why every tool lookup consults this
    /// first.
    #[serde(skip)]
    pub resource_dir: Option<PathBuf>,
    /// Where a downloaded jar is kept: the app's data directory, which survives an app update
    /// and is not inside the bundle. Discovered at run time like `resource_dir`.
    #[serde(skip)]
    pub jar_dir: Option<PathBuf>,
}

/// The newest `-with-deps.jar` in a directory. Several versions can sit side by side in a
/// submodule build; the highest name is the most recent.
fn newest_jar(dir: &Path) -> Option<PathBuf> {
    let mut jars: Vec<PathBuf> = std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.to_string_lossy().ends_with("-with-deps.jar"))
        .collect();
    jars.sort();
    jars.pop()
}

impl Default for Settings {
    fn default() -> Self {
        Self::for_repo(PathBuf::from("."))
    }
}

impl Settings {
    /// Find the repository from a starting directory, climbing a few levels.
    ///
    /// The app's working directory is wherever it was launched from - for `tauri dev` that is
    /// `alpimaps-studio/`, one level below the checkout, so a jar sitting in
    /// `planetiler/planetiler-dist/target` was invisible. A packaged install finds nothing here
    /// and falls back to its bundled resources, which is the point.
    pub fn locate_repo(start: &Path) -> PathBuf {
        let mut dir = start.to_path_buf();
        for _ in 0..4 {
            // markers a checkout has and nothing else does
            if dir.join("valhalla.json").is_file() || dir.join("planetiler").is_dir() {
                return dir;
            }
            match dir.parent() {
                Some(parent) => dir = parent.to_path_buf(),
                None => break,
            }
        }
        start.to_path_buf()
    }

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
            planetiler_jar_url: None,
            valhalla_bin_dir: Some(repo_root.join("valhalla/build")),
            valhalla_config: Some(repo_root.join("valhalla.json")),
            heap_mb: 12288,
            log_interval: "1s".into(),
            areas: Vec::new(),
            resource_dir: None,
            jar_dir: None,
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

    /// The planetiler jar to run.
    ///
    /// Configured first, then one downloaded into the app's own data directory, then the copy
    /// shipped inside the bundle, then the one the submodule builds. A packaged install has no
    /// submodule; a developer checkout has no bundle; either finds a jar without being told.
    pub fn planetiler_jar_path(&self) -> Option<PathBuf> {
        if let Some(jar) = &self.planetiler_jar {
            if jar.is_file() {
                return Some(jar.clone());
            }
        }
        if let Some(jar) = self.jar_dir.as_ref().and_then(|dir| newest_jar(dir)) {
            return Some(jar);
        }
        if let Some(jar) = self.resource_dir.as_ref().and_then(|dir| newest_jar(dir)) {
            return Some(jar);
        }
        newest_jar(&self.repo_root.join("planetiler/planetiler-dist/target"))
    }

    /// Where the Valhalla config template lives.
    ///
    /// The embedded router validates the whole document, so this cannot be a stub: it is either
    /// the one configured, the one shipped with the app, or the repository's.
    pub fn valhalla_config_path(&self) -> PathBuf {
        if let Some(path) = &self.valhalla_config {
            return path.clone();
        }
        if let Some(bundled) = self.resource_dir.as_ref().map(|dir| dir.join("valhalla.json")) {
            if bundled.is_file() {
                return bundled;
            }
        }
        self.repo_root.join("valhalla.json")
    }

    /// Where to look for `valhalla_build_tiles` and friends, in order. `PATH` is searched after
    /// all of these by the tool lookup itself.
    pub fn valhalla_bin_dirs(&self) -> Vec<PathBuf> {
        let mut dirs = Vec::new();
        if let Some(dir) = &self.valhalla_bin_dir {
            dirs.push(dir.clone());
        }
        if let Some(dir) = &self.resource_dir {
            dirs.push(dir.join("valhalla"));
            dirs.push(dir.clone());
        }
        dirs.push(self.repo_root.join("valhalla/build"));
        dirs
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

    /// The app is launched from wherever, and `tauri dev` launches it one level inside the
    /// checkout. Climbing is what makes the submodule jar visible from there.
    #[test]
    fn locate_repo_climbs_to_the_checkout() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("checkout");
        let inner = repo.join("alpimaps-studio/src-tauri");
        std::fs::create_dir_all(&inner).unwrap();
        std::fs::write(repo.join("valhalla.json"), b"{}").unwrap();
        assert_eq!(Settings::locate_repo(&inner), repo);
    }

    /// A packaged install has no checkout above it; inventing one would point every path at a
    /// directory the user never chose.
    #[test]
    fn locate_repo_gives_up_rather_than_guessing() {
        let dir = tempfile::tempdir().unwrap();
        let deep = dir.path().join("a/b/c");
        std::fs::create_dir_all(&deep).unwrap();
        assert_eq!(Settings::locate_repo(&deep), deep);
    }

    /// The jar shipped with the app is found without a checkout - which is the only way a
    /// packaged install can find one at all.
    #[test]
    fn the_bundled_jar_is_found_without_a_checkout() {
        let dir = tempfile::tempdir().unwrap();
        let resources = dir.path().join("Resources");
        std::fs::create_dir_all(&resources).unwrap();
        std::fs::write(resources.join("planetiler-dist-0.10.3-with-deps.jar"), b"x").unwrap();

        let mut settings = Settings::for_repo(dir.path().join("nowhere"));
        assert_eq!(settings.planetiler_jar_path(), None);
        settings.resource_dir = Some(resources.clone());
        assert_eq!(
            settings.planetiler_jar_path(),
            Some(resources.join("planetiler-dist-0.10.3-with-deps.jar"))
        );
    }
}
