use crate::model::Risk;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct RuleMatch {
    pub id: &'static str,
    pub category: &'static str,
    pub risk: Risk,
    pub reason: String,
    pub restore: String,
}

fn has_any(dir: &Path, names: &[&str]) -> bool {
    names.iter().any(|name| dir.join(name).is_file())
}

fn ancestors_to<'a>(path: &'a Path, root: &'a Path) -> impl Iterator<Item = PathBuf> + 'a {
    path.parent()
        .into_iter()
        .flat_map(move |p| p.ancestors().take_while(move |a| a.starts_with(root)))
        .map(Path::to_path_buf)
}

fn project_dir(path: &Path, root: &Path, markers: &[&str]) -> Option<PathBuf> {
    ancestors_to(path, root).find(|dir| has_any(dir, markers))
}

fn package_manager(dir: &Path) -> (&'static str, bool) {
    if dir.join("bun.lock").is_file() || dir.join("bun.lockb").is_file() {
        ("bun install", true)
    } else if dir.join("pnpm-lock.yaml").is_file() {
        ("pnpm install --frozen-lockfile", true)
    } else if dir.join("yarn.lock").is_file() {
        ("yarn install --immutable", true)
    } else if dir.join("package-lock.json").is_file() || dir.join("npm-shrinkwrap.json").is_file() {
        ("npm ci", true)
    } else {
        (
            "install dependencies with the project's package manager",
            false,
        )
    }
}

pub fn classify(path: &Path, root: &Path) -> Option<RuleMatch> {
    let name = path.file_name()?.to_str()?;

    if name == "node_modules" {
        let dir = project_dir(path, root, &["package.json"])?;
        let (restore, locked) = package_manager(&dir);
        return Some(RuleMatch {
            id: "node_modules",
            category: "dependencies",
            risk: if locked { Risk::Safe } else { Risk::Review },
            reason: if locked {
                "Node dependencies are reproducible from the manifest and lockfile".into()
            } else {
                "package.json exists, but no recognized lockfile was found".into()
            },
            restore: restore.into(),
        });
    }

    if name == "target" {
        if project_dir(path, root, &["Cargo.toml"]).is_some() {
            return Some(RuleMatch {
                id: "cargo-target",
                category: "build",
                risk: Risk::Safe,
                reason: "Rust build output can be rebuilt from Cargo.toml".into(),
                restore: "cargo build".into(),
            });
        }
        if project_dir(path, root, &["pom.xml"]).is_some() {
            return Some(RuleMatch {
                id: "maven-target",
                category: "build",
                risk: Risk::Safe,
                reason: "Maven build output can be rebuilt from pom.xml".into(),
                restore: "mvn package".into(),
            });
        }
    }

    if matches!(
        name,
        "__pycache__" | ".pytest_cache" | ".mypy_cache" | ".ruff_cache"
    ) {
        return Some(RuleMatch {
            id: "python-cache",
            category: "cache",
            risk: Risk::Safe,
            reason: "Generated Python interpreter or tool cache".into(),
            restore: "It is recreated automatically when the tool runs".into(),
        });
    }

    if matches!(name, ".venv" | "venv") {
        let markers = [
            "uv.lock",
            "poetry.lock",
            "Pipfile.lock",
            "requirements.txt",
            "pyproject.toml",
        ];
        let locked = project_dir(path, root, &markers).is_some();
        return Some(RuleMatch {
            id: "python-venv",
            category: "dependencies",
            risk: if locked { Risk::Safe } else { Risk::Review },
            reason: if locked {
                "Python environment has a recognized dependency manifest".into()
            } else {
                "No recognized Python dependency manifest was found".into()
            },
            restore: "recreate the environment, then install project dependencies".into(),
        });
    }

    if name == ".next"
        && project_dir(
            path,
            root,
            &[
                "next.config.js",
                "next.config.mjs",
                "next.config.ts",
                "package.json",
            ],
        )
        .is_some()
    {
        return generated(
            "next-cache",
            "framework cache",
            "Next.js output is generated",
            "run the project's Next.js build or dev command",
        );
    }
    if name == ".nuxt"
        && project_dir(
            path,
            root,
            &["nuxt.config.js", "nuxt.config.ts", "package.json"],
        )
        .is_some()
    {
        return generated(
            "nuxt-cache",
            "framework cache",
            "Nuxt output is generated",
            "run the project's Nuxt build or dev command",
        );
    }
    if name == ".turbo" && project_dir(path, root, &["turbo.json", "package.json"]).is_some() {
        return generated(
            "turbo-cache",
            "framework cache",
            "Turborepo cache is generated",
            "rerun the relevant turbo task",
        );
    }
    if name == ".vite" && project_dir(path, root, &["package.json"]).is_some() {
        return generated(
            "vite-cache",
            "framework cache",
            "Vite dependency cache is generated",
            "rerun the Vite dev server or build",
        );
    }

    if matches!(name, "build" | ".gradle")
        && project_dir(
            path,
            root,
            &[
                "build.gradle",
                "build.gradle.kts",
                "settings.gradle",
                "settings.gradle.kts",
            ],
        )
        .is_some()
    {
        return generated(
            "gradle-build",
            "build",
            "Gradle output/cache is generated",
            "./gradlew build",
        );
    }

    if matches!(name, "bin" | "obj") && project_dir(path, root, &["global.json"]).is_some()
        || matches!(name, "bin" | "obj")
            && ancestors_to(path, root).any(|d| {
                std::fs::read_dir(d)
                    .ok()
                    .into_iter()
                    .flatten()
                    .flatten()
                    .any(|e| {
                        matches!(
                            e.path().extension().and_then(|s| s.to_str()),
                            Some("csproj" | "fsproj" | "vbproj")
                        )
                    })
            })
    {
        return generated(
            "dotnet-build",
            "build",
            ".NET bin/obj output is generated",
            "dotnet build",
        );
    }

    if matches!(name, "dist" | "build" | "coverage" | ".cache") {
        return Some(RuleMatch {
            id: "generic-output",
            category: "generated?",
            risk: Risk::Review,
            reason: "Common generated-output name, but it may contain a deliverable or user data"
                .into(),
            restore: "verify the owning tool before deleting".into(),
        });
    }
    None
}

fn generated(
    id: &'static str,
    category: &'static str,
    reason: &'static str,
    restore: &'static str,
) -> Option<RuleMatch> {
    Some(RuleMatch {
        id,
        category,
        risk: Risk::Safe,
        reason: reason.into(),
        restore: restore.into(),
    })
}

pub fn potentially_sensitive(name: &str) -> bool {
    matches!(name, ".git" | ".hg" | ".svn" | ".ssh" | ".gnupg")
}
