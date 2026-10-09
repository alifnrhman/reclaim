use crate::model::{Candidate, Risk, ScanWarnings, Source};
use crate::scanner::{fingerprint, tree_size};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

struct CacheSpec {
    path: PathBuf,
    rule_id: &'static str,
    category: &'static str,
    risk: Risk,
    reason: &'static str,
    restore: &'static str,
}

pub fn scan() -> (Vec<Candidate>, ScanWarnings) {
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
        "go-build-cache",
        "Go build cache",
        "go clean -cache",
    );
    add_tool_path(
        &mut specs,
        &home,
        "go",
        &["env", "GOMODCACHE"],
        "go-module-cache",
        "Go module cache",
        "go mod download",
    );
    add_tool_path(
        &mut specs,
        &home,
        "pnpm",
        &["store", "path"],
        "pnpm-store",
        "pnpm package store",
        "pnpm install",
    );

    let mut seen = HashSet::new();
    let mut candidates = Vec::new();
    for spec in specs {
        let Ok(path) = spec.path.canonicalize() else {
            continue;
        };
        if !path.starts_with(&home) || !seen.insert(path.clone()) {
            continue;
        }
        let Ok(metadata) = fs::symlink_metadata(&path) else {
            continue;
        };
        if crate::scanner::is_link_like(&metadata) {
            candidates.push(Candidate {
                id: 0,
                path: Some(path.clone()),
                scope_root: Some(path),
                source: Source::Global,
                rule_id: spec.rule_id,
                category: spec.category,
                bytes: 0,
                files: 0,
                risk: Risk::Protected,
                reason: "Global cache path is a symlink or reparse point".into(),
                restore: spec.restore.into(),
                fingerprint: Some(fingerprint(&metadata)),
                selected: false,
            });
            continue;
        }
        let (bytes, files, local_warnings) = tree_size(&path);
        warnings.0.extend(local_warnings);
        candidates.push(Candidate {
            id: 0,
            path: Some(path.clone()),
            scope_root: Some(path),
            source: Source::Global,
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
    id: &'static str,
    reason: &'static str,
    restore: &'static str,
) {
    let Ok(output) = Command::new(program).args(args).output() else {
        return;
    };
    if !output.status.success() {
        return;
    }
    let value = String::from_utf8_lossy(&output.stdout);
    let path = PathBuf::from(value.trim());
    if path.is_absolute() && path.starts_with(home) {
        specs.push(safe(path, id, reason, restore));
    }
}
