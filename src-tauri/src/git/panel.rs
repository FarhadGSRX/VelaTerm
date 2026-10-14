//! Path-safe reads for the Git panel and its staged/worktree comparisons.

use super::*;
use std::path::Path;

const DIFF_LIMIT: usize = 2 * 1024 * 1024;

pub(super) fn read_git(cwd: &str, args: &[&str]) -> Result<Vec<u8>, String> {
    let out = crate::host::command("git").arg("-C").arg(cwd).args(args)
        .env("LC_ALL", "C").output().map_err(|e| format!("Failed to run Git: {e}"))?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    Ok(out.stdout)
}

/// A symlink's Git content is its target text, not the contents of the linked file.
pub(super) fn read_worktree(path: &Path) -> Result<Option<Vec<u8>>, String> {
    let meta = match std::fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("Cannot inspect file: {e}")),
    };
    if meta.file_type().is_symlink() {
        return std::fs::read_link(path).map(|p| Some(p.to_string_lossy().as_bytes().to_vec()))
            .map_err(|e| format!("Cannot read symbolic link: {e}"));
    }
    if !meta.is_file() { return Err("Select a file to view its changes; this entry is a directory or submodule".into()); }
    let mut bytes = Vec::new();
    std::fs::File::open(path).and_then(|f| f.take((DIFF_LIMIT + 1) as u64).read_to_end(&mut bytes))
        .map_err(|e| format!("Cannot read file: {e}"))?;
    Ok(Some(bytes))
}

/// Count complete text lines without loading an entire untracked file into memory.
pub(super) fn worktree_stats(path: &Path) -> Result<(u32, bool), String> {
    let meta = match std::fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok((0, false)),
        Err(e) => return Err(format!("Cannot inspect file: {e}")),
    };
    if meta.file_type().is_symlink() {
        let bytes = read_worktree(path)?.unwrap_or_default();
        return Ok((line_count(&bytes), false));
    }
    if !meta.is_file() { return Ok((0, false)); }
    let mut file = std::fs::File::open(path).map_err(|e| format!("Cannot read file: {e}"))?;
    let mut buffer = [0u8; 65536];
    let mut count = 0u32;
    let mut last = None;
    loop {
        let n = file.read(&mut buffer).map_err(|e| format!("Cannot read file: {e}"))?;
        if n == 0 { break; }
        if buffer[..n].contains(&0) { return Ok((0, true)); }
        count = count.saturating_add(buffer[..n].iter().filter(|b| **b == b'\n').count() as u32);
        last = Some(buffer[n - 1]);
    }
    Ok((count.saturating_add(u32::from(last.is_some() && last != Some(b'\n'))), false))
}

pub(super) fn line_count(bytes: &[u8]) -> u32 {
    bytes.iter().filter(|b| **b == b'\n').count() as u32
        + u32::from(!bytes.is_empty() && bytes.last() != Some(&b'\n'))
}

pub(super) fn expand_renames(cwd: &str, paths: &[String]) -> Result<Vec<String>, String> {
    let mut expanded = paths.to_vec();
    for entry in path_status(cwd, true)? {
        if entry.xy.starts_with('R') && paths.contains(&entry.path) {
            if let Some(old) = entry.original_path { expanded.push(old); }
        }
    }
    expanded.sort();
    expanded.dedup();
    Ok(expanded)
}

/// Resolve exact tree/index entries before reading blobs. Missing files and read errors differ.
pub(super) fn blob(cwd: &str, revision: Option<&str>, path: &str) -> Result<Option<Vec<u8>>, String> {
    let out = match revision {
        Some(rev) => read_git(cwd, &["--literal-pathspecs", "ls-tree", "-z", "--full-tree", rev, "--", path])?,
        None => read_git(cwd, &["--literal-pathspecs", "ls-files", "--stage", "-z", "--", path])?,
    };
    for record in out.split(|b| *b == 0) {
        let Some(tab) = record.iter().position(|b| *b == b'\t') else { continue; };
        if &record[tab + 1..] != path.as_bytes() { continue; }
        let header = String::from_utf8_lossy(&record[..tab]);
        let fields: Vec<_> = header.split_whitespace().collect();
        if fields.len() != 3 { return Err("Invalid Git file entry".into()); }
        if revision.is_none() && fields[2] != "0" { return Err("Resolve this file's merge conflict before viewing the index".into()); }
        let oid = if revision.is_some() { fields[2] } else { fields[1] };
        if fields[0] == "160000" { return Ok(Some(format!("Subproject commit {oid}\n").into_bytes())); }
        let size = read_git(cwd, &["cat-file", "-s", oid])?;
        if String::from_utf8_lossy(&size).trim().parse::<usize>().unwrap_or(0) > DIFF_LIMIT {
            return Ok(Some(vec![b' '; DIFF_LIMIT + 1]));
        }
        return read_git(cwd, &["cat-file", "blob", oid]).map(Some);
    }
    Ok(None)
}

pub(super) fn make_diff(path: &str, original: Option<Vec<u8>>, modified: Option<Vec<u8>>) -> FileDiff {
    let original = original.unwrap_or_default();
    let modified = modified.unwrap_or_default();
    let binary = original.contains(&0) || modified.contains(&0);
    let too_large = original.len() > DIFF_LIMIT || modified.len() > DIFF_LIMIT;
    FileDiff {
        path: path.to_string(), binary, too_large,
        original: if binary || too_large { String::new() } else { String::from_utf8_lossy(&original).into_owned() },
        modified: if binary || too_large { String::new() } else { String::from_utf8_lossy(&modified).into_owned() },
    }
}

pub fn file_diff_at(cwd: &str, path: &str, side: &str, work_path: &Path) -> Result<FileDiff, String> {
    if !matches!(side, "all" | "staged" | "unstaged") { return Err("Invalid Git comparison".into()); }
    if cwd.trim().is_empty() || path.trim().is_empty() { return Err("Missing working directory or path".into()); }
    // Preserve the existing read-only viewer outside repositories; the caller gates the same path.
    if side == "all" && run_git(cwd, &["rev-parse", "--is-inside-work-tree"]).as_deref() != Some("true") {
        return Ok(make_diff(path, None, read_worktree(work_path)?));
    }
    require_repo(cwd)?;
    check_rel_path(path)?;
    let root = repo_top(cwd);
    let entry = path_status(&root, true)?.into_iter().find(|e| e.path == path);
    let old = entry.as_ref().and_then(|e| e.original_path.as_deref()).unwrap_or(path);
    let original = if side == "unstaged" {
        blob(&root, None, path)?
    } else if has_head(&root) {
        blob(&root, Some("HEAD"), old)?
    } else { None };
    let modified = if side == "staged" { blob(&root, None, path)? } else { read_worktree(work_path)? };
    Ok(make_diff(path, original, modified))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    struct Repo(PathBuf);
    impl Repo {
        fn new() -> Self {
            let repo = Self(std::env::temp_dir().join(format!("vlx-git-view-{}", Uuid::new_v4())));
            std::fs::create_dir_all(&repo.0).unwrap();
            repo.git(&["init", "-q", "-b", "main"]);
            repo.git(&["config", "user.name", "Git viewer test"]);
            repo.git(&["config", "user.email", "viewer@example.invalid"]);
            repo.git(&["config", "commit.gpgsign", "false"]);
            repo
        }
        fn cwd(&self) -> &str { self.0.to_str().unwrap() }
        fn git(&self, args: &[&str]) -> String { run_git_checked(self.cwd(), args).unwrap() }
        fn write(&self, name: &str, contents: &str) {
            let path = self.0.join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, contents).unwrap();
        }
        fn commit(&self) -> String {
            self.git(&["add", "-A"]);
            self.git(&["commit", "-qm", "fixture"]);
            self.git(&["rev-parse", "HEAD"])
        }
        fn diff(&self, path: &str, side: &str) -> FileDiff {
            file_diff_at(self.cwd(), path, side, &self.0.join(path)).unwrap()
        }
    }
    impl Drop for Repo { fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); } }

    #[test]
    fn staged_and_unstaged_views_show_their_own_contents() {
        let repo = Repo::new();
        let name = "文档/自己的修改.md";
        repo.write(name, "first\nsecond\n"); repo.commit();
        repo.write(name, "staged\nsecond\n"); repo.git(&["add", "--", name]);
        repo.write(name, "staged\nunstaged\n");
        let staged = repo.diff(name, "staged");
        assert_eq!(staged.original, "first\nsecond\n");
        assert_eq!(staged.modified, "staged\nsecond\n");
        let unstaged = repo.diff(name, "unstaged");
        assert_eq!(unstaged.original, staged.modified);
        assert_eq!(unstaged.modified, "staged\nunstaged\n");
        let files = changed_files(repo.cwd()).unwrap();
        assert_eq!((files[0].staged_additions, files[0].unstaged_additions), (1, 1));
    }

    #[test]
    fn unborn_and_untracked_files_have_real_paths_and_counts() {
        let repo = Repo::new();
        repo.write("新目录/甲.txt", "one\ntwo");
        repo.write("新目录/乙.txt", "three\n");
        std::fs::write(repo.0.join("image.bin"), b"a\0b").unwrap();
        assert_eq!(status(repo.cwd()).branch.as_deref(), Some("main"));
        let files = changed_files(repo.cwd()).unwrap();
        assert_eq!(files.len(), 3);
        assert_eq!(files.iter().find(|f| f.path.ends_with("甲.txt")).unwrap().unstaged_additions, 2);
        assert!(files.iter().find(|f| f.path == "image.bin").unwrap().unstaged_binary);
        repo.git(&["add", "--", "新目录/甲.txt"]);
        assert_eq!(repo.diff("新目录/甲.txt", "staged").modified, "one\ntwo");
        let files = changed_files(repo.cwd()).unwrap();
        assert_eq!(files.iter().find(|f| f.path.ends_with("甲.txt")).unwrap().staged_additions, 2);
        std::fs::remove_file(repo.0.join("新目录/甲.txt")).unwrap();
        assert_eq!(changed_files(repo.cwd()).unwrap().len(), 3);
    }

    #[test]
    fn rename_uses_the_old_path_for_its_original_and_unstages_both_paths() {
        let repo = Repo::new();
        repo.write("旧文件.txt", "unchanged\n"); repo.commit();
        repo.git(&["mv", "旧文件.txt", "新文件.txt"]);
        let files = changed_files(repo.cwd()).unwrap();
        assert_eq!(files[0].original_path.as_deref(), Some("旧文件.txt"));
        let diff = repo.diff("新文件.txt", "staged");
        assert_eq!(diff.original, diff.modified);
        assert_eq!(diff.original, "unchanged\n");
        unstage(repo.cwd(), &["新文件.txt".into()]).unwrap();
        assert!(repo.git(&["diff", "--cached", "--name-only"]).is_empty());
    }

    #[test]
    fn literal_file_actions_do_not_touch_a_matching_neighbor() {
        let repo = Repo::new();
        repo.write("item[1].txt", "old\n"); repo.write("item1.txt", "old\n"); repo.commit();
        repo.write("item[1].txt", "new\n"); repo.write("item1.txt", "neighbor\n");
        stage(repo.cwd(), &["item[1].txt".into()]).unwrap();
        let files = changed_files(repo.cwd()).unwrap();
        assert_eq!(files.iter().filter(|f| !f.index.is_empty()).count(), 1);
        discard(repo.cwd(), &["item[1].txt".into()]).unwrap();
        assert_eq!(std::fs::read_to_string(repo.0.join("item1.txt")).unwrap(), "neighbor\n");
    }

    #[test]
    fn root_merge_and_renamed_history_files_are_readable() {
        let repo = Repo::new();
        repo.write("original.txt", "one\n"); let root = repo.commit();
        assert_eq!(commit_files(repo.cwd(), &root).unwrap().len(), 1);
        assert!(commit_file_diff(repo.cwd(), &root, "original.txt").unwrap().original.is_empty());
        repo.git(&["checkout", "-qb", "feature"]);
        repo.write("feature.txt", "feature\n"); repo.commit();
        repo.git(&["checkout", "-q", "main"]);
        repo.git(&["merge", "--no-ff", "-m", "merge", "feature"]);
        let merge = repo.git(&["rev-parse", "HEAD"]);
        assert_eq!(commit_files(repo.cwd(), &merge).unwrap()[0].path, "feature.txt");
        let top = log_page_at(repo.cwd(), 1, 0, None).unwrap().remove(0);
        assert_eq!(top.parents.len(), 2);
        assert!(top.refs.iter().any(|r| r.starts_with("HEAD -> ")), "{:?}", top.refs);
        // Commits made within one second have no guaranteed order, so look for the root by content.
        assert_eq!(log_page_at(repo.cwd(), 10, 0, None).unwrap().iter().filter(|c| c.parents.is_empty()).count(), 1);
        repo.git(&["mv", "original.txt", "新名.txt"]); let rename = repo.commit();
        let diff = commit_file_diff(repo.cwd(), &rename, "新名.txt").unwrap();
        assert_eq!(diff.original, "one\n"); assert_eq!(diff.modified, "one\n");
    }

    #[test]
    fn oversized_text_is_not_reported_as_binary_and_directories_are_not_empty_diffs() {
        let repo = Repo::new();
        repo.write("large.txt", &"a".repeat(DIFF_LIMIT + 20));
        let diff = repo.diff("large.txt", "unstaged");
        assert!(diff.too_large); assert!(!diff.binary);
        std::fs::create_dir(repo.0.join("directory")).unwrap();
        assert!(file_diff_at(repo.cwd(), "directory", "all", &repo.0.join("directory")).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn symlink_diff_shows_the_link_target_instead_of_reading_its_destination() {
        let repo = Repo::new();
        repo.write("target.txt", "private content\n");
        std::os::unix::fs::symlink("target.txt", repo.0.join("link.txt")).unwrap();
        assert_eq!(repo.diff("link.txt", "unstaged").modified, "target.txt");
    }

    #[test]
    fn change_list_stops_at_the_limit_without_line_counts() {
        let repo = Repo::new();
        for i in 0..5 { repo.write(&format!("file-{i}.txt"), "one\ntwo\n"); }
        let cut = changed_file_list_limited(repo.cwd(), 3).unwrap();
        assert!(cut.truncated);
        assert_eq!(cut.files.len(), 3);
        assert!(cut.files.iter().all(|f| f.additions == 0 && f.unstaged_additions == 0));
        let full = changed_file_list_limited(repo.cwd(), 5).unwrap();
        assert!(!full.truncated);
        assert!(full.files.iter().all(|f| f.unstaged_additions == 2));
    }

    #[test]
    fn large_untracked_files_are_listed_without_reading_them() {
        let repo = Repo::new();
        repo.write("small.txt", "one\n");
        repo.write("large.txt", &"line\n".repeat(LINE_COUNT_MAX_FILE_BYTES as usize / 5 + 10));
        let files = changed_files(repo.cwd()).unwrap();
        assert_eq!(files.iter().find(|f| f.path == "small.txt").unwrap().unstaged_additions, 1);
        let large = files.iter().find(|f| f.path == "large.txt").unwrap();
        assert_eq!((large.unstaged_additions, large.worktree.as_str()), (0, "untracked"));
    }

    fn untracked(paths: &[String]) -> Vec<PathStatus> {
        paths.iter().map(|p| PathStatus { path: p.clone(), original_path: None, xy: "??".into() }).collect()
    }

    #[test]
    fn suggestion_prefers_the_outermost_dependency_folder() {
        let mut paths: Vec<String> = (0..20).map(|i| format!("pkg/node_modules/.pnpm/x/node_modules/f{i}")).collect();
        paths.extend((0..5).map(|i| format!("pkg/node_modules/y/f{i}")));
        paths.extend((0..12).map(|i| format!("admin/src/f{i}")));
        assert_eq!(suggest_ignore_folder(&untracked(&paths), 10).as_deref(), Some("pkg/node_modules"));
    }

    #[test]
    fn suggestion_is_the_deepest_folder_that_brings_the_list_under_the_limit() {
        let mut paths: Vec<String> = (0..30).map(|i| format!("plans/impl/run/a/f{i}")).collect();
        paths.extend((0..30).map(|i| format!("plans/impl/run/b/f{i}")));
        paths.extend((0..3).map(|i| format!("docs/f{i}")));
        assert_eq!(suggest_ignore_folder(&untracked(&paths), 10).as_deref(), Some("plans/impl/run"));
    }

    #[test]
    fn no_suggestion_when_changes_are_spread_out() {
        let paths: Vec<String> = ["a", "b", "c", "d"].iter()
            .flat_map(|d| (0..10).map(move |i| format!("{d}/f{i}"))).collect();
        assert_eq!(suggest_ignore_folder(&untracked(&paths), 10), None);
    }

    #[test]
    fn ignoring_accepts_only_the_current_suggestion() {
        let repo = Repo::new();
        repo.write("readme.md", "x\n"); repo.commit();
        for i in 0..GIT_STATUS_LIMIT / 10 { repo.write(&format!("pkg/node_modules/m/f{i}.js"), "x\n"); }
        assert_eq!(ignore_suggestion(repo.cwd()).unwrap().as_deref(), Some("pkg/node_modules"));
        assert!(ignore_folder(repo.cwd(), "pkg").is_err());
        assert!(ignore_folder(repo.cwd(), "pkg/node_modules").unwrap());
        assert_eq!(std::fs::read_to_string(repo.0.join(".gitignore")).unwrap(), "/pkg/node_modules/\n");
        assert_eq!(ignore_suggestion(repo.cwd()).unwrap(), None);
        assert!(changed_files(repo.cwd()).unwrap().iter().all(|f| !f.path.starts_with("pkg/")));
    }

    #[test]
    fn gitignore_lines_are_escaped_appended_once_and_start_on_their_own_line() {
        let repo = Repo::new();
        let file = repo.0.join(".gitignore");
        std::fs::write(&file, "existing").unwrap();
        assert!(append_gitignore(&file, "we*ird [x]").unwrap());
        assert!(!append_gitignore(&file, "we*ird [x]").unwrap());
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "existing\n/we\\*ird \\[x]/\n");
        assert!(append_gitignore(&file, "bad\nname").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn gitignore_symlinks_are_not_written_through() {
        let repo = Repo::new();
        std::fs::write(repo.0.join("elsewhere"), "").unwrap();
        std::os::unix::fs::symlink("elsewhere", repo.0.join(".gitignore")).unwrap();
        assert!(append_gitignore(&repo.0.join(".gitignore"), "build").is_err());
        assert_eq!(std::fs::read_to_string(repo.0.join("elsewhere")).unwrap(), "");
    }
}
