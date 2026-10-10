use crate::process;
use anyhow::{Context, Result, bail};
use std::fs;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

const PACKAGE: &str = "@alifnrhman/reclaim";
const CACHE_MAX_AGE: Duration = Duration::from_secs(24 * 60 * 60);

pub fn check_async(excluded_roots: Vec<PathBuf>) -> Receiver<Option<String>> {
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let latest = fresh_cached_version().or_else(|| {
            let version = fetch_latest_version(&excluded_roots)?;
            write_cached_version(&version);
            Some(version)
        });
        let newer = latest.filter(|version| is_newer(env!("CARGO_PKG_VERSION"), version));
        let _ = sender.send(newer);
    });
    receiver
}

pub fn install_latest() -> Result<()> {
    println!("Updating {PACKAGE}…");
    let excluded = std::env::current_dir().into_iter().collect::<Vec<_>>();
    let mut command = process::trusted_command(npm_program(), &excluded)
        .map_err(anyhow::Error::msg)
        .context("could not locate a trusted npm executable")?;
    let status = command
        .current_dir(trusted_working_directory())
        .args([
            "install",
            "--global",
            "@alifnrhman/reclaim@latest",
            "--registry=https://registry.npmjs.org",
        ])
        .status()
        .context(
            "could not start npm; install Node.js/npm or update the standalone binary manually",
        )?;
    if !status.success() {
        bail!("npm update failed with status {status}");
    }
    println!("Updated. Run `reclaim --version` to verify the installed version.");
    Ok(())
}

fn fetch_latest_version(excluded_roots: &[PathBuf]) -> Option<String> {
    let mut command = process::trusted_command(npm_program(), excluded_roots).ok()?;
    let output = command
        .current_dir(trusted_working_directory())
        .args([
            "view",
            PACKAGE,
            "version",
            "--json",
            "--registry=https://registry.npmjs.org",
        ])
        .env("npm_config_fetch_retries", "0")
        .env("npm_config_fetch_timeout", "1500")
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    serde_json::from_slice::<String>(&output.stdout).ok()
}

fn fresh_cached_version() -> Option<String> {
    let path = cache_path()?;
    let age = fs::metadata(&path).ok()?.modified().ok()?.elapsed().ok()?;
    if age > CACHE_MAX_AGE {
        return None;
    }
    let version = fs::read_to_string(path).ok()?;
    Some(version.trim().to_owned())
}

fn write_cached_version(version: &str) {
    let Some(path) = cache_path() else {
        return;
    };
    let Some(parent) = path.parent() else {
        return;
    };
    if fs::create_dir_all(parent).is_ok() {
        let _ = fs::write(path, version);
    }
}

fn cache_path() -> Option<PathBuf> {
    Some(dirs::cache_dir()?.join("reclaim").join("latest-version"))
}

fn trusted_working_directory() -> PathBuf {
    dirs::home_dir()
        .and_then(|path| path.canonicalize().ok())
        .unwrap_or_else(std::env::temp_dir)
}

fn is_newer(current: &str, latest: &str) -> bool {
    match (numeric_version(current), numeric_version(latest)) {
        (Some(current), Some(latest)) => latest > current,
        _ => false,
    }
}

fn numeric_version(value: &str) -> Option<[u64; 3]> {
    let core = value
        .trim()
        .trim_start_matches('v')
        .split(['-', '+'])
        .next()?;
    let mut parts = core.split('.').map(str::parse::<u64>);
    let version = [
        parts.next()?.ok()?,
        parts.next()?.ok()?,
        parts.next()?.ok()?,
    ];
    if parts.next().is_some() {
        return None;
    }
    Some(version)
}

#[cfg(windows)]
fn npm_program() -> &'static str {
    "npm.cmd"
}

#[cfg(not(windows))]
fn npm_program() -> &'static str {
    "npm"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compares_release_versions_numerically() {
        assert!(is_newer("0.1.5", "0.1.6"));
        assert!(is_newer("0.9.9", "0.10.0"));
        assert!(!is_newer("1.2.3", "1.2.3"));
        assert!(!is_newer("2.0.0", "1.99.99"));
    }
}
