use crate::docker;
use crate::model::{Candidate, Risk, Source};
use crate::rules;
use crate::scanner::{fingerprint, is_link_like, tree_size};
use std::fs;
use std::path::Path;

#[derive(Debug)]
pub struct DeleteReport {
    pub id: usize,
    pub display: String,
    pub estimated_bytes: u64,
    pub result: Result<String, String>,
}

pub fn delete_selected(candidates: &[Candidate]) -> Vec<DeleteReport> {
    delete_selected_with_progress(candidates, |_, _, _| {})
}

pub fn delete_selected_with_progress(
    candidates: &[Candidate],
    mut on_progress: impl FnMut(usize, usize, &Candidate),
) -> Vec<DeleteReport> {
    let selected: Vec<_> = candidates
        .iter()
        .filter(|candidate| candidate.selected && candidate.risk != Risk::Protected)
        .collect();
    let total = selected.len();
    selected
        .into_iter()
        .enumerate()
        .map(|(index, candidate)| {
            on_progress(index + 1, total, candidate);
            delete_one(candidate)
        })
        .collect()
}

fn delete_one(candidate: &Candidate) -> DeleteReport {
    let result = match candidate.source {
        Source::Docker => docker::delete(candidate),
        Source::Project | Source::Global => validate_and_delete(candidate),
    };
    DeleteReport {
        id: candidate.id,
        display: candidate.display_path(),
        estimated_bytes: candidate.bytes,
        result,
    }
}

fn validate_and_delete(candidate: &Candidate) -> Result<String, String> {
    let path = candidate
        .path
        .as_deref()
        .ok_or("candidate has no filesystem path")?;
    let scope = candidate
        .scope_root
        .as_deref()
        .ok_or("candidate has no approved scope")?;
    let metadata =
        fs::symlink_metadata(path).map_err(|error| format!("candidate disappeared: {error}"))?;
    if is_link_like(&metadata) {
        return Err("candidate became a symlink or reparse point".into());
    }
    let canonical = path
        .canonicalize()
        .map_err(|error| format!("cannot resolve candidate: {error}"))?;
    let canonical_scope = scope
        .canonicalize()
        .map_err(|error| format!("cannot resolve approved scope: {error}"))?;
    let in_scope = match candidate.source {
        Source::Project => canonical.starts_with(&canonical_scope) && canonical != canonical_scope,
        Source::Global => canonical == canonical_scope,
        Source::Docker => false,
    };
    if !in_scope {
        return Err("candidate is outside its approved scope".into());
    }

    if let Some(old) = &candidate.fingerprint {
        let current = fingerprint(&metadata);
        if current.modified != old.modified
            || current.len != old.len
            || current.file_id != old.file_id
        {
            return Err("candidate changed after the scan; refresh before deleting".into());
        }
    }
    let (current_bytes, current_files, warnings) = tree_size(path);
    if let Some(warning) = warnings.first() {
        return Err(format!(
            "candidate could not be fully revalidated; refresh before deleting: {warning}"
        ));
    }
    if current_bytes != candidate.bytes || current_files != candidate.files {
        return Err("candidate contents changed after the scan; refresh before deleting".into());
    }
    if candidate.source == Source::Project {
        let rule = rules::classify(&canonical, &canonical_scope)
            .ok_or("candidate no longer matches a built-in rule")?;
        if rule.id != candidate.rule_id {
            return Err("candidate classification changed after the scan".into());
        }
    }
    safe_remove(path).map_err(|error| format!("delete failed: {error}"))?;
    Ok("deleted permanently".into())
}

fn safe_remove(path: &Path) -> std::io::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if is_link_like(&metadata) {
        return Err(std::io::Error::other("refusing to delete a link-like path"));
    }
    if !metadata.is_dir() {
        return fs::remove_file(path);
    }
    // On the supported Unix and Windows targets, the standard library uses
    // descriptor/handle-relative traversal here (openat/unlinkat on Unix and
    // handle-based APIs on Windows), protecting each child operation against
    // symlink TOCTOU replacement.
    fs::remove_dir_all(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scanner;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static NEXT_TEST_DIR: AtomicU64 = AtomicU64::new(0);

    struct TestDir(PathBuf);

    impl TestDir {
        fn new() -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let sequence = NEXT_TEST_DIR.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "reclaim-delete-test-{}-{nonce}-{sequence}",
                std::process::id(),
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn deletes_a_revalidated_safe_candidate() {
        let temp = TestDir::new();
        fs::write(temp.0.join("Cargo.toml"), "[package]").unwrap();
        fs::create_dir(temp.0.join("target")).unwrap();
        fs::write(temp.0.join("target/artifact"), "generated").unwrap();
        let mut scan = scanner::scan_projects(std::slice::from_ref(&temp.0)).unwrap();
        scan.candidates[0].selected = true;
        let path = scan.candidates[0].path.clone().unwrap();

        let reports = delete_selected(&scan.candidates);
        assert!(reports[0].result.is_ok());
        assert!(!path.exists());
        assert!(temp.0.join("Cargo.toml").exists());
    }

    #[test]
    fn refuses_a_candidate_that_changed_after_scan() {
        let temp = TestDir::new();
        fs::write(temp.0.join("Cargo.toml"), "[package]").unwrap();
        fs::create_dir(temp.0.join("target")).unwrap();
        let mut scan = scanner::scan_projects(std::slice::from_ref(&temp.0)).unwrap();
        scan.candidates[0].selected = true;
        fs::write(temp.0.join("target/new-artifact"), "changed").unwrap();

        let reports = delete_selected(&scan.candidates);
        assert!(reports[0].result.is_err());
        assert!(temp.0.join("target").exists());
    }

    #[cfg(unix)]
    #[test]
    fn deleting_a_tree_never_follows_an_inner_symlink() {
        use std::os::unix::fs::symlink;

        let temp = TestDir::new();
        let outside = TestDir::new();
        fs::write(outside.0.join("keep"), "important").unwrap();
        let target = temp.0.join("target");
        fs::create_dir(&target).unwrap();
        symlink(&outside.0, target.join("outside-link")).unwrap();

        safe_remove(&target).unwrap();

        assert!(!target.exists());
        assert_eq!(
            fs::read_to_string(outside.0.join("keep")).unwrap(),
            "important"
        );
    }
}
