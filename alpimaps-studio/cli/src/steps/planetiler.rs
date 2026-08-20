//! `alpimaps basemap` / `alpimaps routes` - the planetiler-driven steps.

use anyhow::{anyhow, Context, Result};
use clap::Args as ClapArgs;
use std::collections::BTreeMap;
use std::path::PathBuf;
use studio_core::presets::PresetStore;
use studio_core::settings::Settings;
use studio_core::steps::options;
use studio_core::steps::planetiler::{run_cancellable, PlanetilerJob, Schema};
use studio_core::steps::{state, StepEvent, StepId};
use studio_core::toolchain;

#[derive(ClapArgs)]
pub struct Args {
    /// Area to build. Also names the output subdirectory.
    #[arg(long)]
    pub area: String,
    /// Named option set to start from, e.g. `measured`.
    #[arg(long)]
    pub preset: Option<String>,
    /// Option override, repeatable: -o simplify_tolerance=0.7
    #[arg(short = 'o', long = "option", value_name = "KEY=VALUE")]
    pub options: Vec<String>,
    /// Path to the planetiler jar. Defaults to the built one in the submodule.
    #[arg(long)]
    pub jar: Option<PathBuf>,
    /// YAML schema to run instead of the bundled OpenMapTiles fork.
    #[arg(long)]
    pub schema: Option<PathBuf>,
    /// JVM heap in megabytes.
    #[arg(long, default_value_t = 12288)]
    pub heap_mb: u32,
    /// Clip to a .poly.
    #[arg(long)]
    pub polygon: Option<PathBuf>,
    /// Print the command that would run, and stop.
    #[arg(long)]
    pub dry_run: bool,
    /// Rebuild even if this step is recorded as already built for the area.
    #[arg(long)]
    pub force: bool,
    /// Stream planetiler's own output.
    #[arg(short, long)]
    pub verbose: bool,
    /// Extra arguments passed to planetiler verbatim, after `--`.
    #[arg(last = true)]
    pub passthrough: Vec<String>,
}

/// Parse `key=value` overrides, checked against the step's schema.
///
/// An unknown key is refused rather than passed along: planetiler ignores flags it does not
/// know, so a typo would otherwise produce a build that silently used the default.
fn parse_overrides(defs: &[options::OptionDef], raw: &[String]) -> Result<BTreeMap<String, serde_json::Value>> {
    let mut values = BTreeMap::new();
    for entry in raw {
        let (key, value) = entry
            .split_once('=')
            .ok_or_else(|| anyhow!("expected key=value, got `{entry}`"))?;
        let def = options::find(defs, key)
            .ok_or_else(|| anyhow!("unknown option `{key}` (see `alpimaps options <step>`)"))?;
        let parsed = match def.kind {
            options::OptionKind::Bool => serde_json::Value::Bool(
                value.parse().with_context(|| format!("`{key}` wants true or false"))?,
            ),
            options::OptionKind::Int { .. } => serde_json::Value::from(
                value.parse::<i64>().with_context(|| format!("`{key}` wants a whole number"))?,
            ),
            options::OptionKind::Float { .. } => serde_json::Value::from(
                value.parse::<f64>().with_context(|| format!("`{key}` wants a number"))?,
            ),
            _ => serde_json::Value::String(value.to_string()),
        };
        values.insert(key.to_string(), parsed);
    }
    Ok(values)
}

/// One line describing what was found on disk, so "already built" says what it saw.
pub fn describe(status: &state::StepStatus) -> String {
    match status {
        state::StepStatus::Built { outputs, elapsed, .. } => {
            let files: Vec<String> = outputs
                .iter()
                .map(|f| format!("{} {}", f.name, super::mb(f.bytes)))
                .collect();
            match elapsed {
                Some(e) => format!("{}, built in {e}", files.join(", ")),
                None => files.join(", "),
            }
        }
        other => format!("{other:?}"),
    }
}

pub fn human_elapsed(d: std::time::Duration) -> String {
    let secs = d.as_secs();
    if secs >= 3600 {
        format!("{}h{}m", secs / 3600, (secs % 3600) / 60)
    } else if secs >= 60 {
        format!("{}m{}s", secs / 60, secs % 60)
    } else {
        format!("{secs}s")
    }
}

fn default_jar(settings: &Settings) -> Option<PathBuf> {
    let dir = settings.repo_root.join("planetiler/planetiler-dist/target");
    let entries = std::fs::read_dir(dir).ok()?;
    entries
        .flatten()
        .map(|e| e.path())
        .find(|p| p.to_string_lossy().ends_with("-with-deps.jar"))
}

pub async fn run(settings: &Settings, args: Args, routes: bool) -> Result<()> {
    let step = if routes { StepId::Routes } else { StepId::Basemap };
    let defs = if routes { options::routes_options() } else { options::basemap_options() };

    let mut values = BTreeMap::new();
    if let Some(name) = &args.preset {
        let mut store = PresetStore::default();
        for preset in studio_core::presets::builtin() {
            store.upsert(preset);
        }
        let preset = store
            .get(step, name)
            .ok_or_else(|| anyhow!("no preset `{name}` for {}", step.label()))?;
        values.extend(preset.values.clone());
    }
    // explicit -o wins over the preset, so a preset can be used as a starting point
    values.extend(parse_overrides(&defs, &args.options)?);

    let jar = args
        .jar
        .or_else(|| default_jar(settings))
        .ok_or_else(|| anyhow!("no planetiler jar found; pass --jar"))?;
    let java = toolchain::find(None, &settings.repo_root.join(".jre"))
        .await
        .ok_or_else(|| anyhow!("no Java {}+ found", toolchain::MIN_JAVA))?;

    let suffix = if routes { "_routes" } else { "" };
    let mut extra = vec![
        "--download".to_string(),
        format!("--area={}", args.area),
        "--force".to_string(),
    ];
    if let Some(polygon) = &args.polygon {
        extra.push(format!("--polygon={}", polygon.display()));
    }
    extra.extend(options::to_args(&defs, &values));
    extra.extend(args.passthrough.clone());

    let job = PlanetilerJob {
        step,
        area: args.area.clone(),
        java: java.path,
        jar,
        schema: match args.schema {
            Some(path) => Schema::Yaml { path },
            None => Schema::OpenMapTiles,
        },
        heap_mb: args.heap_mb,
        output: settings
            .area_dir(&args.area)
            .join(format!("{}{suffix}.mbtiles", args.area)),
        // its own directory: two planetiler runs sharing one delete each other's sort chunks
        tmp_dir: settings.run_tmp_dir(&format!("{}-{}", args.area, if routes { "routes" } else { "basemap" })),
        extra_args: extra,
        working_dir: settings.repo_root.clone(),
        log_interval: settings.log_interval.clone(),
    };

    if args.dry_run {
        println!("{}", job.command_line().join(" \\\n  "));
        return Ok(());
    }

    // the record is shared with the app, so a step built there is not rebuilt here
    let area_dir = settings.area_dir(&args.area);
    if !args.force {
        let status = state::status(&area_dir, &args.area, step, &values);
        if status.is_fresh() {
            println!(
                "{} is already built for {} ({}) - pass --force to rebuild",
                step.label(),
                args.area,
                describe(&status)
            );
            return Ok(());
        }
    }
    let started = std::time::Instant::now();

    let (tx, mut rx) = tokio::sync::mpsc::channel::<StepEvent>(512);
    let handle = tokio::spawn(run_cancellable(job, tx, tokio::sync::mpsc::channel(1).1));

    let mut last_phase = String::new();
    // kept so a failure can show what planetiler said; printing every line would bury the
    // progress display, and printing none leaves "exited non-zero" and nothing else
    let mut log: Vec<String> = Vec::new();
    while let Some(event) = rx.recv().await {
        match event {
            StepEvent::Phase { name, .. } => {
                last_phase = name;
                println!("[{last_phase}]");
            }
            StepEvent::Progress { label, percent, .. } => {
                print!("\r  {label:>12} {percent:>3}%   ");
                use std::io::Write;
                let _ = std::io::stdout().flush();
            }
            StepEvent::Finished { ok, elapsed, outputs, .. } => {
                println!(
                    "\n{} {}",
                    if ok { "finished" } else { "FAILED" },
                    elapsed.map(|e| format!("in {e}")).unwrap_or_default()
                );
                if !ok {
                    for line in log.iter().rev().take(25).rev() {
                        println!("  {line}");
                    }
                }
                for output in outputs {
                    println!("  {output}");
                }
            }
            StepEvent::Log { line, .. } => {
                if args.verbose {
                    println!("{line}");
                }
                log.push(line);
                if log.len() > 400 {
                    log.remove(0);
                }
            }
            StepEvent::Started { .. } | StepEvent::Skipped { .. } => {}
        }
    }

    match handle.await? {
        Ok(true) => {
            state::mark_done(&area_dir, step, Some(human_elapsed(started.elapsed())), &values)?;
            Ok(())
        }
        Ok(false) => Err(anyhow!("planetiler exited non-zero")),
        Err(e) => Err(e),
    }
}
