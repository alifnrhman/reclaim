use anyhow::{Context, Result, bail};
use reclaim::model::Candidate;
use std::collections::HashSet;
use std::io::{IsTerminal, Write};
use std::path::PathBuf;

#[derive(Debug)]
struct Cli {
    paths: Vec<PathBuf>,
    no_global: bool,
    no_docker: bool,
}

fn main() -> Result<()> {
    let cli = parse_args()?;
    if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
        bail!("reclaim requires an interactive terminal");
    }

    scan_status("projects")?;
    let project = reclaim::scanner::scan_projects(&cli.paths)?;
    let mut candidates = project.candidates;
    let mut warnings = project.warnings;

    if !cli.no_global {
        scan_status("global caches")?;
        let (global, global_warnings) = reclaim::global::scan();
        candidates.extend(global);
        warnings.0.extend(global_warnings.0);
    }
    if !cli.no_docker {
        scan_status("Docker")?;
        let (docker, docker_warnings) = reclaim::docker::scan();
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
    reclaim::tui::run(candidates, warnings).context("terminal UI failed")
}

fn scan_status(source: &str) -> Result<()> {
    eprint!("\r\x1b[2K  scanning {source}…");
    std::io::stderr().flush()?;
    Ok(())
}

fn parse_args() -> Result<Cli> {
    let mut paths = Vec::new();
    let mut no_global = false;
    let mut no_docker = false;
    for argument in std::env::args_os().skip(1) {
        match argument.to_str() {
            Some("--no-global") => no_global = true,
            Some("--no-docker") => no_docker = true,
            Some("-h" | "--help") => {
                println!(
                    "reclaim 0.1.0\n\nUSAGE:\n    reclaim [--no-global] [--no-docker] <PATH>...\n\nScans explicit project paths, then user development caches and Docker build cache.\nDeletion is permanent and requires typed confirmation."
                );
                std::process::exit(0);
            }
            Some("-V" | "--version") => {
                println!("reclaim 0.1.0");
                std::process::exit(0);
            }
            Some(value) if value.starts_with('-') => bail!("unknown option: {value}"),
            _ => paths.push(PathBuf::from(argument)),
        }
    }
    if paths.is_empty() {
        bail!("at least one scan PATH is required; run reclaim --help");
    }
    Ok(Cli {
        paths,
        no_global,
        no_docker,
    })
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
