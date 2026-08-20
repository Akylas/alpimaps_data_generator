//! What is already built, judged from the output files themselves.
//!
//! A build is long enough that stopping halfway is normal, and re-running the whole plan to get
//! the one step that is missing is not acceptable. So a step counts as built when the files it
//! produces are there and non-empty - not because something wrote down that it ran. Output
//! built by the shell scripts, copied in from another machine, or produced before any of this
//! existed all count the same way, and deleting a file is all it takes to make the step run
//! again.
//!
//! A record (`<area>/.studio-state.json`) is kept alongside, but only as *extra* information:
//! how long the step took, and which options it ran with, so an option edited since then can be
//! reported as a reason to rebuild. Losing the record loses the timing and the option check,
//! never the knowledge that the output exists.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::StepId;

const FILE_NAME: &str = ".studio-state.json";
const VERSION: u32 = 1;

/// One output file, as found on disk.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutputFile {
    pub name: String,
    pub bytes: u64,
    /// Seconds since the epoch. Formatting is the UI's business.
    pub modified: u64,
}

/// One completed run of one step. Supplementary to the files, never a substitute for them.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepRecord {
    pub finished_at: u64,
    #[serde(default)]
    pub elapsed: Option<String>,
    /// Fingerprint of the options the step ran with.
    pub options_hash: String,
    /// The options themselves, so the UI can say *what* changed rather than only that it did.
    #[serde(default)]
    pub options: BTreeMap<String, Value>,
}

/// Everything recorded for one area.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BuildState {
    #[serde(default)]
    pub version: u32,
    #[serde(default)]
    pub steps: BTreeMap<StepId, StepRecord>,
}

/// Whether a step needs to run, and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum StepStatus {
    /// Nothing on disk. Zero-byte files count as nothing: an interrupted run leaves those
    /// behind, and treating one as output would skip the step that should replace it.
    Missing { missing: Vec<String> },
    /// Every output present. Safe to skip.
    Built {
        outputs: Vec<OutputFile>,
        /// From the record, when there is one.
        elapsed: Option<String>,
        /// When the step last ran, if recorded; otherwise the newest output's timestamp.
        finished_at: u64,
        /// Whether a record backs this up, or the files are all that is known.
        tracked: bool,
    },
    /// Output is there, but the options have been edited since it was produced.
    OptionsChanged { changed: Vec<String>, outputs: Vec<OutputFile> },
    /// The step writes outside the area directory, so its output cannot be checked here.
    Unknown,
}

impl StepStatus {
    /// Whether a re-run can be skipped.
    pub fn is_fresh(&self) -> bool {
        matches!(self, StepStatus::Built { .. })
    }
}

pub fn state_path(area_dir: &Path) -> PathBuf {
    area_dir.join(FILE_NAME)
}

/// Read the record for an area. A missing or unreadable record is an empty one, not an error -
/// the files are what matter.
pub fn load(area_dir: &Path) -> BuildState {
    let raw = match std::fs::read_to_string(state_path(area_dir)) {
        Ok(raw) => raw,
        Err(_) => return BuildState::default(),
    };
    match serde_json::from_str::<BuildState>(&raw) {
        Ok(state) if state.version == VERSION => state,
        // a record written by a different layout is not worth guessing at
        _ => BuildState::default(),
    }
}

pub fn save(area_dir: &Path, state: &BuildState) -> anyhow::Result<()> {
    std::fs::create_dir_all(area_dir)?;
    let mut state = state.clone();
    state.version = VERSION;
    let json = serde_json::to_string_pretty(&state)?;
    // write-then-rename: a crash mid-write must not leave a half-parsed record behind
    let tmp = state_path(area_dir).with_extension("json.tmp");
    std::fs::write(&tmp, json)?;
    std::fs::rename(&tmp, state_path(area_dir))?;
    Ok(())
}

/// Note that a step finished, with the options it used. The files are what make it count as
/// built; this only adds the timing and the option fingerprint.
pub fn mark_done(
    area_dir: &Path,
    step: StepId,
    elapsed: Option<String>,
    options: &BTreeMap<String, Value>,
) -> anyhow::Result<()> {
    let mut state = load(area_dir);
    state.steps.insert(
        step,
        StepRecord {
            finished_at: now(),
            elapsed,
            options_hash: hash_options(options),
            options: options.clone(),
        },
    );
    save(area_dir, &state)
}

/// Drop the record for one step, so its options no longer count as known.
///
/// This does not delete anything the step produced - the output is still there, and the step
/// still reads as built. Removing the file is what makes it run again.
pub fn clear(area_dir: &Path, step: StepId) -> anyhow::Result<()> {
    let mut state = load(area_dir);
    state.steps.remove(&step);
    save(area_dir, &state)
}

pub fn clear_all(area_dir: &Path) -> anyhow::Result<()> {
    save(area_dir, &BuildState::default())
}

/// Delete what a step produced, which is the honest way to make it run again.
///
/// Returns the files actually removed.
pub fn remove_outputs(area_dir: &Path, area: &str, step: StepId) -> anyhow::Result<Vec<String>> {
    let mut removed = Vec::new();
    for name in step.outputs(area) {
        let path = area_dir.join(&name);
        if path.exists() {
            std::fs::remove_file(&path)?;
            removed.push(name);
        }
    }
    let _ = clear(area_dir, step);
    Ok(removed)
}

fn probe(area_dir: &Path, name: &str) -> Option<OutputFile> {
    let meta = std::fs::metadata(area_dir.join(name)).ok()?;
    if !meta.is_file() || meta.len() == 0 {
        return None;
    }
    let modified = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0);
    Some(OutputFile { name: name.to_string(), bytes: meta.len(), modified })
}

/// Judge one step from its output files, then from the record if there is one.
pub fn status(
    area_dir: &Path,
    area: &str,
    step: StepId,
    options: &BTreeMap<String, Value>,
) -> StepStatus {
    let expected = step.outputs(area);
    if expected.is_empty() {
        return StepStatus::Unknown;
    }

    let mut found = Vec::new();
    let mut missing = Vec::new();
    for name in &expected {
        match probe(area_dir, name) {
            Some(file) => found.push(file),
            None => missing.push(name.clone()),
        }
    }
    if !missing.is_empty() {
        return StepStatus::Missing { missing };
    }

    let newest = found.iter().map(|f| f.modified).max().unwrap_or(0);
    let Some(record) = load(area_dir).steps.remove(&step) else {
        return StepStatus::Built {
            outputs: found,
            elapsed: None,
            finished_at: newest,
            tracked: false,
        };
    };

    if record.options_hash != hash_options(options) {
        let mut changed: Vec<String> = Vec::new();
        for key in record.options.keys().chain(options.keys()) {
            if record.options.get(key) != options.get(key) && !changed.contains(key) {
                changed.push(key.clone());
            }
        }
        return StepStatus::OptionsChanged { changed, outputs: found };
    }

    StepStatus::Built {
        outputs: found,
        elapsed: record.elapsed,
        finished_at: record.finished_at,
        tracked: true,
    }
}

/// Status for every step, for the UI to render in one call.
pub fn statuses(
    area_dir: &Path,
    area: &str,
    options: &BTreeMap<StepId, BTreeMap<String, Value>>,
) -> BTreeMap<StepId, StepStatus> {
    let empty = BTreeMap::new();
    super::ALL_STEPS
        .iter()
        .map(|step| (*step, status(area_dir, area, *step, options.get(step).unwrap_or(&empty))))
        .collect()
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Fingerprint a set of option values.
///
/// `BTreeMap` gives a stable key order and `serde_json` a stable rendering of each value, so the
/// same options always hash the same way without pulling in a hashing crate for what is really
/// a change detector.
fn hash_options(options: &BTreeMap<String, Value>) -> String {
    let canonical = serde_json::to_string(options).unwrap_or_default();
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in canonical.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn opts(pairs: &[(&str, Value)]) -> BTreeMap<String, Value> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.clone())).collect()
    }

    #[test]
    fn no_output_means_the_step_has_to_run() {
        let dir = tempfile::tempdir().unwrap();
        match status(dir.path(), "alps", StepId::Basemap, &BTreeMap::new()) {
            StepStatus::Missing { missing } => assert_eq!(missing, vec!["alps.mbtiles"]),
            other => panic!("expected missing output, got {other:?}"),
        }
    }

    /// The whole point of checking files rather than a record: output from the shell scripts
    /// counts, with nothing written down anywhere.
    #[test]
    fn output_alone_counts_as_built() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("alps.mbtiles"), b"x").unwrap();
        let status = status(dir.path(), "alps", StepId::Basemap, &BTreeMap::new());
        assert!(status.is_fresh());
        match status {
            StepStatus::Built { tracked, outputs, .. } => {
                assert!(!tracked);
                assert_eq!(outputs[0].bytes, 1);
            }
            other => panic!("expected built, got {other:?}"),
        }
    }

    /// An interrupted run leaves a zero-byte file. Counting it as output would skip the step
    /// that should replace it.
    #[test]
    fn an_empty_file_is_not_output() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("alps.mbtiles"), b"").unwrap();
        assert!(!status(dir.path(), "alps", StepId::Basemap, &BTreeMap::new()).is_fresh());
    }

    #[test]
    fn deleting_the_output_makes_it_run_again() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("alps.mbtiles");
        std::fs::write(&out, b"x").unwrap();
        mark_done(dir.path(), StepId::Basemap, Some("2m".into()), &BTreeMap::new()).unwrap();
        assert!(status(dir.path(), "alps", StepId::Basemap, &BTreeMap::new()).is_fresh());
        std::fs::remove_file(&out).unwrap();
        assert!(!status(dir.path(), "alps", StepId::Basemap, &BTreeMap::new()).is_fresh());
    }

    #[test]
    fn changing_an_option_makes_it_stale_and_says_which() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("alps.mbtiles"), b"x").unwrap();
        mark_done(dir.path(), StepId::Basemap, None, &opts(&[("maxzoom", json!(14))])).unwrap();
        match status(dir.path(), "alps", StepId::Basemap, &opts(&[("maxzoom", json!(13))])) {
            StepStatus::OptionsChanged { changed, .. } => assert_eq!(changed, vec!["maxzoom"]),
            other => panic!("expected changed options, got {other:?}"),
        }
    }

    /// Key order is an artefact of how the UI happened to build the map, not a change.
    #[test]
    fn option_order_does_not_count_as_a_change() {
        let a = opts(&[("z", json!(1)), ("a", json!(2))]);
        let b = opts(&[("a", json!(2)), ("z", json!(1))]);
        assert_eq!(hash_options(&a), hash_options(&b));
    }

    /// Clearing forgets the options, not the output - the file is still there, so the step is
    /// still built.
    #[test]
    fn clearing_forgets_the_options_not_the_output() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("alps.mbtiles"), b"x").unwrap();
        mark_done(dir.path(), StepId::Basemap, None, &opts(&[("maxzoom", json!(14))])).unwrap();
        clear(dir.path(), StepId::Basemap).unwrap();
        match status(dir.path(), "alps", StepId::Basemap, &opts(&[("maxzoom", json!(13))])) {
            StepStatus::Built { tracked, .. } => assert!(!tracked),
            other => panic!("expected built and untracked, got {other:?}"),
        }
    }

    #[test]
    fn removing_outputs_is_what_makes_a_step_run_again() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("alps.mbtiles"), b"x").unwrap();
        let removed = remove_outputs(dir.path(), "alps", StepId::Basemap).unwrap();
        assert_eq!(removed, vec!["alps.mbtiles"]);
        assert!(!status(dir.path(), "alps", StepId::Basemap, &BTreeMap::new()).is_fresh());
    }

    /// Steps that write outside the area directory cannot be judged from it, and must not be
    /// reported as built just because nothing is missing.
    #[test]
    fn steps_writing_elsewhere_are_unknown() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            status(dir.path(), "alps", StepId::DownloadOsm, &BTreeMap::new()),
            StepStatus::Unknown
        );
        assert!(!StepStatus::Unknown.is_fresh());
    }
}
