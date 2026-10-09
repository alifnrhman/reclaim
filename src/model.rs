use std::path::PathBuf;
use std::time::SystemTime;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Risk {
    Safe,
    Review,
    Protected,
}

impl Risk {
    pub fn label(self) -> &'static str {
        match self {
            Self::Safe => "SAFE",
            Self::Review => "REVIEW",
            Self::Protected => "PROTECTED",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    Project,
    Global,
    Docker,
}

impl Source {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Project => "project",
            Self::Global => "global",
            Self::Docker => "docker",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Fingerprint {
    pub modified: Option<SystemTime>,
    pub len: u64,
    pub file_id: Option<(u64, u64)>,
}

#[derive(Debug, Clone)]
pub struct Candidate {
    pub id: usize,
    pub path: Option<PathBuf>,
    pub scope_root: Option<PathBuf>,
    pub source: Source,
    pub rule_id: &'static str,
    pub category: &'static str,
    pub bytes: u64,
    pub files: u64,
    pub risk: Risk,
    pub reason: String,
    pub restore: String,
    pub fingerprint: Option<Fingerprint>,
    pub selected: bool,
}

impl Candidate {
    pub fn display_path(&self) -> String {
        self.path
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| format!("Docker {}", self.category))
    }
}

#[derive(Debug, Default, Clone)]
pub struct ScanWarnings(pub Vec<String>);

#[derive(Debug)]
pub struct ScanResult {
    pub candidates: Vec<Candidate>,
    pub warnings: ScanWarnings,
}
