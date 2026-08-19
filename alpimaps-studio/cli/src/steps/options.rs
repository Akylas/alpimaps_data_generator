//! `alpimaps options` - what a step accepts, and what happens when it is left alone.

use anyhow::{anyhow, Result};
use clap::Args as ClapArgs;
use studio_core::presets;
use studio_core::steps::options::{self, OptionDef, OptionKind};
use studio_core::steps::StepId;

#[derive(ClapArgs)]
pub struct Args {
    /// Step to describe: basemap or routes.
    pub step: String,
    /// Also list the presets that ship for it.
    #[arg(long)]
    pub presets: bool,
}

pub fn defs_for(step: &str) -> Result<Vec<OptionDef>> {
    match step {
        "basemap" => Ok(options::basemap_options()),
        "routes" => Ok(options::routes_options()),
        other => Err(anyhow!("no options known for step `{other}` (try basemap or routes)")),
    }
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
        let step = match args.step.as_str() {
            "basemap" => StepId::Basemap,
            _ => StepId::Routes,
        };
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
