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
    // `--repo .` from inside the workspace should still find the checkout above it
    let mut settings = Settings::for_repo(Settings::locate_repo(&repo));
    // when this binary ships inside the app bundle, the jar and valhalla.json sit beside it;
    // in a checkout there is nothing there and the repository paths take over
    settings.resource_dir = std::env::current_exe().ok().and_then(|exe| {
        exe.parent().map(|dir| dir.to_path_buf())
    });
    if let Some(path) = cli.output_root.clone() {
        settings.output_root = path;
    }
    if let Some(path) = cli.data_dir.clone() {
        settings.data_dir = path;
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
