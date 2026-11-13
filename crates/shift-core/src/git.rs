//! The clean start for a coder session: every diff later reviewed is the
//! coder's alone, on its own branch with a known base.

use std::io;
use std::path::Path;
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

/// Run `git` in `repo` and return its trimmed stdout, or its stderr as the
/// error.
fn git(repo: &Path, args: &[&str]) -> io::Result<String> {
    let output = Command::new("git")
        .args(args)
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
mod tests {
    use std::path::PathBuf;

    use super::*;

    /// A temp directory removed on drop.
    struct Temp(PathBuf);

    impl Temp {
        fn new(name: &str) -> Self {
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
    fn repo(name: &str) -> Temp {
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
