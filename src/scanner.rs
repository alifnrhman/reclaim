use crate::model::{Candidate, Fingerprint, Risk, ScanResult, ScanWarnings, Source};
use crate::rules;
use anyhow::{Context, Result, bail};
use std::collections::{HashSet, VecDeque};
use std::fs::{self, Metadata};
use std::path::{Path, PathBuf};

#[derive(Debug)]
struct Found {
    path: PathBuf,
    root: PathBuf,
    rule: rules::RuleMatch,
    protected_reason: Option<String>,
}

pub fn validate_root(path: &Path) -> Result<PathBuf> {
    let root = path
        .canonicalize()
        .with_context(|| format!("cannot access scan root {}", path.display()))?;
    if !root.is_dir() {
        bail!("scan root is not a directory: {}", root.display());
    }
    if root.parent().is_none() {
        bail!("refusing to scan a filesystem root: {}", root.display());
    }
    Ok(root)
}

pub fn scan_projects(paths: &[PathBuf]) -> Result<ScanResult> {
    let mut warnings = ScanWarnings::default();
    let mut roots = Vec::new();
    for path in paths {
        let root = validate_root(path)?;
        if !roots.contains(&root) {
            roots.push(root);
        }
    }

    let mut found = Vec::new();
    for root in &roots {
        discover(root, &mut found, &mut warnings);
    }

    let mut candidates: Vec<Candidate> = found
        .into_iter()
        .map(|item| {
            let (bytes, files, size_warnings) = tree_size(&item.path);
            let metadata = fs::symlink_metadata(&item.path).ok();
            let protected = item.protected_reason;
            let risk = if protected.is_some() {
                Risk::Protected
            } else {
                item.rule.risk
            };
            let reason = protected.unwrap_or(item.rule.reason);
            (
                Candidate {
                    id: 0,
                    path: Some(item.path),
                    scope_root: Some(item.root),
                    source: Source::Project,
                    docker_resource: None,
                    rule_id: item.rule.id,
                    category: item.rule.category,
                    bytes,
                    files,
                    risk,
                    reason,
                    restore: item.rule.restore,
                    fingerprint: metadata.as_ref().map(fingerprint),
                    selected: false,
                },
                size_warnings,
            )
        })
        .collect::<Vec<_>>()
        .into_iter()
        .map(|(candidate, local_warnings)| {
            warnings.0.extend(local_warnings);
            candidate
        })
        .collect();

    candidates.sort_by_key(|candidate| std::cmp::Reverse(candidate.bytes));
    for (id, candidate) in candidates.iter_mut().enumerate() {
        candidate.id = id;
    }
    Ok(ScanResult {
        candidates,
        warnings,
    })
}

fn discover(root: &Path, found: &mut Vec<Found>, warnings: &mut ScanWarnings) {
    let root_dev = fs::symlink_metadata(root).ok().and_then(|m| device_id(&m));
    let mut queue = VecDeque::from([root.to_path_buf()]);
    while let Some(dir) = queue.pop_front() {
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(error) => {
                warnings.0.push(format!("{}: {error}", dir.display()));
                continue;
            }
        };
        for entry in entries {
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => {
                    warnings.0.push(format!("{}: {error}", dir.display()));
                    continue;
                }
            };
            let path = entry.path();
            let name = entry.file_name();
            if rules::potentially_sensitive(&name.to_string_lossy()) {
                continue;
            }
            let metadata = match fs::symlink_metadata(&path) {
                Ok(metadata) => metadata,
                Err(error) => {
                    warnings.0.push(format!("{}: {error}", path.display()));
                    continue;
                }
            };
            let link_like = is_link_like(&metadata);
            let other_device = root_dev.is_some() && device_id(&metadata) != root_dev;

            if let Some(rule) = rules::classify(&path, root) {
                let protected_reason = if link_like {
                    Some(
                        "Symlinks and filesystem reparse points are never followed or deleted"
                            .into(),
                    )
                } else if other_device {
                    Some("Candidate crosses the scan root's filesystem boundary".into())
                } else {
                    None
                };
                found.push(Found {
                    path,
                    root: root.to_path_buf(),
                    rule,
                    protected_reason,
                });
                continue;
            }
            if metadata.is_dir() && !link_like && !other_device {
                queue.push_back(path);
            }
        }
    }
}

pub fn tree_size(path: &Path) -> (u64, u64, Vec<String>) {
    let mut bytes = 0_u64;
    let mut files = 0_u64;
    let mut warnings = Vec::new();
    let mut seen = HashSet::new();
    let mut queue = vec![path.to_path_buf()];
    while let Some(current) = queue.pop() {
        let metadata = match fs::symlink_metadata(&current) {
            Ok(metadata) => metadata,
            Err(error) => {
                warnings.push(format!("{}: {error}", current.display()));
                continue;
            }
        };
        if is_link_like(&metadata) {
            files += 1;
            bytes = bytes.saturating_add(allocated_size(&metadata));
            continue;
        }
        if metadata.is_dir() {
            match fs::read_dir(&current) {
                Ok(entries) => queue.extend(entries.filter_map(Result::ok).map(|e| e.path())),
                Err(error) => warnings.push(format!("{}: {error}", current.display())),
            }
        } else {
            if let Some(identity) = file_identity(&metadata)
                && !seen.insert(identity)
            {
                continue;
            }
            files += 1;
            bytes = bytes.saturating_add(allocated_size(&metadata));
        }
    }
    (bytes, files, warnings)
}

pub fn fingerprint(metadata: &Metadata) -> Fingerprint {
    Fingerprint {
        modified: metadata.modified().ok(),
        len: metadata.len(),
        file_id: file_identity(metadata),
    }
}

#[cfg(unix)]
pub fn allocated_size(metadata: &Metadata) -> u64 {
    use std::os::unix::fs::MetadataExt;
    metadata.blocks().saturating_mul(512)
}

#[cfg(not(unix))]
pub fn allocated_size(metadata: &Metadata) -> u64 {
    metadata.len()
}

#[cfg(unix)]
fn device_id(metadata: &Metadata) -> Option<u64> {
    use std::os::unix::fs::MetadataExt;
    Some(metadata.dev())
}

#[cfg(not(unix))]
fn device_id(_metadata: &Metadata) -> Option<u64> {
    None
}

#[cfg(unix)]
fn file_identity(metadata: &Metadata) -> Option<(u64, u64)> {
    use std::os::unix::fs::MetadataExt;
    Some((metadata.dev(), metadata.ino()))
}

#[cfg(not(unix))]
fn file_identity(_metadata: &Metadata) -> Option<(u64, u64)> {
    None
}

#[cfg(windows)]
pub fn is_link_like(metadata: &Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    metadata.file_type().is_symlink()
        || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
pub fn is_link_like(metadata: &Metadata) -> bool {
    metadata.file_type().is_symlink()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct TestDir(PathBuf);

    impl TestDir {
        fn new(label: &str) -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "reclaim-test-{label}-{}-{nonce}",
                std::process::id()
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn node_modules_needs_a_lockfile_to_be_safe() {
        let temp = TestDir::new("node");
        fs::write(temp.0.join("package.json"), "{}").unwrap();
        fs::create_dir(temp.0.join("node_modules")).unwrap();
        fs::write(temp.0.join("node_modules/package.bin"), vec![0; 4096]).unwrap();

        let scan = scan_projects(std::slice::from_ref(&temp.0)).unwrap();
        assert_eq!(scan.candidates.len(), 1);
        assert_eq!(scan.candidates[0].risk, Risk::Review);

        fs::write(temp.0.join("bun.lock"), "").unwrap();
        let scan = scan_projects(std::slice::from_ref(&temp.0)).unwrap();
        assert_eq!(scan.candidates[0].risk, Risk::Safe);
        assert!(!scan.candidates[0].selected);
    }

    #[test]
    fn generated_and_generic_outputs_get_different_risks() {
        let temp = TestDir::new("rules");
        fs::write(temp.0.join("Cargo.toml"), "[package]").unwrap();
        fs::create_dir(temp.0.join("target")).unwrap();
        fs::create_dir(temp.0.join("dist")).unwrap();

        let scan = scan_projects(std::slice::from_ref(&temp.0)).unwrap();
        assert_eq!(scan.candidates.len(), 2);
        assert!(
            scan.candidates
                .iter()
                .any(|c| c.rule_id == "cargo-target" && c.risk == Risk::Safe)
        );
        assert!(
            scan.candidates
                .iter()
                .any(|c| c.rule_id == "generic-output" && c.risk == Risk::Review)
        );
    }

    #[test]
    fn recognizes_locked_node_outputs_without_trusting_every_dist() {
        let temp = TestDir::new("node-outputs");
        fs::write(
            temp.0.join("package.json"),
            r#"{"scripts":{"build":"vite build"}}"#,
        )
        .unwrap();
        fs::write(temp.0.join("bun.lock"), "").unwrap();
        for directory in ["dist", "coverage", ".nyc_output"] {
            fs::create_dir(temp.0.join(directory)).unwrap();
        }
        fs::write(temp.0.join(".eslintcache"), "generated").unwrap();

        let scan = scan_projects(std::slice::from_ref(&temp.0)).unwrap();
        assert_eq!(scan.candidates.len(), 4);
        assert!(scan.candidates.iter().all(|c| c.risk == Risk::Safe));
        assert!(scan.candidates.iter().any(|c| c.rule_id == "node-dist"));

        fs::remove_file(temp.0.join("bun.lock")).unwrap();
        let scan = scan_projects(std::slice::from_ref(&temp.0)).unwrap();
        assert!(
            scan.candidates
                .iter()
                .any(|c| c.rule_id == "generic-output" && c.risk == Risk::Review)
        );
        assert!(
            scan.candidates
                .iter()
                .filter(|c| c.rule_id == "node-tool-cache")
                .all(|c| c.risk == Risk::Review)
        );
    }

    #[test]
    fn recognizes_nuxt_output_as_one_safe_candidate() {
        let temp = TestDir::new("nuxt-output");
        fs::write(temp.0.join("nuxt.config.ts"), "export default {};").unwrap();
        fs::create_dir(temp.0.join(".output")).unwrap();
        fs::create_dir_all(temp.0.join(".output/server/node_modules")).unwrap();

        let scan = scan_projects(std::slice::from_ref(&temp.0)).unwrap();
        assert_eq!(scan.candidates.len(), 1);
        assert_eq!(scan.candidates[0].rule_id, "nuxt-output");
        assert_eq!(scan.candidates[0].risk, Risk::Safe);
    }

    #[cfg(unix)]
    #[test]
    fn matching_symlink_is_protected_and_never_followed() {
        use std::os::unix::fs::symlink;
        let temp = TestDir::new("symlink");
        let outside = TestDir::new("outside");
        fs::write(temp.0.join("package.json"), "{}").unwrap();
        fs::write(temp.0.join("bun.lock"), "").unwrap();
        symlink(&outside.0, temp.0.join("node_modules")).unwrap();

        let scan = scan_projects(std::slice::from_ref(&temp.0)).unwrap();
        assert_eq!(scan.candidates.len(), 1);
        assert_eq!(scan.candidates[0].risk, Risk::Protected);
        assert_eq!(scan.candidates[0].bytes, 0);
    }

    #[cfg(unix)]
    #[test]
    fn hardlinks_are_counted_once_inside_a_candidate() {
        let temp = TestDir::new("hardlinks");
        let cache = temp.0.join(".cache");
        fs::create_dir(&cache).unwrap();
        fs::write(cache.join("one"), vec![1; 4096]).unwrap();
        fs::hard_link(cache.join("one"), cache.join("two")).unwrap();
        let (size, files, warnings) = tree_size(&cache);
        assert!(warnings.is_empty());
        assert_eq!(files, 1);
        assert!(size >= 4096);
    }
}
