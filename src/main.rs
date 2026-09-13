mod config;
mod plan;
mod scan;
mod ui;

use anyhow::Result;
use clap::Parser;
use config::{Config, Mode};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "trashy", version, about = "Plans, prompts, and executes a cleanse of your trashy filesystem(s)")]
struct Cli {
    #[arg(short, long, help = "config file (else $TRASHY_CONFIG, ./trashy.yaml, then the user config dir)")]
    config: Option<PathBuf>,
    #[arg(short, long, value_name = "MOUNT", help = "target a mount point (persisted)")]
    target: Vec<String>,
    #[arg(short, long, value_name = "MOUNT", help = "untarget a mount point (persisted)")]
    untarget: Vec<String>,
    #[arg(short, long, help = "trash or delete (persisted)")]
    mode: Option<Mode>,
    #[arg(long, hide = true)]
    df: Option<PathBuf>,
    #[arg(value_name = "DIR", help = "scan these directories for this session instead of the cwd's filesystem")]
    dirs: Vec<PathBuf>,
}

fn norm(m: String) -> String { let t = m.trim_end_matches('/'); if t.is_empty() { "/".into() } else { t.into() } }

fn main() -> Result<()> {
    let cli = Cli::parse();
    if let Some(p) = cli.df {
        let Some((t, u, a)) = scan::statvfs(&p) else { std::process::exit(1) };
        println!("{t} {u} {a}");
        return Ok(());
    }
    rayon::ThreadPoolBuilder::new().stack_size(32 << 20).build_global()?;
    let mut cfg = Config::load(cli.config)?;
    let dirty = !cli.target.is_empty() || !cli.untarget.is_empty() || cli.mode.is_some();
    for t in cli.target { cfg.targets.insert(norm(t), true); }
    for t in cli.untarget { cfg.targets.insert(norm(t), false); }
    if let Some(m) = cli.mode { cfg.mode = m; }
    if dirty { cfg.save()?; }
    let mounts = scan::mounts();
    ui::run(ui::App::new(cfg, mounts, cli.dirs))?;
    std::process::exit(0)
}
