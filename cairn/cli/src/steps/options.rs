//! `cairn options` - what a step accepts, and what happens when it is left alone.

use anyhow::{anyhow, Result};
use clap::Args as ClapArgs;
use cairn_core::presets;
use cairn_core::steps::options::{self, OptionDef, OptionKind};
use cairn_core::steps::StepId;

#[derive(ClapArgs)]
pub struct Args {
    /// Step to describe, by its subcommand name: basemap, routes, bathymap, terrain, package.
    pub step: String,
    /// Also list the presets that ship for it.
    #[arg(long)]
    pub presets: bool,
}

/// Resolve a step by the name its subcommand uses, so `cairn options X` and `cairn X` agree.
pub fn step_by_name(name: &str) -> Option<StepId> {
    cairn_core::steps::ALL_STEPS.into_iter().find(|s| s.command() == name)
}

pub fn defs_for(step: &str) -> Result<Vec<OptionDef>> {
    let Some(id) = step_by_name(step) else {
        let known: Vec<&str> = cairn_core::steps::ALL_STEPS
            .iter()
            .filter(|s| !options::for_step(**s).is_empty())
            .map(|s| s.command())
            .collect();
        return Err(anyhow!("no step named `{step}` (try {})", known.join(", ")));
    };
    let defs = options::for_step(id);
    if defs.is_empty() {
        return Err(anyhow!("`{step}` takes no per-run options"));
    }
    Ok(defs)
}

fn kind_label(kind: &OptionKind) -> String {
    match kind {
        OptionKind::Bool => "bool".into(),
        OptionKind::Int { .. } => "int".into(),
        OptionKind::Float { .. } => "float".into(),
        OptionKind::Text => "text".into(),
        OptionKind::Choice { choices } => choices.join("|"),
    }
}

pub fn run(args: Args) -> Result<()> {
    let defs = defs_for(&args.step)?;
    let mut group = String::new();
    for def in &defs {
        if def.group != group {
            group = def.group.clone();
            println!("\n{}", group.to_uppercase());
        }
        println!("  {:<34} {}", format!("{} <{}>", def.key, kind_label(&def.kind)), def.label);
        println!("      {}", def.help);
        // the hint is what planetiler does when the flag is absent; the schema never asserts it
        println!("      unset -> {}", def.hint);
    }

    if args.presets {
        let step = step_by_name(&args.step).unwrap_or(StepId::Basemap);
        println!("\nPRESETS");
        for preset in presets::builtin().into_iter().filter(|p| p.step == step) {
            println!("  {:<12} {}", preset.name, preset.description);
            for (key, value) in &preset.values {
                println!("      {key} = {value}");
            }
        }
    }
    Ok(())
}
