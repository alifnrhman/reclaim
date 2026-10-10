use crate::model::{Candidate, Risk, ScanWarnings, Source};
use crate::process;
use crate::scanner::{fingerprint, tree_size};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

struct CacheSpec {
    path: PathBuf,
    rule_id: &'static str,
    category: &'static str,
    risk: Risk,
    reason: &'static str,
    restore: &'static str,
}

pub fn scan(excluded_roots: &[PathBuf]) -> (Vec<Candidate>, ScanWarnings) {
    let mut warnings = ScanWarnings::default();
    let Some(home) = dirs::home_dir().and_then(|p| p.canonicalize().ok()) else {
        warnings
            .0
            .push("Could not determine the current user's home directory".into());
        return (Vec::new(), warnings);
    };
    let mut specs = fallback_specs(&home);
    add_tool_path(
        &mut specs,
        &home,
        "go",
        &["env", "GOCACHE"],
        ("go-build-cache", "Go build cache", "go clean -cache"),
        excluded_roots,
    );
    add_tool_path(
        &mut specs,
        &home,
        "go",
        &["env", "GOMODCACHE"],
        ("go-module-cache", "Go module cache", "go mod download"),
        excluded_roots,
    );
    add_tool_path(
        &mut specs,
        &home,
        "pnpm",
        &["store", "path"],
        ("pnpm-store", "pnpm package store", "pnpm install"),
        excluded_roots,
    );

    let mut seen = HashSet::new();
    let mut candidates = Vec::new();
    for spec in specs {
        let Ok(metadata) = fs::symlink_metadata(&spec.path) else {
            continue;
        };
        if path_contains_link(&home, &spec.path) {
            candidates.push(protected_cache(
                spec,
                metadata,
                "Global cache path contains a symlink or reparse point",
            ));
            continue;
        }
        let Ok(path) = spec.path.canonicalize() else {
            continue;
        };
        if path == home || !path.starts_with(&home) {
            candidates.push(protected_cache(
                spec,
                metadata,
                "Global cache path resolves outside its permitted home subdirectory",
            ));
            continue;
        }
        if !seen.insert(path.clone()) {
            continue;
        }
        let (bytes, files, local_warnings) = tree_size(&path);
        warnings.0.extend(local_warnings);
        candidates.push(Candidate {
            id: 0,
            path: Some(path.clone()),
            scope_root: Some(path),
            source: Source::Global,
            docker_resource: None,
            external_program: None,
            rule_id: spec.rule_id,
            category: spec.category,
            bytes,
            files,
            risk: spec.risk,
            reason: spec.reason.into(),
            restore: spec.restore.into(),
            fingerprint: Some(fingerprint(&metadata)),
            selected: false,
        });
    }
    (candidates, warnings)
}

fn protected_cache(spec: CacheSpec, metadata: fs::Metadata, reason: &str) -> Candidate {
    Candidate {
        id: 0,
        path: Some(spec.path.clone()),
        scope_root: Some(spec.path),
        source: Source::Global,
        docker_resource: None,
        external_program: None,
        rule_id: spec.rule_id,
        category: spec.category,
        bytes: 0,
        files: 0,
        risk: Risk::Protected,
        reason: reason.into(),
        restore: spec.restore.into(),
        fingerprint: Some(fingerprint(&metadata)),
        selected: false,
    }
}

fn path_contains_link(home: &Path, path: &Path) -> bool {
    let Ok(relative) = path.strip_prefix(home) else {
        return true;
    };
    let mut current = home.to_path_buf();
    for component in relative.components() {
        if !matches!(component, std::path::Component::Normal(_)) {
            return true;
        }
        current.push(component);
        let Ok(metadata) = fs::symlink_metadata(&current) else {
            return true;
        };
        if crate::scanner::is_link_like(&metadata) {
            return true;
        }
    }
    false
}

fn safe(path: PathBuf, id: &'static str, reason: &'static str, restore: &'static str) -> CacheSpec {
    CacheSpec {
        path,
        rule_id: id,
        category: "global cache",
        risk: Risk::Safe,
        reason,
        restore,
    }
}

fn fallback_specs(home: &Path) -> Vec<CacheSpec> {
    #[allow(unused_mut)]
    let mut items = vec![
        safe(
            home.join(".npm/_cacache"),
            "npm-cache",
            "npm's content-addressed download cache",
            "npm cache verify or reinstall dependencies",
        ),
        safe(
            home.join(".bun/install/cache"),
            "bun-cache",
            "Bun's downloaded package cache",
            "bun install",
        ),
        safe(
            home.join(".cache/pip"),
            "pip-cache",
            "pip's download and wheel cache",
            "pip install project dependencies",
        ),
        safe(
            home.join(".cache/yarn"),
            "yarn-cache",
            "Yarn's user cache",
            "yarn install",
        ),
        safe(
            home.join(".cargo/registry"),
            "cargo-registry",
            "Cargo's downloaded registry data",
            "cargo fetch or cargo build",
        ),
        safe(
            home.join(".cargo/git"),
            "cargo-git-cache",
            "Cargo's cached Git dependency repositories and checkouts",
            "cargo fetch or cargo build",
        ),
        safe(
            home.join(".gradle/caches"),
            "gradle-cache",
            "Gradle's user cache",
            "run the Gradle build again",
        ),
        safe(
            home.join(".nuget/packages"),
            "nuget-cache",
            "NuGet's global packages cache",
            "dotnet restore",
        ),
        CacheSpec {
            path: home.join(".m2/repository"),
            rule_id: "maven-local-repository",
            category: "global repository",
            risk: Risk::Review,
            reason: "Maven's local repository may contain artifacts installed only from local projects",
            restore: "mvn dependency:go-offline, if every artifact is available remotely",
        },
    ];
    #[cfg(windows)]
    if let Some(local) = std::env::var_os("LOCALAPPDATA").map(PathBuf::from) {
        items.extend([
            safe(
                local.join("npm-cache/_cacache"),
                "npm-cache",
                "npm's content-addressed download cache",
                "npm cache verify or reinstall dependencies",
            ),
            safe(
                local.join("pip/Cache"),
                "pip-cache",
                "pip's download and wheel cache",
                "pip install project dependencies",
            ),
            safe(
                local.join("NuGet/v3-cache"),
                "nuget-http-cache",
                "NuGet's HTTP cache",
                "dotnet restore",
            ),
        ]);
    }
    items
}

fn add_tool_path(
    specs: &mut Vec<CacheSpec>,
    home: &Path,
    program: &str,
    args: &[&str],
    cache: (&'static str, &'static str, &'static str),
    excluded_roots: &[PathBuf],
) {
    let Ok(mut command) = process::trusted_command(program, excluded_roots) else {
        return;
    };
    let Ok(output) = command.current_dir(home).args(args).output() else {
        return;
    };
    if !output.status.success() {
        return;
    }
    let value = String::from_utf8_lossy(&output.stdout);
    let path = PathBuf::from(value.trim());
    if path.is_absolute() && path.starts_with(home) {
        specs.push(safe(path, cache.0, cache.1, cache.2));
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn detects_a_symlink_in_a_cache_path() {
        use std::os::unix::fs::symlink;

        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let home = std::env::temp_dir().join(format!("reclaim-global-test-{nonce}"));
        let target = home.join("documents");
        fs::create_dir_all(home.join(".npm")).unwrap();
        fs::create_dir(&target).unwrap();
        symlink(&target, home.join(".npm/_cacache")).unwrap();

        assert!(path_contains_link(&home, &home.join(".npm/_cacache")));
        let _ = fs::remove_dir_all(home);
    }
}
