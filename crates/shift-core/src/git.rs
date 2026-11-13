//! The clean start for a coder session: every diff later reviewed is the
//! coder's alone, on its own branch with a known base.

use std::ffi::OsStr;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Where a coder session's work lands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkBranch {
    /// `HEAD` when the session started (full sha). Reviews diff against it.
    pub base: String,
    /// The `shift/<slug>` branch now checked out.
    pub branch: String,
}

/// Refuse a dirty working tree; otherwise record `HEAD` as the base and
/// check out a new `shift/<slug>` branch from it. shift never commits there.
pub fn start_work_branch(repo: &Path, slug: &str) -> io::Result<WorkBranch> {
    git(repo, &["rev-parse", "--is-inside-work-tree"])
        .map_err(|_| io::Error::other(format!("`{}` is not a git repository", repo.display())))?;
    let base = git(repo, &["rev-parse", "--verify", "HEAD"])
        .map_err(|_| io::Error::other("the repository has no commits yet; make one first"))?;
    // Untracked, non-ignored files count: a review would include them, even
    // when the user's config hides them from `git status`.
    if !git(repo, &["status", "--porcelain", "--untracked-files=normal"])?.is_empty() {
        return Err(io::Error::other(
            "the working tree has uncommitted changes; commit or stash first",
        ));
    }
    let branch = format!("shift/{slug}");
    git(repo, &["checkout", "--quiet", "-b", &branch])?;
    Ok(WorkBranch { base, branch })
}

/// The coder's working tree, frozen for one review.
#[derive(Debug)]
pub struct Snapshot {
    repo: PathBuf,
    /// The commit holding the snapshot. No branch points at it.
    pub commit: String,
    /// A temporary worktree with the snapshot checked out.
    pub worktree: PathBuf,
}

/// Commit the working tree of `repo` (tracked changes plus untracked,
/// non-ignored files) without touching its branch or index, and check that
/// commit out as a detached worktree at `worktree`.
pub fn snapshot(repo: &Path, worktree: &Path) -> io::Result<Snapshot> {
    // A scratch index keeps the coder's own index as it is.
    let index = worktree.with_extension("index");
    let index_env = [("GIT_INDEX_FILE", index.as_os_str())];
    let commit = (|| {
        git_env(repo, &["read-tree", "HEAD"], &index_env)?;
        git_env(repo, &["add", "--all"], &index_env)?;
        let tree = git_env(repo, &["write-tree"], &index_env)?;
        git(
            repo,
            &[
                "-c",
                "user.name=shift",
                "-c",
                "user.email=shift@localhost",
                "commit-tree",
                &tree,
                "-p",
                "HEAD",
                "-m",
                "shift review snapshot",
            ],
        )
    })();
    let _ = std::fs::remove_file(&index);
    // From here on, dropping the snapshot removes the worktree: git keeps a
    // worktree it has created even when `worktree add` then fails.
    let snapshot = Snapshot {
        repo: repo.to_owned(),
        commit: commit?,
        worktree: worktree.to_owned(),
    };
    let path = worktree.to_string_lossy();
    git(
        repo,
        &[
            "worktree",
            "add",
            "--quiet",
            "--detach",
            &path,
            &snapshot.commit,
        ],
    )?;
    Ok(snapshot)
}

impl Drop for Snapshot {
    /// The worktree only lives for one review.
    fn drop(&mut self) {
        let path = self.worktree.to_string_lossy();
        let _ = git(&self.repo, &["worktree", "remove", "--force", &path]);
    }
}

/// Run `git` in `repo` and return its trimmed stdout, or its stderr as the
/// error.
fn git(repo: &Path, args: &[&str]) -> io::Result<String> {
    git_env(repo, args, &[])
}

fn git_env(repo: &Path, args: &[&str], env: &[(&str, &OsStr)]) -> io::Result<String> {
    let output = Command::new("git")
        .args(args)
        .envs(env.iter().copied())
        .current_dir(repo)
        .output()
        .map_err(|e| io::Error::new(e.kind(), format!("could not run `git`: {e}")))?;
    if !output.status.success() {
        return Err(io::Error::other(format!(
            "git {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

#[cfg(test)]
pub(crate) mod tests {
    use std::path::PathBuf;

    use super::*;

    /// A temp directory removed on drop.
    pub(crate) struct Temp(pub(crate) PathBuf);

    impl Temp {
        pub(crate) fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("shift-git-{}-{name}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
    }

    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// A repo with one commit, on branch `feature`.
    pub(crate) fn repo(name: &str) -> Temp {
        let temp = Temp::new(name);
        let dir = &temp.0;
        git(dir, &["init", "--quiet", "--initial-branch=feature"]).unwrap();
        std::fs::write(dir.join("a.txt"), "hello\n").unwrap();
        git(dir, &["add", "-A"]).unwrap();
        git(
            dir,
            &[
                "-c",
                "user.name=shift",
                "-c",
                "user.email=shift@example.com",
                "commit",
                "--quiet",
                "-m",
                "init",
            ],
        )
        .unwrap();
        temp
    }

    #[test]
    fn clean_tree_records_head_and_checks_out_the_work_branch() {
        let temp = repo("clean");
        let head = git(&temp.0, &["rev-parse", "HEAD"]).unwrap();

        let work = start_work_branch(&temp.0, "abc12345").unwrap();

        assert_eq!(work.base, head);
        assert_eq!(work.branch, "shift/abc12345");
        assert_eq!(
            git(&temp.0, &["branch", "--show-current"]).unwrap(),
            "shift/abc12345"
        );
        assert_eq!(git(&temp.0, &["rev-parse", "HEAD"]).unwrap(), head);
    }

    #[test]
    fn dirty_tree_is_refused_and_left_alone() {
        for (name, file) in [("tracked", "a.txt"), ("untracked", "new.txt")] {
            let temp = repo(name);
            std::fs::write(temp.0.join(file), "changed\n").unwrap();

            let error = start_work_branch(&temp.0, "abc12345")
                .unwrap_err()
                .to_string();

            assert!(error.contains("commit or stash first"), "{name}: {error}");
            assert_eq!(
                git(&temp.0, &["branch", "--show-current"]).unwrap(),
                "feature"
            );
        }
    }

    #[test]
    fn untracked_files_hidden_by_config_still_refuse() {
        let temp = repo("hidden");
        git(&temp.0, &["config", "status.showUntrackedFiles", "no"]).unwrap();
        std::fs::write(temp.0.join("new.txt"), "x\n").unwrap();

        assert!(start_work_branch(&temp.0, "abc12345").is_err());
        assert_eq!(
            git(&temp.0, &["branch", "--show-current"]).unwrap(),
            "feature"
        );
    }

    #[test]
    fn ignored_files_do_not_make_the_tree_dirty() {
        let temp = repo("ignored");
        std::fs::write(temp.0.join(".git/info/exclude"), "target/\n").unwrap();
        std::fs::create_dir(temp.0.join("target")).unwrap();
        std::fs::write(temp.0.join("target/out"), "x").unwrap();
        assert!(start_work_branch(&temp.0, "abc12345").is_ok());
    }

    #[test]
    fn snapshot_holds_the_working_tree_and_leaves_the_checkout_alone() {
        let temp = repo("snapshot");
        let dir = &temp.0;
        std::fs::write(dir.join(".git/info/exclude"), "target/\n").unwrap();
        std::fs::write(dir.join("a.txt"), "changed\n").unwrap();
        std::fs::write(dir.join("new.txt"), "new\n").unwrap();
        std::fs::create_dir(dir.join("target")).unwrap();
        std::fs::write(dir.join("target/out"), "x").unwrap();
        let head = git(dir, &["rev-parse", "HEAD"]).unwrap();
        let status = git(dir, &["status", "--porcelain"]).unwrap();
        let worktree = Temp::new("snapshot-wt");
        std::fs::remove_dir(&worktree.0).unwrap();

        let snap = snapshot(dir, &worktree.0).unwrap();

        // The snapshot has tracked changes and untracked files, not ignored ones.
        let read = |name: &str| std::fs::read_to_string(worktree.0.join(name));
        assert_eq!(read("a.txt").unwrap(), "changed\n");
        assert_eq!(read("new.txt").unwrap(), "new\n");
        assert!(read("target/out").is_err());
        assert_eq!(
            git(&worktree.0, &["rev-parse", "HEAD"]).unwrap(),
            snap.commit
        );
        assert_eq!(
            git(&worktree.0, &["diff", "--name-only", &head]).unwrap(),
            "a.txt\nnew.txt"
        );
        // The coder's branch, index and files are as they were.
        assert_eq!(git(dir, &["rev-parse", "HEAD"]).unwrap(), head);
        assert_eq!(git(dir, &["branch", "--show-current"]).unwrap(), "feature");
        assert_eq!(git(dir, &["status", "--porcelain"]).unwrap(), status);

        drop(snap);
        assert!(!worktree.0.exists());
        assert_eq!(git(dir, &["worktree", "list"]).unwrap().lines().count(), 1);
    }

    #[test]
    fn a_failed_checkout_leaves_no_worktree_behind() {
        use std::os::unix::fs::PermissionsExt;

        let temp = repo("snapshot-hook");
        let hook = temp.0.join(".git/hooks/post-checkout");
        std::fs::write(&hook, "#!/bin/sh\nexit 1\n").unwrap();
        std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
        let worktree = Temp::new("snapshot-hook-wt");
        std::fs::remove_dir(&worktree.0).unwrap();

        assert!(snapshot(&temp.0, &worktree.0).is_err());

        assert!(!worktree.0.exists());
        assert_eq!(
            git(&temp.0, &["worktree", "list"]).unwrap().lines().count(),
            1
        );
    }

    #[test]
    fn a_directory_without_a_usable_repo_is_refused() {
        let temp = Temp::new("norepo");
        let error = start_work_branch(&temp.0, "abc12345")
            .unwrap_err()
            .to_string();
        assert!(error.contains("not a git repository"), "{error}");

        git(&temp.0, &["init", "--quiet"]).unwrap();
        let error = start_work_branch(&temp.0, "abc12345")
            .unwrap_err()
            .to_string();
        assert!(error.contains("no commits"), "{error}");
    }
}
