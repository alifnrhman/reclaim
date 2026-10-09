use crate::format::parse_human_bytes;
use crate::model::{Candidate, Risk, ScanWarnings, Source};
use serde_json::Value;
use std::process::Command;

pub fn scan() -> (Vec<Candidate>, ScanWarnings) {
    let mut warnings = ScanWarnings::default();
    let output = match Command::new("docker")
        .args(["system", "df", "--format", "{{json .}}"])
        .output()
    {
        Ok(output) => output,
        Err(_) => return (Vec::new(), warnings),
    };
    if !output.status.success() {
        warnings
            .0
            .push("Docker was found, but its engine is unavailable".into());
        return (Vec::new(), warnings);
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut build_bytes = 0_u64;
    let mut candidates = Vec::new();
    for line in stdout.lines().filter(|line| !line.trim().is_empty()) {
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            warnings
                .0
                .push("Docker returned an unrecognized disk-usage response".into());
            continue;
        };
        let kind = field(&value, &["Type", "type"]).unwrap_or_default();
        if kind.to_ascii_lowercase().contains("build") {
            let reclaimable = field(&value, &["Reclaimable", "reclaimable"]).unwrap_or_default();
            build_bytes = build_bytes.saturating_add(parse_human_bytes(&reclaimable).unwrap_or(0));
        } else if !kind.is_empty() {
            let size = field(&value, &["Size", "size"]).unwrap_or_default();
            let bytes = parse_human_bytes(&size).unwrap_or(0);
            candidates.push(Candidate {
                id: 0,
                path: None,
                scope_root: None,
                source: Source::Docker,
                rule_id: "docker-protected-resource",
                category: docker_category(&kind),
                bytes,
                files: 0,
                risk: Risk::Protected,
                reason: "Only Docker build cache is eligible for deletion in this version".into(),
                restore: "Protected: reclaim does not remove this Docker resource".into(),
                fingerprint: None,
                selected: false,
            });
        }
    }
    if build_bytes == 0 {
        return (candidates, warnings);
    }
    let candidate = Candidate {
        id: 0,
        path: None,
        scope_root: None,
        source: Source::Docker,
        rule_id: "docker-build-cache",
        category: "build cache",
        bytes: build_bytes,
        files: 0,
        risk: Risk::Safe,
        reason: "Docker reports this build cache as reclaimable".into(),
        restore: "Docker rebuilds layers during the next image build".into(),
        fingerprint: None,
        selected: false,
    };
    candidates.push(candidate);
    (candidates, warnings)
}

fn docker_category(kind: &str) -> &'static str {
    let lower = kind.to_ascii_lowercase();
    if lower.contains("image") {
        "images"
    } else if lower.contains("container") {
        "containers"
    } else if lower.contains("volume") {
        "volumes"
    } else {
        "resources"
    }
}

fn field(value: &Value, names: &[&str]) -> Option<String> {
    names
        .iter()
        .find_map(|name| value.get(*name))
        .and_then(|v| {
            v.as_str()
                .map(ToOwned::to_owned)
                .or_else(|| Some(v.to_string()))
        })
}

pub fn prune_build_cache() -> Result<String, String> {
    let output = Command::new("docker")
        .args(["builder", "prune", "--force"])
        .output()
        .map_err(|error| format!("could not start Docker: {error}"))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
    } else {
        let error = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        Err(if error.is_empty() {
            "Docker prune failed".into()
        } else {
            error
        })
    }
}
