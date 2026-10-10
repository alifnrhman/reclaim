use anyhow::{Context, Result, bail};
use reclaim::model::Candidate;
use std::collections::HashSet;
use std::ffi::OsString;
use std::io::{IsTerminal, Write};
use std::path::PathBuf;
use std::time::Duration;

#[derive(Debug)]
struct Cli {
    paths: Vec<PathBuf>,
    no_global: bool,
    no_docker: bool,
}

enum Command {
    Scan(Cli),
    Update,
}

fn main() -> Result<()> {
    let Command::Scan(cli) = parse_args()? else {
        return reclaim::update::install_latest();
    };
    if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
        bail!("reclaim requires an interactive terminal");
    }

    let update_check = reclaim::update::check_async(cli.paths.clone());
    scan_status("projects")?;
    let project = reclaim::scanner::scan_projects(&cli.paths)?;
    let mut candidates = project.candidates;
    let mut warnings = project.warnings;

    if !cli.no_global {
        scan_status("global caches")?;
        let (global, global_warnings) = reclaim::global::scan(&cli.paths);
        candidates.extend(global);
        warnings.0.extend(global_warnings.0);
    }
    if !cli.no_docker {
        scan_status("Docker")?;
        let (docker, docker_warnings) = reclaim::docker::scan(&cli.paths);
        candidates.extend(docker);
        warnings.0.extend(docker_warnings.0);
    }

    candidates = deduplicate(candidates);
    candidates.sort_by_key(|candidate| std::cmp::Reverse(candidate.bytes));
    for (id, candidate) in candidates.iter_mut().enumerate() {
        candidate.id = id;
    }
    eprint!("\r\x1b[2K");
    std::io::stderr().flush()?;
    let update_notice = update_check
        .recv_timeout(Duration::from_millis(250))
        .ok()
        .flatten()
        .map(|version| {
            format!(
                "Update available: {} → {version} · run `reclaim update`",
                env!("CARGO_PKG_VERSION")
            )
        });
    reclaim::tui::run(candidates, warnings, update_notice).context("terminal UI failed")
}

fn scan_status(source: &str) -> Result<()> {
    eprint!("\r\x1b[2K  scanning {source}…");
    std::io::stderr().flush()?;
    Ok(())
}

fn parse_args() -> Result<Command> {
    let arguments: Vec<OsString> = std::env::args_os().skip(1).collect();
    if arguments.len() == 1 && arguments[0] == "update" {
        return Ok(Command::Update);
    }
    let mut paths = Vec::new();
    let mut no_global = false;
    let mut no_docker = false;
    for argument in arguments {
        match argument.to_str() {
            Some("--no-global") => no_global = true,
            Some("--no-docker") => no_docker = true,
            Some("-h" | "--help") => {
                println!(
                    "reclaim {}\n\nUSAGE:\n    reclaim [--no-global] [--no-docker] [PATH]...\n    reclaim update\n\nScans the current directory when PATH is omitted, then user development caches, Docker build cache, and Docker Compose volumes.\nDeletion is permanent and requires a second confirmation keypress.",
                    env!("CARGO_PKG_VERSION")
                );
                std::process::exit(0);
            }
            Some("-V" | "--version") => {
                println!("reclaim {}", env!("CARGO_PKG_VERSION"));
                std::process::exit(0);
            }
            Some(value) if value.starts_with('-') => bail!("unknown option: {value}"),
            _ => paths.push(PathBuf::from(argument)),
        }
    }
    if paths.is_empty() {
        paths.push(std::env::current_dir().context("cannot determine the current directory")?);
    }
    Ok(Command::Scan(Cli {
        paths,
        no_global,
        no_docker,
    }))
}

fn deduplicate(candidates: Vec<Candidate>) -> Vec<Candidate> {
    let mut seen = HashSet::new();
    candidates
        .into_iter()
        .filter(|candidate| {
            candidate.path.as_ref().is_none_or(|path| {
                path.canonicalize()
                    .ok()
                    .is_none_or(|canonical| seen.insert(canonical))
            })
        })
        .collect()
}
