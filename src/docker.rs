use crate::format::parse_human_bytes;
use crate::model::{Candidate, DockerResource, Risk, ScanWarnings, Source};
use serde_json::Value;
use std::process::{Command, Output};

const COMPOSE_PROJECT_LABEL: &str = "com.docker.compose.project";
const COMPOSE_VOLUME_LABEL: &str = "com.docker.compose.volume";
const VOLUME_FORMAT: &str = "{{range .Volumes}}{{.Name}}\t{{.Links}}\t{{.Size}}\n{{end}}";

#[derive(Debug, Clone, PartialEq, Eq)]
struct VolumeUsage {
    name: String,
    links: u64,
    bytes: u64,
}

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
    let mut volume_summary = None;
    let mut candidates = Vec::new();
    for line in stdout.lines().filter(|line| !line.trim().is_empty()) {
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            warnings
                .0
                .push("Docker returned an unrecognized disk-usage response".into());
            continue;
        };
        let kind = field(&value, &["Type", "type"]).unwrap_or_default();
        let kind_lower = kind.to_ascii_lowercase();
        if kind_lower.contains("build") {
            let reclaimable = field(&value, &["Reclaimable", "reclaimable"]).unwrap_or_default();
            build_bytes = build_bytes.saturating_add(parse_human_bytes(&reclaimable).unwrap_or(0));
        } else if kind_lower.contains("volume") {
            let size = field(&value, &["Size", "size"]).unwrap_or_default();
            let count = field(&value, &["TotalCount", "total"])
                .and_then(|value| value.parse().ok())
                .unwrap_or(0);
            volume_summary = Some((count, parse_human_bytes(&size).unwrap_or(0)));
        } else if !kind.is_empty() {
            let size = field(&value, &["Size", "size"]).unwrap_or_default();
            candidates.push(protected_summary(
                &kind,
                parse_human_bytes(&size).unwrap_or(0),
            ));
        }
    }

    let expected_volumes = volume_summary.map(|(count, _)| count).unwrap_or(0);
    match scan_volumes(expected_volumes) {
        Ok(volumes) => candidates.extend(volumes),
        Err(error) => {
            warnings.0.push(error);
            if let Some((_, bytes)) = volume_summary {
                candidates.push(protected_summary("volumes", bytes));
            }
        }
    }

    if build_bytes > 0 {
        candidates.push(Candidate {
            id: 0,
            path: None,
            scope_root: None,
            source: Source::Docker,
            docker_resource: Some(DockerResource::BuildCache),
            rule_id: "docker-build-cache",
            category: "build cache",
            bytes: build_bytes,
            files: 0,
            risk: Risk::Safe,
            reason: "Docker reports this build cache as reclaimable".into(),
            restore: "Docker rebuilds layers during the next image build".into(),
            fingerprint: None,
            selected: false,
        });
    }
    (candidates, warnings)
}

fn scan_volumes(expected_count: usize) -> Result<Vec<Candidate>, String> {
    let output = Command::new("docker")
        .args(["system", "df", "--verbose", "--format", VOLUME_FORMAT])
        .output()
        .map_err(|error| format!("Could not inspect Docker volume sizes: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "Could not inspect Docker volume sizes: {}",
            output_error(&output, "Docker disk-usage query failed")
        ));
    }
    let usages = parse_volume_usages(&String::from_utf8_lossy(&output.stdout))?;
    if usages.len() != expected_count {
        return Err("Docker volume inventory changed during the scan; run reclaim again".into());
    }
    if usages.is_empty() {
        return Ok(Vec::new());
    }

    let mut inspected = Vec::new();
    for chunk in usages.chunks(100) {
        let mut command = Command::new("docker");
        command.args(["volume", "inspect"]);
        command.args(chunk.iter().map(|usage| usage.name.as_str()));
        let output = command
            .output()
            .map_err(|error| format!("Could not inspect Docker volume labels: {error}"))?;
        if !output.status.success() {
            return Err(format!(
                "Could not inspect Docker volume labels: {}",
                output_error(&output, "Docker volume inspect failed")
            ));
        }
        let mut values: Vec<Value> = serde_json::from_slice(&output.stdout)
            .map_err(|_| "Docker returned unrecognized volume metadata".to_owned())?;
        inspected.append(&mut values);
    }

    Ok(build_volume_candidates(&usages, &inspected))
}

fn parse_volume_usages(stdout: &str) -> Result<Vec<VolumeUsage>, String> {
    stdout
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            let fields: Vec<_> = line.split('\t').collect();
            if fields.len() != 3 {
                return Err("Docker returned unrecognized volume disk usage".into());
            }
            let name = fields[0].trim();
            let links = fields[1]
                .trim()
                .parse()
                .map_err(|_| "Docker returned an invalid volume reference count".to_owned())?;
            let bytes = parse_human_bytes(fields[2].trim())
                .ok_or_else(|| "Docker returned an invalid volume size".to_owned())?;
            if name.is_empty() {
                return Err("Docker returned a volume without a name".into());
            }
            Ok(VolumeUsage {
                name: name.to_owned(),
                links,
                bytes,
            })
        })
        .collect()
}

fn build_volume_candidates(usages: &[VolumeUsage], inspected: &[Value]) -> Vec<Candidate> {
    usages
        .iter()
        .filter_map(|usage| {
            let metadata = inspected
                .iter()
                .find(|value| value.get("Name").and_then(Value::as_str) == Some(&usage.name))?;
            Some(volume_candidate(usage, metadata))
        })
        .collect()
}

fn volume_candidate(usage: &VolumeUsage, metadata: &Value) -> Candidate {
    let project = label(metadata, COMPOSE_PROJECT_LABEL);
    let logical_name = label(metadata, COMPOSE_VOLUME_LABEL);
    let compose_identity = project.zip(logical_name);
    let eligible = compose_identity.is_some() && usage.links == 0;
    let (reason, restore) = if eligible {
        (
            "Unused Docker Compose volume; deleting it permanently removes its stored data".into(),
            "Data cannot be regenerated automatically; restore it from a backup if needed".into(),
        )
    } else if usage.links > 0 {
        (
            format!("Referenced by {} Docker container(s)", usage.links),
            "Protected: remove every referencing container before reviewing this volume".into(),
        )
    } else {
        (
            "Volume is not attributable to a Docker Compose project".into(),
            "Protected: reclaim does not remove anonymous or unrecognized volumes".into(),
        )
    };
    let docker_resource = compose_identity
        .map(|(project, logical_name)| DockerResource::ComposeVolume {
            name: usage.name.clone(),
            project: project.to_owned(),
            logical_name: logical_name.to_owned(),
        })
        .unwrap_or_else(|| DockerResource::Volume {
            name: usage.name.clone(),
        });
    Candidate {
        id: 0,
        path: None,
        scope_root: None,
        source: Source::Docker,
        docker_resource: Some(docker_resource),
        rule_id: if eligible {
            "docker-compose-volume"
        } else {
            "docker-protected-volume"
        },
        category: "volume",
        bytes: usage.bytes,
        files: 0,
        risk: if eligible {
            Risk::Review
        } else {
            Risk::Protected
        },
        reason,
        restore,
        fingerprint: None,
        selected: false,
    }
}

fn protected_summary(kind: &str, bytes: u64) -> Candidate {
    Candidate {
        id: 0,
        path: None,
        scope_root: None,
        source: Source::Docker,
        docker_resource: None,
        rule_id: "docker-protected-resource",
        category: docker_category(kind),
        bytes,
        files: 0,
        risk: Risk::Protected,
        reason: "This Docker resource is not eligible for deletion".into(),
        restore: "Protected: reclaim does not remove this Docker resource".into(),
        fingerprint: None,
        selected: false,
    }
}

fn label<'a>(metadata: &'a Value, name: &str) -> Option<&'a str> {
    metadata
        .get("Labels")?
        .as_object()?
        .get(name)?
        .as_str()
        .filter(|value| !value.is_empty())
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
    names.iter().find_map(|name| value.get(*name)).map(|value| {
        value
            .as_str()
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| value.to_string())
    })
}

pub fn delete(candidate: &Candidate) -> Result<String, String> {
    match &candidate.docker_resource {
        Some(DockerResource::BuildCache) => prune_build_cache(),
        Some(DockerResource::ComposeVolume {
            name,
            project,
            logical_name,
        }) if candidate.risk == Risk::Review => remove_compose_volume(name, project, logical_name),
        Some(DockerResource::ComposeVolume { .. } | DockerResource::Volume { .. }) => {
            Err("this Docker volume is protected from deletion".into())
        }
        None => Err("this Docker resource is protected from deletion".into()),
    }
}

fn remove_compose_volume(name: &str, project: &str, logical_name: &str) -> Result<String, String> {
    let output = Command::new("docker")
        .args(["volume", "inspect", name])
        .output()
        .map_err(|error| format!("could not start Docker: {error}"))?;
    if !output.status.success() {
        return Err(output_error(&output, "Docker volume inspect failed"));
    }
    let inspected: Vec<Value> = serde_json::from_slice(&output.stdout)
        .map_err(|_| "Docker returned unrecognized volume metadata".to_owned())?;
    let metadata = inspected
        .first()
        .ok_or("Docker volume disappeared before deletion")?;
    let identity_matches = metadata.get("Name").and_then(Value::as_str) == Some(name)
        && label(metadata, COMPOSE_PROJECT_LABEL) == Some(project)
        && label(metadata, COMPOSE_VOLUME_LABEL) == Some(logical_name);
    if !identity_matches {
        return Err("volume identity changed after the scan; refresh before deleting".into());
    }

    let output = Command::new("docker")
        .args(["container", "ls", "--all", "--quiet", "--filter"])
        .arg(format!("volume={name}"))
        .output()
        .map_err(|error| format!("could not recheck Docker volume references: {error}"))?;
    if !output.status.success() {
        return Err(output_error(
            &output,
            "Docker volume reference check failed",
        ));
    }
    if !String::from_utf8_lossy(&output.stdout).trim().is_empty() {
        return Err("volume is now referenced by a container; refresh before deleting".into());
    }

    let output = Command::new("docker")
        .args(["volume", "rm", name])
        .output()
        .map_err(|error| format!("could not start Docker: {error}"))?;
    if output.status.success() {
        Ok("Docker volume deleted permanently".into())
    } else {
        Err(output_error(&output, "Docker volume deletion failed"))
    }
}

fn prune_build_cache() -> Result<String, String> {
    let output = Command::new("docker")
        .args(["builder", "prune", "--force"])
        .output()
        .map_err(|error| format!("could not start Docker: {error}"))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
    } else {
        Err(output_error(&output, "Docker prune failed"))
    }
}

fn output_error(output: &Output, fallback: &str) -> String {
    let error = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    if error.is_empty() {
        fallback.into()
    } else {
        error
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn metadata(name: &str, project: Option<&str>, logical_name: Option<&str>) -> Value {
        serde_json::json!({
            "Name": name,
            "Labels": {
                (COMPOSE_PROJECT_LABEL): project,
                (COMPOSE_VOLUME_LABEL): logical_name,
            }
        })
    }

    #[test]
    fn parses_docker_volume_usage_rows() {
        let parsed = parse_volume_usages("shop_db\t0\t8.2GB\nactive_cache\t2\t512MB\n").unwrap();
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].name, "shop_db");
        assert_eq!(parsed[0].links, 0);
        assert!(parsed[0].bytes > 8_000_000_000);
    }

    #[test]
    fn only_unused_labeled_compose_volumes_are_reviewable() {
        let unused = VolumeUsage {
            name: "shop_db".into(),
            links: 0,
            bytes: 100,
        };
        let active = VolumeUsage {
            name: "current_db".into(),
            links: 1,
            bytes: 200,
        };
        let unknown = VolumeUsage {
            name: "orphan".into(),
            links: 0,
            bytes: 300,
        };
        let candidates = build_volume_candidates(
            &[unused, active, unknown],
            &[
                metadata("shop_db", Some("shop"), Some("db")),
                metadata("current_db", Some("current"), Some("db")),
                metadata("orphan", None, None),
            ],
        );

        assert_eq!(candidates[0].risk, Risk::Review);
        assert_eq!(candidates[0].display_path(), "shop / db");
        assert!(!candidates[0].selected);
        assert_eq!(candidates[1].risk, Risk::Protected);
        assert_eq!(candidates[2].risk, Risk::Protected);
    }
}
