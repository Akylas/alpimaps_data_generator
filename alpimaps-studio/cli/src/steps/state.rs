//! `alpimaps state` - what is already built, and how to forget it.

use anyhow::Result;
use std::collections::BTreeMap;
use clap::{Args as ClapArgs, Subcommand};
use studio_core::settings::Settings;
use studio_core::steps::state::{self, StepStatus};
use studio_core::steps::{StepId, ALL_STEPS};

#[derive(ClapArgs)]
pub struct Args {
    /// Area to report on.
    #[arg(long)]
    pub area: String,
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Subcommand)]
pub enum Command {
    /// Forget the options a step ran with. Its output stays, so it still counts as built.
    Forget {
        /// Step id, e.g. `basemap`. Omit for every step in the area.
        step: Option<String>,
    },
    /// Delete what a step produced, which is what actually makes it run again.
    Clear {
        /// Step id, e.g. `basemap`. Omit for every step in the area.
        step: Option<String>,
    },
}

fn parse_step(raw: &str) -> Result<StepId> {
    serde_json::from_value::<StepId>(serde_json::Value::String(raw.to_string())).map_err(|_| {
        anyhow::anyhow!(
            "unknown step `{raw}`; one of {}",
            ALL_STEPS
                .iter()
                .map(|s| format!("{s:?}").to_lowercase())
                .collect::<Vec<_>>()
                .join(", ")
        )
    })
}

pub fn run(settings: &Settings, args: Args) -> Result<()> {
    let dir = settings.area_dir(&args.area);
    match args.command {
        Some(Command::Forget { step: Some(raw) }) => {
            let step = parse_step(&raw)?;
            state::clear(&dir, step)?;
            println!("forgot the options for {} - its output is still there", step.label());
        }
        Some(Command::Forget { step: None }) => {
            state::clear_all(&dir)?;
            println!("forgot every recorded option set for {}", args.area);
        }
        Some(Command::Clear { step }) => {
            let steps: Vec<_> = match step {
                Some(raw) => vec![parse_step(&raw)?],
                None => ALL_STEPS.to_vec(),
            };
            let mut removed = Vec::new();
            for step in steps {
                removed.extend(state::remove_outputs(&dir, &args.area, step)?);
            }
            if removed.is_empty() {
                println!("nothing to delete in {}", dir.display());
            } else {
                println!("deleted {}", removed.join(", "));
            }
        }
        None => {
            let statuses = state::statuses(&dir, &args.area, &BTreeMap::new());
            for step in ALL_STEPS {
                let line = match statuses.get(&step) {
                    Some(StepStatus::Built { outputs, elapsed, tracked, .. }) => {
                        let files: Vec<String> = outputs
                            .iter()
                            .map(|f| format!("{} {}", f.name, super::mb(f.bytes)))
                            .collect();
                        format!(
                            "built  {}{}{}",
                            files.join(", "),
                            elapsed.as_ref().map(|e| format!(" in {e}")).unwrap_or_default(),
                            if *tracked { "" } else { "  (no record of the options used)" }
                        )
                    }
                    Some(StepStatus::Missing { missing }) => {
                        format!("missing  {}", missing.join(", "))
                    }
                    Some(StepStatus::OptionsChanged { changed, .. }) => {
                        format!("stale  {} changed since it ran", changed.join(", "))
                    }
                    _ => "not tracked here".to_string(),
                };
                println!("  {:<18} {line}", step.label());
            }
        }
    }
    Ok(())
}
