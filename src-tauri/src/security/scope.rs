//! Enumerate the exact local source scope and retain fingerprints for evidence checks.
use super::Result;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
#[cfg(test)]
use std::collections::BTreeSet;
use std::path::{Component, Path};

#[derive(Clone, Serialize, Deserialize)]
pub struct File {
    pub path: String,
    pub sha256: String,
}
pub fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
pub fn safe_relative(path: &str) -> bool {
    !path.is_empty()
        && !path.contains(['\n', '\r', '\0', '\\'])
        && !Path::new(path).is_absolute()
        && Path::new(path)
            .components()
            .all(|c| matches!(c, Component::Normal(_)))
}
pub fn target(root: &str, mode: &str, selected: &str) -> Result<String> {
    if !["repository", "path", "working-tree"].contains(&mode)
        || (mode == "path" && !safe_relative(selected))
        || (mode != "path" && !selected.is_empty())
    {
        return Err("security_invalid:scope".into());
    }
    let root = Path::new(root)
        .canonicalize()
        .map_err(|_| "security_missing_root")?;
    if !root.is_dir() {
        return Err("security_missing_root".into());
    }
    if mode == "path" {
        let chosen = root
            .join(selected)
            .canonicalize()
            .map_err(|_| "security_source_missing")?;
        if !chosen.starts_with(&root) {
            return Err("security_outside_scope".into());
        }
    }
    Ok(root.to_string_lossy().into_owned())
}
#[cfg(test)]
pub fn source(root: &str, path: &str) -> Result<String> {
    if !safe_relative(path) {
        return Err("security_invalid:path".into());
    }
    let root = Path::new(root)
        .canonicalize()
        .map_err(|_| "security_missing_root")?;
    let full = root.join(path);
    let metadata = std::fs::symlink_metadata(&full).map_err(|_| "security_source_missing")?;
    if !metadata.is_file()
        || metadata.len() > 2 * 1024 * 1024
        || !full.canonicalize().is_ok_and(|p| p.starts_with(&root))
    {
        return Err("security_source_unavailable".into());
    }
    let bytes = std::fs::read(full).map_err(|_| "security_source_missing")?;
    if bytes.contains(&0) {
        return Err("security_binary".into());
    }
    String::from_utf8(bytes).map_err(|_| "security_binary".into())
}
#[cfg(test)]
fn git(root: &str, args: &[&str]) -> Result<Vec<u8>> {
    let out = crate::host::command("git")
        .args([
            "--no-optional-locks",
            "--literal-pathspecs",
            "-c",
            "core.fsmonitor=false",
            "-C",
        ])
        .arg(root)
        .args(args)
        .output()
        .map_err(|_| "security_git_failed")?;
    if !out.status.success() {
        return Err("security_git_failed".into());
    }
    if out.stdout.len() > 8 * 1024 * 1024 {
        return Err("security_scope_too_large".into());
    }
    Ok(out.stdout)
}
#[cfg(test)]
fn walk(
    root: &Path,
    dir: &Path,
    names: &mut BTreeSet<String>,
    excluded: &mut Vec<String>,
) -> Result<()> {
    for entry in std::fs::read_dir(dir).map_err(|_| "security_source_unavailable")? {
        let entry = entry.map_err(|_| "security_source_unavailable")?;
        let full = entry.path();
        let name = full
            .strip_prefix(root)
            .map_err(|_| "security_invalid:path")?
            .to_string_lossy()
            .replace('\\', "/");
        let kind = entry
            .file_type()
            .map_err(|_| "security_source_unavailable")?;
        if kind.is_dir()
            && [
                ".git",
                ".hg",
                ".svn",
                "node_modules",
                ".venv",
                "venv",
                "target",
                "dist",
                "build",
            ]
            .contains(&entry.file_name().to_string_lossy().as_ref())
        {
            excluded.push(format!("{name}/"));
            continue;
        }
        if names.len() + excluded.len() >= 20000 {
            return Err("security_scope_too_large".into());
        }
        if kind.is_dir() {
            walk(root, &full, names, excluded)?;
        } else {
            names.insert(name);
        }
    }
    Ok(())
}
#[cfg(test)]
pub fn collect(root: &str, mode: &str, selected: &str) -> Result<(String, Vec<File>, Vec<String>)> {
    if !["repository", "path", "working-tree"].contains(&mode)
        || (mode == "path" && !safe_relative(selected))
        || (mode != "path" && !selected.is_empty())
    {
        return Err("security_invalid:scope".into());
    }
    let root = Path::new(root)
        .canonicalize()
        .map_err(|_| "security_missing_root")?
        .to_string_lossy()
        .into_owned();
    let in_git =
        git(&root, &["rev-parse", "--is-inside-work-tree"]).is_ok_and(|v| v.starts_with(b"true"));
    let mut excluded = vec![];
    let names: BTreeSet<String> = if in_git {
        let bytes = if mode == "working-tree" {
            let mut bytes = git(
                &root,
                &[
                    "diff",
                    "--no-ext-diff",
                    "--no-textconv",
                    "--relative",
                    "--name-only",
                    "-z",
                    "--",
                ],
            )?;
            bytes.extend(git(
                &root,
                &[
                    "diff",
                    "--cached",
                    "--no-ext-diff",
                    "--no-textconv",
                    "--relative",
                    "--name-only",
                    "-z",
                    "--",
                ],
            )?);
            bytes.extend(git(
                &root,
                &["ls-files", "--others", "--exclude-standard", "-z"],
            )?);
            bytes
        } else if mode == "path" {
            git(
                &root,
                &[
                    "ls-files",
                    "--cached",
                    "--others",
                    "--exclude-standard",
                    "-z",
                    "--",
                    selected,
                ],
            )?
        } else {
            git(
                &root,
                &[
                    "ls-files",
                    "--cached",
                    "--others",
                    "--exclude-standard",
                    "-z",
                ],
            )?
        };
        bytes
            .split(|b| *b == 0)
            .filter(|p| !p.is_empty())
            .map(|p| {
                String::from_utf8(p.to_vec()).map_err(|_| "security_invalid_filename".to_string())
            })
            .collect::<Result<_>>()?
    } else {
        if mode == "working-tree" {
            return Err("security_git_required".into());
        }
        let mut names = BTreeSet::new();
        let start = if mode == "path" {
            Path::new(&root).join(selected)
        } else {
            Path::new(&root).to_path_buf()
        };
        if !start.canonicalize().is_ok_and(|p| p.starts_with(&root)) {
            return Err("security_invalid:path".into());
        }
        let meta = std::fs::symlink_metadata(&start).map_err(|_| "security_source_missing")?;
        if meta.is_dir() {
            walk(Path::new(&root), &start, &mut names, &mut excluded)?;
        } else if meta.is_file() {
            names.insert(selected.into());
        } else {
            excluded.push(selected.into());
        }
        names
    };
    if names.len() > 20000 {
        return Err("security_scope_too_large".into());
    }
    let mut files = vec![];
    if mode == "path" {
        excluded.retain(|p| {
            p.trim_end_matches('/') == selected || p.starts_with(&format!("{selected}/"))
        });
    }
    for path in names {
        if mode == "path" && path != selected && !path.starts_with(&format!("{selected}/")) {
            continue;
        }
        if !safe_relative(&path) {
            excluded.push(path);
            continue;
        }
        match source(&root, &path) {
            Ok(text) => files.push(File {
                path,
                sha256: digest(text.as_bytes()),
            }),
            Err(_) => excluded.push(path),
        }
    }
    Ok((root, files, excluded))
}
