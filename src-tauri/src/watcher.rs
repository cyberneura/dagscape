//! .git 配下の ref の変化を監視して、フロントに "repo-changed" を送る。

use std::path::Path;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use git2::Repository;
use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher};
use tauri::{AppHandle, Emitter};

pub const EVENT: &str = "repo-changed";

/// 連続するファイル操作 (commit は HEAD / refs / ORIG_HEAD を続けて書く) を 1 回にまとめる
const DEBOUNCE: Duration = Duration::from_millis(400);

/// 返り値の watcher を drop すると監視が止まり、通知スレッドも終わる。
pub fn watch(repo: &Repository, app: AppHandle) -> Result<RecommendedWatcher, String> {
    let (tx, rx) = mpsc::channel::<notify::Result<Event>>();
    let mut watcher = notify::recommended_watcher(tx).map_err(|e| e.to_string())?;

    // worktree では HEAD が各 worktree の gitdir に、refs は共通の commondir にある
    let gitdir = repo.path().to_path_buf();
    let common = repo.commondir().to_path_buf();
    watcher
        .watch(&gitdir, RecursiveMode::NonRecursive)
        .map_err(|e| e.to_string())?;
    if common != gitdir {
        watcher
            .watch(&common, RecursiveMode::NonRecursive)
            .map_err(|e| e.to_string())?;
    }
    let refs = common.join("refs");
    if refs.is_dir() {
        watcher
            .watch(&refs, RecursiveMode::Recursive)
            .map_err(|e| e.to_string())?;
    }

    std::thread::spawn(move || {
        while let Ok(event) = rx.recv() {
            if !is_relevant(&event) {
                continue;
            }
            let deadline = Instant::now() + DEBOUNCE;
            loop {
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    break;
                }
                match rx.recv_timeout(remaining) {
                    Ok(_) => continue,
                    Err(mpsc::RecvTimeoutError::Timeout) => break,
                    Err(mpsc::RecvTimeoutError::Disconnected) => return,
                }
            }
            if app.emit(EVENT, ()).is_err() {
                return;
            }
        }
    });

    Ok(watcher)
}

fn is_relevant(event: &notify::Result<Event>) -> bool {
    let Ok(event) = event else { return false };
    event.paths.iter().any(|p| is_relevant_path(p))
}

fn is_relevant_path(path: &Path) -> bool {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    if name.ends_with(".lock") {
        return false;
    }
    if matches!(name, "HEAD" | "packed-refs" | "ORIG_HEAD") {
        return true;
    }
    path.components()
        .any(|c| c.as_os_str() == "refs")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_ref_related_paths_are_relevant() {
        assert!(is_relevant_path(Path::new("/r/.git/HEAD")));
        assert!(is_relevant_path(Path::new("/r/.git/packed-refs")));
        assert!(is_relevant_path(Path::new("/r/.git/refs/heads/main")));
        assert!(is_relevant_path(Path::new("/r/.git/refs/remotes/origin/main")));
        assert!(!is_relevant_path(Path::new("/r/.git/refs/heads/main.lock")));
        assert!(!is_relevant_path(Path::new("/r/.git/index")));
        assert!(!is_relevant_path(Path::new("/r/.git/objects/ab/cdef")));
        assert!(!is_relevant_path(Path::new("/r/.git/COMMIT_EDITMSG")));
    }
}
