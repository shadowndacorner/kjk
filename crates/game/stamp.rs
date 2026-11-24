//! The commit stamp both crates' build scripts write (`KJK_GIT_SHA`): the
//! commit, with `-dirty` when the working tree had changes at build time;
//! and the release (`KJK_VERSION`).
//! `launcher/build.rs` reads this file by path.

use std::path::Path;
use std::process::Command;

fn git(args: &[&str]) -> Option<String> {
    Command::new("git")
        .args(args)
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Stamps `KJK_GIT_SHA`, and stamps again when the commit moves (HEAD, its
/// branch, the packed refs, wherever git keeps them: in a worktree `.git`
/// is a file and they live elsewhere) or any of `sources` changes, so
/// `-dirty` holds for what is built. Only paths that exist are watched:
/// cargo reruns a script on every build while a watched path is missing.
/// The index is not watched; every `git add` and `status` touches it.
pub fn git_sha<P: AsRef<Path>>(sources: &[P]) {
    let sha = git(&["rev-parse", "HEAD"]).unwrap_or_else(|| "unknown".to_string());
    let dirty =
        git(&["status", "--porcelain", "--untracked-files=no"]).is_some_and(|out| !out.is_empty());
    println!(
        "cargo:rustc-env=KJK_GIT_SHA={sha}{}",
        if dirty { "-dirty" } else { "" }
    );
    println!("cargo:rerun-if-env-changed=KJK_RELEASE_TAG");
    let version = match std::env::var("KJK_RELEASE_TAG") {
        Ok(tag) => {
            assert!(
                !tag.is_empty()
                    && tag.len() <= 128
                    && tag.as_bytes()[0].is_ascii_alphanumeric()
                    && tag
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || b"._-".contains(&c)),
                "KJK_RELEASE_TAG must be a safe release tag of at most 128 ASCII characters"
            );
            tag
        }
        Err(std::env::VarError::NotPresent) => {
            git(&["describe", "--tags", "--match", "v*", "--abbrev=0"])
                .unwrap_or_else(|| "dev".to_string())
        }
        Err(error) => panic!("KJK_RELEASE_TAG: {error}"),
    };
    println!("cargo:rustc-env=KJK_VERSION={version}");
    for source in sources {
        println!("cargo:rerun-if-changed={}", source.as_ref().display());
    }
    // Not all of `refs/heads`: every worktree shares it, so another
    // worktree's commit would rebuild this one. A detached HEAD holds the
    // commit itself. `logs/HEAD` is this worktree's own and grows on every
    // commit, which catches the first commit after `git gc` packed the
    // branch away: its loose file did not exist, so it was not watched.
    let branch = git(&["symbolic-ref", "-q", "HEAD"]);
    for name in ["HEAD", "logs/HEAD", "packed-refs", "refs/tags"]
        .into_iter()
        .chain(branch.as_deref())
    {
        let path = git(&["rev-parse", "--path-format=absolute", "--git-path", name]);
        if let Some(path) = path.filter(|path| Path::new(path).exists()) {
            println!("cargo:rerun-if-changed={path}");
        }
    }
}
