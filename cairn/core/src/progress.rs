//! Parser for planetiler's stdout.
//!
//! Planetiler is run as a subprocess (see `steps::planetiler` for why), so progress has to come
//! from its log lines. Real samples, ANSI already stripped:
//!
//! ```text
//! 0:00:01 INF [lake_centerlines] - Starting...
//! 0:00:02 INF [lake_centerlines] -  read: [   22 100%   53/s ] write: [    0    0/s ] 0
//! 0:00:02 INF [lake_centerlines] - Finished in 1s cpu:3s avg:2.7
//! 0:00:02 INF [lake_centerlines] -   read     1x(19% 0.2s)
//! 0:00:47 INF [sort] -  chunks: [   2 /   2 100% ] 1.7G
//! 0:00:50 INF [archive:write] - Finished z12 in 1s cpu:20s avg:15.7, now starting z13
//! 0:00:53 INF [archive] - Finished in 54s cpu:5m24s gc:1s avg:6
//! 0:00:53 INF [archive] - FINISHED!
//! 0:00:53 INF [archive] -  features: [  22M 100% 3.7M/s ] 1.7G  tiles: [  21k 3.6k/s ] 296M
//!     cpus: 0.7 gc:  0% heap: 67M/34G direct: 245k postGC: 76M
//! ```
//!
//! Two traps the shapes above encode:
//!
//! 1. `read 1x(19% 0.2s)` is a per-worker CPU summary, not progress. The percent that means
//!    progress always sits inside `[...]`; the percent that doesn't always sits inside `(...)`.
//!    So brackets are the discriminator, not "first percent on the line".
//! 2. Continuation lines (`cpus:`, `->`) carry a `gc: 0%` and have no `time LEVEL [stage] -`
//!    prefix. Requiring the prefix drops them.
//! 3. The whole-run end is **not** distinguishable by the absence of a stage - it is logged
//!    under `[archive]`, same as that stage's own end, and the two `Finished in` lines sit two
//!    lines apart. `FINISHED!` is the only reliable terminator, and it carries no duration, so
//!    the elapsed total has to be remembered from the last `Finished in` before it.
//!
//! Anything unrecognised becomes `Log`. The format is not a contract - upstream merges move it -
//! so the parser must never fail a build over a line it does not understand.

use regex::Regex;
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

/// One parsed line. `Log` is the catch-all and by far the most common.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LogEvent {
    /// A named stage began, e.g. `osm_pass1`.
    PhaseStart { stage: String },
    /// Progress within a stage. `label` is the counter name (`read`, `features`, `chunks`).
    Progress {
        stage: String,
        label: String,
        percent: u8,
    },
    /// A stage completed. `elapsed` is planetiler's own rendering, e.g. `1s`, `2m30s`.
    PhaseEnd { stage: String, elapsed: String },
    /// One zoom level of the archive stage finished. The archive stage is the bulk of a build,
    /// so this is the only fine-grained signal available inside it.
    ZoomEnd { stage: String, zoom: u8, elapsed: String },
    /// The whole run terminated. Carries no duration of its own - see trap 3.
    RunEnd,
    Log { line: String },
}

fn ansi() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"\x1b\[[0-9;]*[a-zA-Z]").unwrap())
}

/// `0:00:02 INF [stage] - content`, stage optional.
fn prefix() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| {
        Regex::new(r"^(?<time>\d+:\d{2}:\d{2})\s+(?<level>[A-Z]+)\s+(?:\[(?<stage>[^\]]+)\]\s+)?-\s?(?<content>.*)$")
            .unwrap()
    })
}

/// `label: [ ...counter... ]` - the percent inside the brackets is the real one.
fn counter() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"(?<label>[A-Za-z][A-Za-z_ ]*):\s*\[(?<body>[^\]]*)\]").unwrap())
}

fn percent() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"(?<pct>\d{1,3})%").unwrap())
}

/// Matches both `Finished in 54s ...` and `Finished z12 in 1s ..., now starting z13`.
fn finished() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"^Finished (?:z(?<zoom>\d+) )?in (?<elapsed>\S+)").unwrap())
}

pub fn strip_ansi(line: &str) -> String {
    ansi().replace_all(line, "").into_owned()
}

/// Parse a single raw line. Never fails; unrecognised input returns `Log`.
pub fn parse_line(raw: &str) -> LogEvent {
    let clean = strip_ansi(raw);
    let trimmed = clean.trim_end();

    let Some(caps) = prefix().captures(trimmed) else {
        return LogEvent::Log { line: trimmed.to_string() };
    };
    let content = caps.name("content").map(|m| m.as_str()).unwrap_or("").trim();
    let stage = caps.name("stage").map(|m| m.as_str().to_string());

    // the only trustworthy end-of-run marker; every `Finished in` line is a stage
    if content == "FINISHED!" {
        return LogEvent::RunEnd;
    }

    if let Some(f) = finished().captures(content) {
        let elapsed = f["elapsed"].to_string();
        let stage = stage.unwrap_or_default();
        return match f.name("zoom").and_then(|z| z.as_str().parse::<u8>().ok()) {
            Some(zoom) => LogEvent::ZoomEnd { stage, zoom, elapsed },
            None => LogEvent::PhaseEnd { stage, elapsed },
        };
    }

    if let Some(stage) = stage {
        if content == "Starting..." {
            return LogEvent::PhaseStart { stage };
        }
        // first bracketed counter that actually carries a percent. `tiles: [ 21k 3.6k/s ]` has
        // none and must not shadow a later counter that does.
        for c in counter().captures_iter(content) {
            if let Some(p) = percent().captures(&c["body"]) {
                if let Ok(pct) = p["pct"].parse::<u8>() {
                    return LogEvent::Progress {
                        stage,
                        label: c["label"].trim().to_string(),
                        percent: pct.min(100),
                    };
                }
            }
        }
    }

    LogEvent::Log { line: trimmed.to_string() }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prog(line: &str) -> (String, String, u8) {
        match parse_line(line) {
            LogEvent::Progress { stage, label, percent } => (stage, label, percent),
            other => panic!("expected Progress, got {other:?} for {line:?}"),
        }
    }

    #[test]
    fn reads_progress_counter() {
        let (stage, label, pct) =
            prog("0:00:02 INF [lake_centerlines] -  read: [   22 100%   53/s ] write: [    0    0/s ] 0");
        assert_eq!((stage.as_str(), label.as_str(), pct), ("lake_centerlines", "read", 100));
    }

    #[test]
    fn reads_suffixed_counts() {
        let (stage, label, pct) =
            prog("0:00:53 INF [archive] -  features: [  22M 100% 3.7M/s ] 1.7G  tiles: [  21k 3.6k/s ] 296M");
        assert_eq!((stage.as_str(), label.as_str(), pct), ("archive", "features", 100));
    }

    #[test]
    fn reads_ratio_counter() {
        let (stage, label, pct) = prog("0:00:47 INF [sort] -  chunks: [   2 /   2 100% ] 1.7G");
        assert_eq!((stage.as_str(), label.as_str(), pct), ("sort", "chunks", 100));
    }

    /// The trap: `1x(19% 0.2s)` is per-worker CPU time, not progress.
    #[test]
    fn worker_cpu_summary_is_not_progress() {
        assert!(matches!(
            parse_line("0:00:02 INF [lake_centerlines] -   read     1x(19% 0.2s)"),
            LogEvent::Log { .. }
        ));
    }

    /// The other trap: continuation lines carry `gc: 0%` and no prefix.
    #[test]
    fn continuation_line_is_not_progress() {
        assert!(matches!(
            parse_line("    cpus: 0.7 gc:  0% heap: 67M/34G direct: 245k postGC: 76M"),
            LogEvent::Log { .. }
        ));
    }

    #[test]
    fn pipeline_detail_line_is_not_progress() {
        assert!(matches!(
            parse_line("    ->     (0/3) -> read( -%) ->    (0/1k) -> process( -%  -%) ->   (0/68k)"),
            LogEvent::Log { .. }
        ));
    }

    #[test]
    fn phase_start_and_end() {
        assert_eq!(
            parse_line("0:00:01 INF [lake_centerlines] - Starting..."),
            LogEvent::PhaseStart { stage: "lake_centerlines".into() }
        );
        assert_eq!(
            parse_line("0:00:02 INF [lake_centerlines] - Finished in 1s cpu:3s avg:2.7"),
            LogEvent::PhaseEnd { stage: "lake_centerlines".into(), elapsed: "1s".into() }
        );
    }

    /// The whole-run end is logged under `[archive]`, indistinguishable from that stage's own
    /// end except by the `FINISHED!` line that follows it.
    #[test]
    fn run_end_is_the_finished_bang_line() {
        assert_eq!(parse_line("0:00:53 INF [archive] - FINISHED!"), LogEvent::RunEnd);
        assert_eq!(
            parse_line("0:00:53 INF [archive] - Finished in 54s cpu:5m24s gc:1s avg:6"),
            LogEvent::PhaseEnd { stage: "archive".into(), elapsed: "54s".into() }
        );
    }

    /// Per-zoom completions inside the archive stage - the finest progress signal available
    /// for the stage that dominates a build.
    #[test]
    fn reads_zoom_completions() {
        assert_eq!(
            parse_line("0:00:50 INF [archive:write] - Finished z12 in 1s cpu:20s avg:15.7, now starting z13"),
            LogEvent::ZoomEnd { stage: "archive:write".into(), zoom: 12, elapsed: "1s".into() }
        );
    }

    #[test]
    fn strips_ansi_before_matching() {
        let (stage, _, pct) =
            prog("\u{1b}[0m0:00:02 INF [osm_pass1] -  read: [ \u{1b}[32m 50 50%\u{1b}[0m  53/s ]");
        assert_eq!((stage.as_str(), pct), ("osm_pass1", 50));
    }

    #[test]
    fn debug_lines_are_logs() {
        assert!(matches!(
            parse_line("0:00:01 DEB - argument: vacuum_analyze=false (mbtiles: vacuum analyze)"),
            LogEvent::Log { .. }
        ));
    }
}
