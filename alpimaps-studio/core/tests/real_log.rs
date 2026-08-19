//! Runs the parser over a real planetiler log captured from a rhone-alpes build.
//!
//! Unit tests cover the shapes we know about; this covers the ones we do not. Skips silently
//! when the log is absent so the suite still runs on a fresh clone.

use studio_core::progress::{parse_line, LogEvent};

fn log_path() -> Option<std::path::PathBuf> {
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../bench/base.log")
        .canonicalize()
        .ok()?;
    p.exists().then_some(p)
}

#[test]
fn parses_a_real_build_log() {
    let Some(path) = log_path() else {
        eprintln!("skipping: bench/base.log not present");
        return;
    };
    let text = std::fs::read_to_string(&path).expect("read log");

    let (mut starts, mut progress, mut phase_ends, mut run_ends) = (0, 0, 0, 0);
    let mut zoom_ends = 0;
    let mut stages = std::collections::BTreeSet::new();

    for line in text.lines() {
        match parse_line(line) {
            LogEvent::PhaseStart { stage } => {
                stages.insert(stage);
                starts += 1;
            }
            LogEvent::Progress { stage, percent, .. } => {
                assert!(percent <= 100, "percent out of range in {line:?}");
                stages.insert(stage);
                progress += 1;
            }
            LogEvent::PhaseEnd { .. } => phase_ends += 1,
            LogEvent::ZoomEnd { zoom, .. } => {
                assert!((0..=20).contains(&zoom), "implausible zoom in {line:?}");
                zoom_ends += 1;
            }
            LogEvent::RunEnd => run_ends += 1,
            LogEvent::Log { .. } => {}
        }
    }

    eprintln!(
        "starts={starts} progress={progress} phase_ends={phase_ends} zoom_ends={zoom_ends} run_ends={run_ends} stages={}",
        stages.len()
    );
    eprintln!("stages: {stages:?}");

    assert!(starts > 5, "expected several stages to start, got {starts}");

    // Self-calibrating: count the lines that carry a bracketed percent independently, and
    // require the parser to have found exactly those. Catches both misses and false positives
    // without hard-coding a number that drifts with the sample log.
    let expected = text
        .lines()
        .map(studio_core::progress::strip_ansi)
        .filter(|l| {
            l.split('[')
                .skip(1)
                .filter_map(|rest| rest.split_once(']'))
                .any(|(body, _)| body.contains('%') && body.chars().any(|c| c.is_ascii_digit()))
        })
        .count();
    assert_eq!(progress, expected, "parser missed or invented progress lines");
    assert!(expected > 0, "sample log carries no progress lines at all");
    assert_eq!(run_ends, 1, "expected exactly one FINISHED! terminator, got {run_ends}");
    assert!(zoom_ends > 0, "expected per-zoom completions from the archive stage");
    // the stages planetiler always runs for an OSM build
    for expected in ["osm_pass1", "osm_pass2", "sort", "archive"] {
        assert!(stages.contains(expected), "missing stage {expected} in {stages:?}");
    }
}

/// The per-worker CPU summaries (`read 1x(19% 0.2s)`) must never register as progress - they
/// are the most common line shape in the log that contains a percent.
#[test]
fn no_worker_summary_is_read_as_progress() {
    let Some(path) = log_path() else { return };
    let text = std::fs::read_to_string(&path).expect("read log");
    for line in text.lines() {
        let stripped = studio_core::progress::strip_ansi(line);
        if !stripped.contains("x(") {
            continue;
        }
        // a line may legitimately carry both a bracketed counter and a paren summary; only
        // assert on the ones that have no bracketed counter at all
        if stripped.contains('[') && stripped.contains(']') && stripped.contains('%') {
            continue;
        }
        assert!(
            !matches!(parse_line(line), LogEvent::Progress { .. }),
            "worker summary parsed as progress: {stripped:?}"
        );
    }
}
