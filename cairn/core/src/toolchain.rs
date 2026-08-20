//! Finding and, if necessary, fetching a JRE.
//!
//! Planetiler runs as a subprocess, so the app needs *a* Java 21+ on disk but does not care
//! where it came from. Resolution order is explicit-override, then app-managed, then the
//! machine's own - so a user with Homebrew Java pays no download, and a user with none gets one
//! without being told to install anything.

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Stdio;

/// Minimum planetiler will run on.
pub const MIN_JAVA: u32 = 21;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JavaInstall {
    pub path: PathBuf,
    pub version: u32,
    pub source: JavaSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JavaSource {
    /// Explicitly configured in settings.
    Configured,
    /// Downloaded by us into the app data dir.
    Managed,
    /// `$JAVA_HOME/bin/java`.
    JavaHome,
    /// Found on `$PATH`.
    Path,
}

/// Parse the feature version out of `java -version` output.
///
/// The interesting part is that this goes to **stderr**, not stdout, and the shape differs
/// between vendors and eras:
///   openjdk version "21.0.11" 2026-04-21   -> 21
///   java version "1.8.0_452"               -> 8   (legacy 1.x scheme)
fn parse_version(output: &str) -> Option<u32> {
    let quoted = output.split('"').nth(1)?;
    let mut parts = quoted.split(['.', '_', '-']);
    let first: u32 = parts.next()?.parse().ok()?;
    if first == 1 {
        parts.next()?.parse().ok()
    } else {
        Some(first)
    }
}

/// Run `java -version` against a candidate binary and read its feature version.
pub async fn probe(java: &Path) -> Result<u32> {
    let out = tokio::process::Command::new(java)
        .arg("-version")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .with_context(|| format!("running {} -version", java.display()))?;
    // vendors print the banner to stderr; check both rather than assume
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stderr),
        String::from_utf8_lossy(&out.stdout)
    );
    parse_version(&text).ok_or_else(|| anyhow!("could not parse version from: {text}"))
}

fn exe(dir: &Path) -> PathBuf {
    let name = if cfg!(windows) { "java.exe" } else { "java" };
    dir.join("bin").join(name)
}

/// Resolve a usable Java, or `None` if nothing on this machine qualifies.
///
/// `configured` is the settings override, `managed_root` the directory we download into.
pub async fn find(configured: Option<&Path>, managed_root: &Path) -> Option<JavaInstall> {
    let mut candidates: Vec<(PathBuf, JavaSource)> = Vec::new();

    if let Some(home) = configured {
        // accept either a JAVA_HOME-style dir or the binary itself
        candidates.push((exe(home), JavaSource::Configured));
        candidates.push((home.to_path_buf(), JavaSource::Configured));
    }
    if let Some(managed) = managed_home(managed_root) {
        candidates.push((exe(&managed), JavaSource::Managed));
    }
    if let Ok(home) = std::env::var("JAVA_HOME") {
        candidates.push((exe(Path::new(&home)), JavaSource::JavaHome));
    }
    candidates.push((PathBuf::from("java"), JavaSource::Path));

    for (path, source) in candidates {
        let Ok(version) = probe(&path).await else { continue };
        if version >= MIN_JAVA {
            return Some(JavaInstall { path, version, source });
        }
    }
    None
}

/// The single extracted JDK directory under `managed_root`, if we have one.
fn managed_home(managed_root: &Path) -> Option<PathBuf> {
    let entries = std::fs::read_dir(managed_root).ok()?;
    for entry in entries.flatten() {
        let dir = entry.path();
        if !dir.is_dir() {
            continue;
        }
        // macOS bundles the runtime under Contents/Home; Linux and Windows do not
        for candidate in [dir.join("Contents/Home"), dir.clone()] {
            if exe(&candidate).exists() {
                return Some(candidate);
            }
        }
    }
    None
}

/// Adoptium's binary redirect endpoint for the current host.
pub fn adoptium_url(feature_version: u32) -> Result<String> {
    let os = match std::env::consts::OS {
        "macos" => "mac",
        "linux" => "linux",
        "windows" => "windows",
        other => bail!("unsupported OS for JRE download: {other}"),
    };
    let arch = match std::env::consts::ARCH {
        "x86_64" => "x64",
        "aarch64" => "aarch64",
        other => bail!("unsupported arch for JRE download: {other}"),
    };
    Ok(format!(
        "https://api.adoptium.net/v3/binary/latest/{feature_version}/ga/{os}/{arch}/jre/hotspot/normal/eclipse"
    ))
}

/// Download and extract a JRE into `managed_root`, reporting bytes as they arrive.
///
/// `on_progress` receives `(downloaded, total)`; total is `None` when the server omits
/// Content-Length, which Adoptium's redirect sometimes does.
pub async fn download<F>(managed_root: &Path, mut on_progress: F) -> Result<JavaInstall>
where
    F: FnMut(u64, Option<u64>),
{
    use futures_util::StreamExt;
    use tokio::io::AsyncWriteExt;

    tokio::fs::create_dir_all(managed_root).await?;
    let url = adoptium_url(MIN_JAVA)?;
    let response = reqwest::get(&url).await?.error_for_status()?;
    let total = response.content_length();

    let archive = managed_root.join("jre-download.tar.gz");
    let mut file = tokio::fs::File::create(&archive).await?;
    let mut stream = response.bytes_stream();
    let mut received: u64 = 0;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        received += chunk.len() as u64;
        file.write_all(&chunk).await?;
        on_progress(received, total);
    }
    file.flush().await?;
    drop(file);

    // tar+gzip extraction is sync and CPU-bound; keep it off the async runtime
    let root = managed_root.to_path_buf();
    let archive_for_task = archive.clone();
    tokio::task::spawn_blocking(move || -> Result<()> {
        let reader = std::fs::File::open(&archive_for_task)?;
        let decoder = flate2::read::GzDecoder::new(reader);
        tar::Archive::new(decoder).unpack(&root)?;
        Ok(())
    })
    .await??;
    let _ = tokio::fs::remove_file(&archive).await;

    let home = managed_home(managed_root)
        .ok_or_else(|| anyhow!("extracted archive contained no bin/java"))?;
    let path = exe(&home);
    let version = probe(&path).await?;
    Ok(JavaInstall { path, version, source: JavaSource::Managed })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_modern_version() {
        assert_eq!(
            parse_version("openjdk version \"21.0.11\" 2026-04-21\nOpenJDK Runtime Environment"),
            Some(21)
        );
    }

    #[test]
    fn parses_bare_major() {
        assert_eq!(parse_version("openjdk version \"25\" 2026-09-16"), Some(25));
    }

    /// Java 8 and earlier used the `1.x` scheme, where the feature version is the second field.
    #[test]
    fn parses_legacy_scheme() {
        assert_eq!(parse_version("java version \"1.8.0_452\""), Some(8));
    }

    #[test]
    fn rejects_garbage() {
        assert_eq!(parse_version("command not found"), None);
    }

    #[test]
    fn builds_a_host_url() {
        let url = adoptium_url(21).unwrap();
        assert!(url.contains("/21/ga/"), "{url}");
        assert!(url.ends_with("/jre/hotspot/normal/eclipse"), "{url}");
    }
}
