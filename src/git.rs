use std::fs;
use std::path::{Path, PathBuf};
use std::process::{self, Command, Output};

use anyhow::{bail, Context, Result};

use crate::paths;

pub const SNAPSHOT_REFS: &str = "refs/verifique/snapshots";

const EMPTY_TREE: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";

fn output(cwd: &Path, args: &[&str], envs: &[(&str, &Path)]) -> Result<Output> {
    let mut command = Command::new("git");
    command.current_dir(cwd).args(args);
    for (name, value) in envs {
        command.env(name, value);
    }
    command.output().context("running git")
}

fn git_with(cwd: &Path, args: &[&str], envs: &[(&str, &Path)]) -> Result<String> {
    let out = output(cwd, args, envs)?;
    if !out.status.success() {
        bail!("git {}: {}", args.join(" "), String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn git(cwd: &Path, args: &[&str]) -> Result<String> {
    git_with(cwd, args, &[])
}

/// The main checkout, so that every linked worktree of a repository shares one identity.
pub fn repo_root(cwd: &Path) -> Result<String> {
    let common = git(cwd, &["rev-parse", "--path-format=absolute", "--git-common-dir"])?;
    let root = Path::new(&common).parent().context("repository has no parent directory")?;
    Ok(root.to_string_lossy().into_owned())
}

pub fn worktree_root(cwd: &Path) -> Result<PathBuf> {
    Ok(PathBuf::from(git(cwd, &["rev-parse", "--show-toplevel"])?))
}

pub fn current_branch(cwd: &Path) -> Result<Option<String>> {
    let branch = git(cwd, &["rev-parse", "--abbrev-ref", "HEAD"])?;
    Ok((branch != "HEAD").then_some(branch))
}

pub struct Snapshot {
    pub commit: String,
    pub tree: String,
    pub dirty: bool,
}

/// The working tree as a commit, without touching the branch, the real index or any file. Starting
/// from a copy of the index keeps git's cached file stats, so large repositories stay quick.
/// Ignored files stay out, exactly as they would from a commit.
pub fn snapshot(cwd: &Path) -> Result<Snapshot> {
    let root = worktree_root(cwd)?;
    let head = git(&root, &["rev-parse", "HEAD"])?;
    let head_tree = git(&root, &["rev-parse", "HEAD^{tree}"])?;

    let real_index = git(&root, &["rev-parse", "--path-format=absolute", "--git-path", "index"])?;
    let temp_dir = paths::home().join("tmp");
    fs::create_dir_all(&temp_dir)?;
    let temp_index = temp_dir.join(format!("index-{}", process::id()));
    if Path::new(&real_index).exists() {
        fs::copy(&real_index, &temp_index)?;
    }
    let tree = git_with(&root, &["add", "-A"], &[("GIT_INDEX_FILE", &temp_index)])
        .and_then(|_| git_with(&root, &["write-tree"], &[("GIT_INDEX_FILE", &temp_index)]));
    let _ = fs::remove_file(&temp_index);
    let tree = tree?;

    if tree == head_tree {
        return Ok(Snapshot { commit: head, tree, dirty: false });
    }
    let commit = git(
        &root,
        &[
            "-c", "user.name=verifique", "-c", "user.email=verifique@localhost",
            "commit-tree", &tree, "-p", &head, "-m", "verifique snapshot",
        ],
    )?;
    // A ref keeps the snapshot alive until it has been checked.
    git(&root, &["update-ref", &format!("{SNAPSHOT_REFS}/{tree}"), &commit])?;
    Ok(Snapshot { commit, tree, dirty: true })
}

/// What the diff is taken against: where this work left the base branch.
pub fn diff_base(cwd: &Path, configured: Option<&str>, commit: &str) -> Option<String> {
    let candidates = configured
        .map(|base| vec![base])
        .unwrap_or_else(|| vec!["origin/HEAD", "origin/main", "origin/master", "main", "master"]);
    candidates
        .iter()
        .find_map(|base| git(cwd, &["merge-base", base, commit]).ok())
        .filter(|base| base != commit)
        .or_else(|| git(cwd, &["rev-parse", "--verify", &format!("{commit}^")]).ok())
}

pub fn diff(repo: &Path, base: Option<&str>, commit: &str) -> Result<Vec<u8>> {
    let out = output(repo, &["diff", "--no-color", base.unwrap_or(EMPTY_TREE), commit], &[])?;
    if !out.status.success() {
        bail!("git diff: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(out.stdout)
}

/// Each slot keeps one worktree per repository and moves it between commits. `clean` without -x
/// keeps ignored dependencies and build output, which is what makes the next build incremental.
pub fn checkout_slot(repo: &Path, path: &Path, commit: &str) -> Result<()> {
    let parent = path.parent().context("slot worktree has no parent")?;
    fs::create_dir_all(parent)?;
    fs::write(parent.join("repo"), repo.to_string_lossy().as_bytes())?;
    if !path.exists() {
        git(repo, &["worktree", "prune"])?;
        let target = path.to_string_lossy();
        git(repo, &["worktree", "add", "--detach", "--force", &target, commit])?;
        return Ok(());
    }
    git(path, &["reset", "--hard"])?;
    git(path, &["checkout", "--detach", "--force", commit])?;
    git(path, &["clean", "-fd"])?;
    Ok(())
}

pub fn remove_worktree(repo: &Path, path: &Path) -> Result<()> {
    git(repo, &["worktree", "remove", "--force", &path.to_string_lossy()])?;
    Ok(())
}

pub fn delete_snapshot_refs(repo: &Path, keep: &[String]) -> Result<usize> {
    let refs = git(repo, &["for-each-ref", "--format=%(refname)", SNAPSHOT_REFS])?;
    let mut removed = 0;
    for reference in refs.lines() {
        if !keep.iter().any(|tree| reference.ends_with(tree.as_str())) {
            git(repo, &["update-ref", "-d", reference])?;
            removed += 1;
        }
    }
    Ok(removed)
}
