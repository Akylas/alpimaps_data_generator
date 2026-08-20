pub mod catalog;
pub mod options;
pub mod planetiler;
pub mod profile;
pub mod serve;
pub mod state;
pub mod terrain;
pub mod tools;
pub mod valhalla;

use anyhow::Result;
use std::path::PathBuf;
use studio_core::settings::Settings;

/// Paths for a run: the repository layout, with every one of them overridable on the line.
///
/// The CLI deliberately does not read the GUI's saved settings, and nothing here is required -
/// `--repo` only supplies defaults. A command line should do what it says on the line, not what
/// a window was last configured to do.
pub fn settings_for(cli: &crate::Cli) -> Result<Settings> {
    let repo = cli.repo.canonicalize().unwrap_or_else(|_| cli.repo.clone());
    let mut settings = Settings::for_repo(repo);
    if let Some(path) = cli.output_root.clone() {
        settings.output_root = path;
    }
    if let Some(path) = cli.data_dir.clone() {
        settings.data_dir = path;
    }
    if let Some(path) = cli.elevation_dir.clone() {
        settings.elevation_tiles_dir = path;
    }
    if let Some(path) = cli.sources_json.clone() {
        settings.sources_json = path;
    }
    if let Some(path) = cli.valhalla_bin.clone() {
        settings.valhalla_bin_dir = Some(path);
    }
    if let Some(path) = cli.valhalla_config.clone() {
        settings.valhalla_config = Some(path);
    }
    Ok(settings)
}

/// Format a byte count for a terminal.
pub fn mb(bytes: u64) -> String {
    let mb = bytes as f64 / 1_048_576.0;
    if mb >= 1024.0 {
        format!("{:.2} GB", mb / 1024.0)
    } else {
        format!("{mb:.1} MB")
    }
}
