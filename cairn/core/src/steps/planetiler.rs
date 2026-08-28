//! Running planetiler as a subprocess.
//!
//! Deliberately not embedded as a library, even though planetiler is Java and we could be too:
//!
//! * heap - builds run with `-Xmx32g`; in-process an OOM takes the GUI down with it, and the
//!   whole app would have to reserve that heap up front
//! * `Planetiler` refuses a second `run()` on the same instance, and the static state around it
//!   (logging, stats, worker pools, mmap arenas) has never been exercised twice in one JVM
//! * a cancelled build leaves mmap'd sort chunks behind; killing a process guarantees cleanup
//! * the with-deps jar is ~89 MB of classpath we would otherwise merge into our own
//!
//! Consequence: progress has to be scraped from stdout. See `crate::progress`.

use crate::progress::{parse_line, LogEvent};
use crate::steps::{StepEvent, StepId};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::mpsc;

/// Which planetiler entry point to invoke. Verified against `planetiler-dist` `Main.java`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Schema {
    /// The bundled OpenMapTiles fork - our own schema, which stays Java because `Route.java`
    /// needs OSM relations and `planetiler-custommap` has no relation support.
    OpenMapTiles,
    /// Any YAML custommap schema, including the bundled `shortbread.yml`.
    Yaml { path: PathBuf },
}

impl Schema {
    fn subcommand(&self) -> &'static str {
        match self {
            Schema::OpenMapTiles => "openmaptiles",
            Schema::Yaml { .. } => "custom",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanetilerJob {
    pub step: StepId,
    pub area: String,
    pub java: PathBuf,
    pub jar: PathBuf,
    pub schema: Schema,
    pub heap_mb: u32,
    pub output: PathBuf,
    /// Per-run temp directory. Two concurrent planetiler processes sharing `data/tmp` delete
    /// each other's sort chunks (`NoSuchFileException: data/tmp/feature.db/chunk8`), so the
    /// runner must never let two jobs share one.
    pub tmp_dir: PathBuf,
    /// Everything else, verbatim: `--area=`, `--polygon=`, the simplification flags, and our
    /// custom ones (`route_road_tolerance`, `landcover_tolerance_z11_13`, ...).
    pub extra_args: Vec<String>,
    pub working_dir: PathBuf,
    /// Planetiler's `--loginterval`, which gates how often a progress line is emitted at all.
    /// It defaults to `10s`, which is fine for a terminal and far too coarse for a progress
    /// bar - a 54s rhone-alpes build produces only 6 progress lines at the default.
    pub log_interval: String,
}

impl PlanetilerJob {
    pub fn command_line(&self) -> Vec<String> {
        let mut argv = vec![
            self.java.display().to_string(),
            format!("-Xmx{}m", self.heap_mb),
            "-jar".into(),
            self.jar.display().to_string(),
            self.schema.subcommand().into(),
        ];
        if let Schema::Yaml { path } = &self.schema {
            argv.push(format!("--schema={}", path.display()));
        }
        argv.push(format!("--mbtiles={}", self.output.display()));
        argv.push(format!("--tmpdir={}", self.tmp_dir.display()));
        argv.push(format!("--loginterval={}", self.log_interval));
        argv.extend(self.extra_args.iter().cloned());
        argv
    }
}

/// Run the job to completion, streaming events into `tx`.
///
/// Returns `Ok(true)` when planetiler exited 0. A non-zero exit is `Ok(false)`, not an `Err` -
/// a failed build is a result to display, not an error in the runner.
pub async fn run(job: PlanetilerJob, tx: mpsc::Sender<StepEvent>) -> Result<bool> {
    let (_cancel_tx, cancel_rx) = mpsc::channel(1);
    run_cancellable(job, tx, cancel_rx).await
}

/// As [`run`], but a message on `cancel` kills the subprocess.
///
/// Killing rather than signalling is deliberate: planetiler leaves mmap'd sort chunks in its
/// tmpdir, and a dead process releases them where a half-unwound one may not.
pub async fn run_cancellable(
    job: PlanetilerJob,
    tx: mpsc::Sender<StepEvent>,
    mut cancel: mpsc::Receiver<()>,
) -> Result<bool> {
    let argv = job.command_line();
    let step = job.step;

    tokio::fs::create_dir_all(&job.tmp_dir).await.ok();
    if let Some(parent) = job.output.parent() {
        tokio::fs::create_dir_all(parent).await.ok();
    }

    let _ = tx.send(StepEvent::Started { step, area: job.area.clone() }).await;
    let _ = tx.send(StepEvent::Command { step, argv: argv.clone() }).await;

    let mut child = tokio::process::Command::new(&argv[0])
        .args(&argv[1..])
        .current_dir(&job.working_dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("spawning {}", argv[0]))?;

    // planetiler writes its progress banner to stdout and some warnings to stderr; both matter,
    // so merge them into one ordered stream rather than picking a favourite
    let stdout = child.stdout.take().context("no stdout pipe")?;
    let stderr = child.stderr.take().context("no stderr pipe")?;
    let (line_tx, mut line_rx) = mpsc::channel::<String>(256);

    for pipe in [
        Box::new(stdout) as Box<dyn tokio::io::AsyncRead + Unpin + Send>,
        Box::new(stderr),
    ] {
        let line_tx = line_tx.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(pipe).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if line_tx.send(line).await.is_err() {
                    break;
                }
            }
        });
    }
    drop(line_tx);

    let mut last_elapsed = None;
    let mut cancelled = false;
    loop {
        let line = tokio::select! {
            maybe = line_rx.recv() => match maybe {
                Some(line) => line,
                None => break,
            },
            // `recv()` also completes with None once every sender is dropped. Treating that as
            // a cancellation kills the build the moment a caller does not hold on to the
            // sender - which looks exactly like planetiler failing for no reason.
            signal = cancel.recv() => {
                if signal.is_none() {
                    // nobody can cancel any more; keep reading output until the process ends
                    cancel = mpsc::channel(1).1;
                    continue;
                }
                cancelled = true;
                let _ = child.kill().await;
                let _ = tx.send(StepEvent::Log { step, line: "cancelled".into() }).await;
                break;
            }
        };
        let event = match parse_line(&line) {
            LogEvent::PhaseStart { stage } => StepEvent::Phase { step, name: stage },
            LogEvent::Progress { label, percent, .. } => {
                StepEvent::Progress { step, label, percent }
            }
            // `FINISHED!` carries no duration; the total is on the `Finished in` line two
            // lines above it, which we have already seen as a PhaseEnd.
            LogEvent::RunEnd => StepEvent::Log { step, line: "FINISHED!".into() },
            LogEvent::PhaseEnd { stage, elapsed } => {
                last_elapsed = Some(elapsed.clone());
                StepEvent::Log { step, line: format!("{stage} finished in {elapsed}") }
            }
            LogEvent::ZoomEnd { zoom, elapsed, .. } => {
                StepEvent::Phase { step, name: format!("archive z{zoom} ({elapsed})") }
            }
            LogEvent::Log { line } => StepEvent::Log { step, line },
        };
        if tx.send(event).await.is_err() {
            // receiver dropped: nobody is listening, so stop the build too
            let _ = child.kill().await;
            break;
        }
    }

    let status = child.wait().await?;
    let ok = status.success() && !cancelled;
    let outputs = if ok {
        vec![job.output.display().to_string()]
    } else {
        vec![]
    };
    let _ = tx
        .send(StepEvent::Finished { step, ok, elapsed: last_elapsed, outputs })
        .await;
    Ok(ok)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn job() -> PlanetilerJob {
        PlanetilerJob {
            step: StepId::Basemap,
            area: "rhone-alpes".into(),
            java: "/usr/bin/java".into(),
            jar: "planetiler.jar".into(),
            schema: Schema::OpenMapTiles,
            heap_mb: 12288,
            output: "out/rhone-alpes.mbtiles".into(),
            tmp_dir: "tmp/run-1".into(),
            extra_args: vec!["--area=rhone-alpes".into(), "--force".into()],
            working_dir: ".".into(),
            log_interval: "1s".into(),
        }
    }

    #[test]
    fn builds_openmaptiles_command() {
        let argv = job().command_line();
        assert_eq!(argv[1], "-Xmx12288m");
        assert_eq!(argv[4], "openmaptiles");
        assert!(argv.contains(&"--mbtiles=out/rhone-alpes.mbtiles".to_string()));
        assert!(argv.contains(&"--area=rhone-alpes".to_string()));
    }

    /// Every job gets its own tmpdir; sharing `data/tmp` between two runs corrupts both.
    #[test]
    fn always_passes_a_tmpdir() {
        assert!(job()
            .command_line()
            .iter()
            .any(|a| a == "--tmpdir=tmp/run-1"));
    }

    /// The default 10s interval yields ~6 progress lines for a one-minute build; a GUI needs
    /// one every second or the bar looks frozen.
    #[test]
    fn overrides_the_log_interval() {
        assert!(job().command_line().iter().any(|a| a == "--loginterval=1s"));
    }

    /// A caller that drops the cancel sender - or never holds one - must still get a full run.
    #[tokio::test]
    async fn a_dropped_cancel_sender_does_not_cancel() {
        let dir = tempfile::tempdir().unwrap();
        let mut j = job();
        // `true` exits 0 immediately and needs no jar
        j.java = "/usr/bin/true".into();
        j.working_dir = dir.path().to_path_buf();
        j.output = dir.path().join("out.mbtiles");
        j.tmp_dir = dir.path().join("tmp");

        let (tx, mut rx) = mpsc::channel(64);
        let cancel = mpsc::channel(1).1; // sender dropped right here
        let handle = tokio::spawn(run_cancellable(j, tx, cancel));

        let mut cancelled = false;
        while let Some(event) = rx.recv().await {
            if let StepEvent::Log { line, .. } = &event {
                if line == "cancelled" {
                    cancelled = true;
                }
            }
        }
        assert!(!cancelled, "a dropped sender must not read as a cancellation");
        assert!(handle.await.unwrap().unwrap(), "the process ran to completion");
    }

    #[test]
    fn yaml_schema_uses_custom_subcommand() {
        let mut j = job();
        j.schema = Schema::Yaml { path: "shortbread.yml".into() };
        let argv = j.command_line();
        assert_eq!(argv[4], "custom");
        assert!(argv.contains(&"--schema=shortbread.yml".to_string()));
    }
}
