use std::env;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Resolve a helper executable without accepting relative PATH entries or
/// executables located inside a directory that reclaim is about to scan.
pub fn trusted_command(program: &str, excluded_roots: &[PathBuf]) -> Result<Command, String> {
    let executable = trusted_program(program, excluded_roots)?;
    Ok(Command::new(executable))
}

pub fn trusted_program(program: &str, excluded_roots: &[PathBuf]) -> Result<PathBuf, String> {
    let path = env::var_os("PATH").ok_or_else(|| "PATH is not set".to_owned())?;
    trusted_program_in_path(program, excluded_roots, &path)
}

fn trusted_program_in_path(
    program: &str,
    excluded_roots: &[PathBuf],
    path: &std::ffi::OsStr,
) -> Result<PathBuf, String> {
    let roots: Vec<_> = excluded_roots
        .iter()
        .filter_map(|path| path.canonicalize().ok())
        .collect();
    for directory in env::split_paths(path) {
        if !directory.is_absolute() {
            continue;
        }
        let Ok(directory) = directory.canonicalize() else {
            continue;
        };
        if roots.iter().any(|root| directory.starts_with(root)) {
            continue;
        }
        for name in executable_names(program) {
            let candidate = directory.join(name);
            if !is_executable(&candidate) {
                continue;
            }
            let Ok(candidate) = candidate.canonicalize() else {
                continue;
            };
            if roots.iter().any(|root| candidate.starts_with(root)) {
                continue;
            }
            return Ok(candidate);
        }
    }

    Err(format!(
        "could not find a trusted {program} executable outside the scan roots"
    ))
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;

    path.metadata()
        .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
}

#[cfg(windows)]
fn is_executable(path: &Path) -> bool {
    path.is_file()
}

fn executable_names(program: &str) -> Vec<OsString> {
    #[cfg(windows)]
    {
        let path = Path::new(program);
        if path.extension().is_some() {
            return vec![program.into()];
        }
        let extensions = env::var_os("PATHEXT").unwrap_or_else(|| ".COM;.EXE;.BAT;.CMD".into());
        extensions
            .to_string_lossy()
            .split(';')
            .filter(|extension| !extension.is_empty())
            .map(|extension| format!("{program}{extension}"))
            .map(OsString::from)
            .collect()
    }
    #[cfg(not(windows))]
    {
        vec![program.into()]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn rejects_programs_inside_an_excluded_root() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = env::temp_dir().join(format!("reclaim-path-test-{nonce}"));
        fs::create_dir(&root).unwrap();
        let program = root.join(if cfg!(windows) {
            "helper.exe"
        } else {
            "helper"
        });
        fs::write(&program, "not executable").unwrap();

        let path = env::join_paths([&root]).unwrap();
        let result = trusted_program_in_path("helper", std::slice::from_ref(&root), &path);
        let _ = fs::remove_file(program);
        let _ = fs::remove_dir(root);

        assert!(result.is_err());
    }
}
