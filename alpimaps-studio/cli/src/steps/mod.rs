pub mod catalog;
pub mod options;
pub mod planetiler;
pub mod profile;
pub mod serve;
pub mod terrain;
pub mod valhalla;

use anyhow::Result;
use std::path::PathBuf;
use studio_core::settings::Settings;

/// Settings for a run, from the repository layout plus any overrides.
///
/// The CLI deliberately does not read the GUI's saved settings: a command line should do what it
/// says on the line, not what a window was last configured to do.
pub fn settings_for(repo: &std::path::Path, output: Option<PathBuf>) -> Result<Settings> {
    let repo = repo.canonicalize().unwrap_or_else(|_| repo.to_path_buf());
    let mut settings = Settings::for_repo(repo);
    if let Some(output) = output {
        settings.output_root = output;
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
