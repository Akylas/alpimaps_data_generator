//! Running the Valhalla command-line tools as steps.
//!
//! `valhalla_build_elevation` and `valhalla_build_tiles` are the two parts of the pipeline that
//! stay subprocesses. Both are C++ binaries from the submodule build, both take an hour or more
//! on a large area, and neither has an embeddable form worth the linking. What this module adds
//! is what the app needs and a shell does not give: their output as events, and a cancel that
//! actually kills the process.

use std::path::{Path, PathBuf};
use std::process::Stdio;

use anyhow::{anyhow, Result};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc;

use super::{StepEvent, StepId};

/// One Valhalla binary, its arguments, and where to run it.
pub struct ToolJob {
    pub step: StepId,
    pub area: String,
    /// The binary, already resolved to a path.
    pub program: PathBuf,
    pub args: Vec<String>,
    pub working_dir: PathBuf,
}

impl ToolJob {
    /// What would run, for a dry run or an error message.
    pub fn command_line(&self) -> Vec<String> {
        let mut line = vec![self.program.display().to_string()];
        line.extend(self.args.clone());
        line
    }
}

/// Find a Valhalla binary, preferring the configured build directory.
///
/// Falls back to `PATH`, which is how a system install or `env.sh` provides them.
pub fn find_tool(bin_dir: Option<&Path>, name: &str) -> Option<PathBuf> {
    if let Some(dir) = bin_dir {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).map(|dir| dir.join(name)).find(|p| p.is_file())
}

/// Run the tool, streaming its output as events, killing it if `cancel` fires.
///
/// Returns whether it exited cleanly. Both binaries log to stderr, so the two streams are read
/// together - reading only stdout is how a run looks silent for forty minutes.
pub async fn run(
    job: ToolJob,
    tx: mpsc::Sender<StepEvent>,
    mut cancel: mpsc::Receiver<()>,
) -> Result<bool> {
    let step = job.step;
    if !job.program.is_file() {
        return Err(anyhow!(
            "{} not found - build the Valhalla submodule, or set the binary directory in Settings",
            job.program.display()
        ));
    }
    let _ = tx.send(StepEvent::Started { step, area: job.area.clone() }).await;
    let _ = tx
        .send(StepEvent::Log { step, line: job.command_line().join(" ") })
        .await;

    let mut child = Command::new(&job.program)
        .args(&job.args)
        .current_dir(&job.working_dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()?;

    let mut out = BufReader::new(child.stdout.take().expect("stdout piped")).lines();
    let mut err = BufReader::new(child.stderr.take().expect("stderr piped")).lines();
    let mut cancelled = false;

    loop {
        tokio::select! {
            line = out.next_line() => match line? {
                Some(line) => emit(&tx, step, line).await,
                // stdout closing does not mean the process is done; stderr may still be open
                None => if err_done(&mut err, &tx, step).await? { break },
            },
            line = err.next_line() => match line? {
                Some(line) => emit(&tx, step, line).await,
                None => if out_done(&mut out, &tx, step).await? { break },
            },
            signal = cancel.recv() => {
                if signal.is_none() {
                    // nobody can cancel any more; keep reading until the process ends
                    cancel = mpsc::channel(1).1;
                    continue;
                }
                cancelled = true;
                let _ = child.start_kill();
                break;
            }
        }
    }

    let status = child.wait().await?;
    let ok = status.success() && !cancelled;
    let _ = tx
        .send(StepEvent::Finished { step, ok, elapsed: None, outputs: vec![] })
        .await;
    Ok(ok)
}

/// Drain the other stream once one has closed, then report that both are done.
async fn err_done(
    err: &mut tokio::io::Lines<BufReader<tokio::process::ChildStderr>>,
    tx: &mpsc::Sender<StepEvent>,
    step: StepId,
) -> Result<bool> {
    while let Some(line) = err.next_line().await? {
        emit(tx, step, line).await;
    }
    Ok(true)
}

async fn out_done(
    out: &mut tokio::io::Lines<BufReader<tokio::process::ChildStdout>>,
    tx: &mpsc::Sender<StepEvent>,
    step: StepId,
) -> Result<bool> {
    while let Some(line) = out.next_line().await? {
        emit(tx, step, line).await;
    }
    Ok(true)
}

/// Turn a log line into an event, promoting the stage markers to phases.
async fn emit(tx: &mpsc::Sender<StepEvent>, step: StepId, line: String) {
    if let Some(name) = phase_of(&line) {
        let _ = tx.send(StepEvent::Phase { step, name }).await;
    }
    let _ = tx.send(StepEvent::Log { step, line }).await;
}

/// Recognise the stage banners Valhalla prints, so the UI can show where a long build is.
///
/// `valhalla_build_tiles` announces each stage as `[INFO] Parsing files: ...` or
/// `[INFO] Building tiles ...`; anything else is ordinary chatter.
fn phase_of(line: &str) -> Option<String> {
    let rest = line.split_once("[INFO] ")?.1;
    const BANNERS: [&str; 8] = [
        "Parsing files",
        "Building",
        "Sorting",
        "Enhancing",
        "Filtering",
        "Validating",
        "Elevation",
        "Cleaning",
    ];
    let banner = BANNERS.iter().find(|b| rest.starts_with(**b))?;
    Some(rest.split(':').next().unwrap_or(banner).trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stage_banners_become_phases() {
        assert_eq!(
            phase_of("2026/08/20 10:00:00.000000 [INFO] Parsing files: data/alps.osm.pbf"),
            Some("Parsing files".to_string())
        );
        assert_eq!(
            phase_of("[INFO] Building tiles from level 0"),
            Some("Building tiles from level 0".to_string())
        );
    }

    /// Ordinary log lines are not phases. Treating every INFO line as one would reset the phase
    /// display several times a second.
    #[test]
    fn chatter_is_not_a_phase() {
        assert_eq!(phase_of("[INFO] 1000 nodes processed"), None);
        assert_eq!(phase_of("[WARN] Building something"), None);
        assert_eq!(phase_of("no marker at all"), None);
    }
}
