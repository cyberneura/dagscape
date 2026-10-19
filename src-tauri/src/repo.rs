//! git2 (libgit2) でリポジトリを読み、グラフ・コミット詳細・差分を組み立てる。

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use git2::{Commit, Delta, DiffFindOptions, DiffOptions, Direction, Oid, Remote, Repository, Sort};
use serde::Serialize;

use crate::github;
use crate::graph::{LayoutBuilder, Segment};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RefKind {
    /// detached HEAD の印
    Head,
    Branch,
    Remote,
    Tag,
}

/// リモート上のブランチ。remote 種別の ref ならそれ自身、ローカルブランチなら upstream。
/// リモート名に "/" が入り得るので、"origin/feature" を分けたものをここで持たせる
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Tracking {
    pub remote: String,
    pub branch: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct RefLabel {
    pub name: String,
    pub kind: RefKind,
    pub is_head: bool,
    pub tracking: Option<Tracking>,
}

#[derive(Clone, Debug, Serialize)]
pub struct RefInfo {
    pub full_name: String,
    pub name: String,
    pub kind: RefKind,
    pub oid: String,
    pub is_head: bool,
    pub tracking: Option<Tracking>,
}

#[derive(Clone, Debug, Serialize)]
pub struct RemoteInfo {
    pub name: String,
    /// GitHub のリモートなら https://host/owner/repo
    pub web_url: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Row {
    pub index: usize,
    pub oid: String,
    pub short: String,
    pub summary: String,
    pub author: String,
    pub email: String,
    /// author date (unix 秒)
    pub time: i64,
    pub parents: Vec<String>,
    pub lane: u16,
    pub color: u16,
    pub segments: Vec<Segment>,
    pub refs: Vec<RefLabel>,
}

#[derive(Clone, Debug, Serialize)]
pub struct HeadInfo {
    pub name: Option<String>,
    pub oid: String,
    pub detached: bool,
}

pub struct Graph {
    pub rows: Vec<Row>,
    pub index_by_oid: HashMap<String, usize>,
    pub refs: Vec<RefInfo>,
    pub head: Option<HeadInfo>,
    pub remotes: Vec<RemoteInfo>,
    pub max_lanes: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct Signature {
    pub name: String,
    pub email: String,
    pub time: i64,
    pub offset_minutes: i32,
}

#[derive(Clone, Debug, Serialize)]
pub struct FileChange {
    pub path: String,
    pub old_path: Option<String>,
    /// A / M / D / R / C / T のいずれか
    pub status: String,
    pub additions: usize,
    pub deletions: usize,
    pub binary: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct CommitDetail {
    pub oid: String,
    pub short: String,
    pub parents: Vec<String>,
    pub author: Signature,
    pub committer: Signature,
    pub message: String,
    pub files: Vec<FileChange>,
    /// 差分が取れなかった理由 (partial clone で blob が無い等)。ヘッダとメッセージは出せるので別建て
    pub diff_error: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct DiffLine {
    /// ctx / add / del / hunk
    pub kind: &'static str,
    pub old_no: Option<u32>,
    pub new_no: Option<u32>,
    pub text: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct FileDiff {
    pub path: String,
    pub binary: bool,
    pub truncated: bool,
    pub lines: Vec<DiffLine>,
}

const MAX_DIFF_LINES: usize = 50_000;
const MAX_SEARCH_HITS: usize = 20_000;

pub fn open(path: &Path) -> Result<Repository, String> {
    Repository::discover(path).map_err(|e| format!("Not a git repository: {} ({})", path.display(), e.message()))
}

/// 表示用のパス。作業ツリーがあればその親、bare なら .git 自体。
pub fn display_path(repo: &Repository) -> PathBuf {
    let path = repo.workdir().unwrap_or_else(|| repo.path());
    // libgit2 は末尾に "/" を付けて返す
    PathBuf::from(path.to_string_lossy().trim_end_matches(['/', '\\']))
}

pub fn load_graph(repo: &Repository, include_remotes: bool) -> Result<Graph, String> {
    let err = |e: git2::Error| e.message().to_string();

    let head_ref = repo.head().ok();
    let detached = repo.head_detached().unwrap_or(false);
    let head_oid = head_ref.as_ref().and_then(|h| h.target());
    let head_branch = if detached {
        None
    } else {
        head_ref.as_ref().and_then(|h| h.name().ok().map(str::to_string))
    };

    let mut walk = repo.revwalk().map_err(err)?;
    walk.set_sorting(Sort::TOPOLOGICAL | Sort::TIME).map_err(err)?;

    let remotes = open_remotes(repo);
    let mut labels: HashMap<Oid, Vec<RefLabel>> = HashMap::new();
    let mut refs = Vec::new();
    for reference in repo.references().map_err(err)? {
        let reference = reference.map_err(err)?;
        let Ok(full_name) = reference.name() else { continue };
        let (kind, name) = if let Some(name) = full_name.strip_prefix("refs/heads/") {
            (RefKind::Branch, name)
        } else if let Some(name) = full_name.strip_prefix("refs/remotes/") {
            if !include_remotes || name.ends_with("/HEAD") {
                continue;
            }
            (RefKind::Remote, name)
        } else if let Some(name) = full_name.strip_prefix("refs/tags/") {
            (RefKind::Tag, name)
        } else {
            continue;
        };
        // コミット以外を指すタグ (blob / tree) は表示できないので飛ばす
        let Ok(commit) = reference.peel_to_commit() else { continue };
        let oid = commit.id();
        let is_head = kind == RefKind::Branch && head_branch.as_deref() == Some(full_name);
        let tracking = match kind {
            RefKind::Remote => remote_branch_of(&remotes, full_name),
            RefKind::Branch => upstream_of(repo, &remotes, full_name),
            _ => None,
        };
        labels.entry(oid).or_default().push(RefLabel {
            name: name.to_string(),
            kind,
            is_head,
            tracking: tracking.clone(),
        });
        refs.push(RefInfo {
            full_name: full_name.to_string(),
            name: name.to_string(),
            kind,
            oid: oid.to_string(),
            is_head,
            tracking,
        });
        walk.push(oid).map_err(err)?;
    }
    if detached {
        if let Some(oid) = head_oid {
            labels.entry(oid).or_default().insert(
                0,
                RefLabel {
                    name: "HEAD".to_string(),
                    kind: RefKind::Head,
                    is_head: true,
                    tracking: None,
                },
            );
            walk.push(oid).map_err(err)?;
        }
    }
    refs.sort_by(|a, b| a.name.cmp(&b.name));

    let head = head_oid.map(|oid| HeadInfo {
        name: head_branch
            .as_deref()
            .and_then(|n| n.strip_prefix("refs/heads/"))
            .map(str::to_string),
        oid: oid.to_string(),
        detached,
    });

    let mut builder = LayoutBuilder::new();
    let mut rows = Vec::new();
    let mut index_by_oid = HashMap::new();
    for oid in walk {
        let oid = oid.map_err(err)?;
        let commit = repo.find_commit(oid).map_err(err)?;
        let parents: Vec<Oid> = commit.parent_ids().collect();
        builder.push(oid, &parents);
        let author = commit.author();
        let hex = oid.to_string();
        index_by_oid.insert(hex.clone(), rows.len());
        rows.push(Row {
            index: rows.len(),
            short: hex[..7].to_string(),
            oid: hex,
            summary: commit.summary().ok().flatten().unwrap_or("").to_string(),
            author: author.name().unwrap_or("").to_string(),
            email: author.email().unwrap_or("").to_string(),
            time: author.when().seconds(),
            parents: parents.iter().map(Oid::to_string).collect(),
            lane: 0,
            color: 0,
            segments: Vec::new(),
            refs: labels.remove(&oid).unwrap_or_default(),
        });
    }
    let (layouts, max_lanes) = builder.finish();
    for (row, layout) in rows.iter_mut().zip(layouts) {
        row.lane = layout.lane;
        row.color = layout.color;
        row.segments = layout.segments;
    }

    Ok(Graph {
        rows,
        index_by_oid,
        refs,
        head,
        remotes: remotes
            .iter()
            .filter_map(|r| {
                Some(RemoteInfo {
                    name: r.name().ok()??.to_string(),
                    web_url: r.url().ok().and_then(github::web_url),
                })
            })
            .collect(),
        max_lanes,
    })
}

fn open_remotes(repo: &Repository) -> Vec<Remote<'_>> {
    let Ok(names) = repo.remotes() else { return Vec::new() };
    names
        .iter()
        .flatten()
        .flatten()
        .filter_map(|name| repo.find_remote(name).ok())
        .collect()
}

/// "refs/remotes/origin/x" のリモート上の名前。fetch refspec を逆に当てて求める。
/// リモート名に "/" が入ることも、refspec で別の名前に写していることもあるので、名前を切り分けない
fn remote_branch_of(remotes: &[Remote<'_>], full_name: &str) -> Option<Tracking> {
    remotes.iter().find_map(|remote| {
        let spec = remote
            .refspecs()
            .find(|spec| spec.direction() == Direction::Fetch && spec.dst_matches(full_name))?;
        let src = spec.rtransform(full_name).ok()?;
        Some(Tracking {
            remote: remote.name().ok()??.to_string(),
            branch: src.as_str().ok()?.strip_prefix("refs/heads/")?.to_string(),
        })
    })
}

/// branch.<name>.remote / merge をそのまま読む。remote が "." (ローカルブランチを追跡) なら None。
/// upstream を設定せずに push したブランチは、GitHub のリモートに同名のリモート追跡 ref が実在する時だけ
/// (origin 優先) それとみなす。GitHub でないリモートを選んでもリンクを作れないので候補にしない。
/// 実在を確かめずに同名を推測すると、未 push なら 404、別物の同名ブランチがあればそれを開いてしまう
fn upstream_of(repo: &Repository, remotes: &[Remote<'_>], full_name: &str) -> Option<Tracking> {
    let Ok(remote) = repo.branch_upstream_remote(full_name) else {
        let branch = full_name.strip_prefix("refs/heads/")?;
        let mut ordered: Vec<&Remote<'_>> = remotes
            .iter()
            .filter(|r| r.url().ok().and_then(github::web_url).is_some())
            .collect();
        ordered.sort_by_key(|r| r.name().ok().flatten() != Some("origin"));
        return ordered.into_iter().find_map(|r| {
            let spec = r
                .refspecs()
                .find(|spec| spec.direction() == Direction::Fetch && spec.src_matches(full_name))?;
            let dst = spec.transform(full_name).ok()?;
            repo.find_reference(dst.as_str().ok()?).ok()?;
            Some(Tracking {
                remote: r.name().ok()??.to_string(),
                branch: branch.to_string(),
            })
        });
    };
    let remote = remote.as_str().ok()?;
    if remote == "." {
        return None;
    }
    let merge = repo.branch_upstream_merge(full_name).ok()?;
    Some(Tracking {
        remote: remote.to_string(),
        branch: merge.as_str().ok()?.strip_prefix("refs/heads/")?.to_string(),
    })
}

pub fn search(graph: &Graph, query: &str) -> Vec<usize> {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return Vec::new();
    }
    graph
        .rows
        .iter()
        .filter(|row| {
            row.oid.starts_with(&query)
                || row.summary.to_lowercase().contains(&query)
                || row.author.to_lowercase().contains(&query)
                || row.email.to_lowercase().contains(&query)
                || row.refs.iter().any(|r| r.name.to_lowercase().contains(&query))
        })
        .map(|row| row.index)
        .take(MAX_SEARCH_HITS)
        .collect()
}

fn signature(sig: &git2::Signature<'_>) -> Signature {
    Signature {
        name: sig.name().unwrap_or("").to_string(),
        email: sig.email().unwrap_or("").to_string(),
        time: sig.when().seconds(),
        offset_minutes: sig.when().offset_minutes(),
    }
}

fn find_commit<'r>(repo: &'r Repository, oid: &str) -> Result<Commit<'r>, String> {
    let oid = Oid::from_str(oid).map_err(|e| e.message().to_string())?;
    repo.find_commit(oid).map_err(|e| e.message().to_string())
}

/// 第一親との差分。root コミットは空ツリーとの差分になる。
fn diff_against_first_parent<'r>(
    repo: &'r Repository,
    commit: &Commit<'r>,
    opts: &mut DiffOptions,
) -> Result<git2::Diff<'r>, String> {
    let err = |e: git2::Error| e.message().to_string();
    let tree = commit.tree().map_err(err)?;
    // 親が無い (root) のと、親が読めない (shallow clone の境界等) のは別。後者は空ツリーとの差分にしない
    let parent_tree = if commit.parent_count() == 0 {
        None
    } else {
        Some(commit.parent(0).map_err(err)?.tree().map_err(err)?)
    };
    let mut diff = repo
        .diff_tree_to_tree(parent_tree.as_ref(), Some(&tree), Some(opts))
        .map_err(err)?;
    diff.find_similar(Some(DiffFindOptions::new().renames(true)))
        .map_err(err)?;
    Ok(diff)
}

fn status_letter(status: Delta) -> &'static str {
    match status {
        Delta::Added => "A",
        Delta::Deleted => "D",
        Delta::Modified => "M",
        Delta::Renamed => "R",
        Delta::Copied => "C",
        Delta::Typechange => "T",
        _ => "M",
    }
}

fn delta_path(file: git2::DiffFile<'_>) -> String {
    file.path().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default()
}

pub fn commit_detail(repo: &Repository, oid: &str) -> Result<CommitDetail, String> {
    let commit = find_commit(repo, oid)?;
    let hex = commit.id().to_string();
    let author = signature(&commit.author());
    let committer = signature(&commit.committer());
    let header = |files, diff_error| CommitDetail {
        short: hex[..7].to_string(),
        oid: hex.clone(),
        parents: commit.parent_ids().map(|p| p.to_string()).collect(),
        author: author.clone(),
        committer: committer.clone(),
        message: commit.message().unwrap_or("").to_string(),
        files,
        diff_error,
    };
    let diff = match diff_against_first_parent(repo, &commit, &mut DiffOptions::new()) {
        Ok(diff) => diff,
        Err(e) => return Ok(header(Vec::new(), Some(e))),
    };

    let count = diff.deltas().len();
    // file_cb と line_cb の両方から同じ Vec を触るため RefCell。current は「今どの delta の行か」
    let counts = RefCell::new(vec![(0usize, 0usize, false); count]);
    let current = Cell::new(usize::MAX);
    diff.foreach(
        &mut |_, _| {
            current.set(current.get().wrapping_add(1));
            true
        },
        Some(&mut |_, _| {
            if let Some(c) = counts.borrow_mut().get_mut(current.get()) {
                c.2 = true;
            }
            true
        }),
        None,
        Some(&mut |_, _, line| {
            if let Some(c) = counts.borrow_mut().get_mut(current.get()) {
                match line.origin() {
                    '+' => c.0 += 1,
                    '-' => c.1 += 1,
                    _ => {}
                }
            }
            true
        }),
    )
    .map_err(|e| e.message().to_string())?;
    let counts = counts.into_inner();

    let files = diff
        .deltas()
        .enumerate()
        .map(|(i, delta)| {
            let status = delta.status();
            let old_path = matches!(status, Delta::Renamed | Delta::Copied)
                .then(|| delta_path(delta.old_file()));
            let path = if status == Delta::Deleted {
                delta_path(delta.old_file())
            } else {
                delta_path(delta.new_file())
            };
            let (additions, deletions, binary) = counts.get(i).copied().unwrap_or((0, 0, false));
            FileChange {
                path,
                old_path,
                status: status_letter(status).to_string(),
                additions,
                deletions,
                binary: binary || delta.flags().is_binary(),
            }
        })
        .collect();

    Ok(header(files, None))
}

pub fn file_diff(
    repo: &Repository,
    oid: &str,
    path: &str,
    old_path: Option<&str>,
) -> Result<FileDiff, String> {
    let commit = find_commit(repo, oid)?;
    let mut opts = DiffOptions::new();
    // ファイル名をそのままパターンにすると "[ab].txt" のような名前が他のファイルにも一致する
    opts.disable_pathspec_match(true);
    opts.pathspec(path);
    // rename は旧パスの削除 + 新パスの追加として出てくるので、両方を pathspec に入れてから
    // find_similar で 1 つの rename に畳む
    if let Some(old) = old_path {
        opts.pathspec(old);
    }
    let diff = diff_against_first_parent(repo, &commit, &mut opts)?;

    let lines = RefCell::new(Vec::new());
    let binary = Cell::new(false);
    let truncated = Cell::new(false);
    let push = |line: DiffLine| {
        let mut lines = lines.borrow_mut();
        if lines.len() >= MAX_DIFF_LINES {
            truncated.set(true);
            return false;
        }
        lines.push(line);
        true
    };
    let result = diff.foreach(
        &mut |_, _| true,
        Some(&mut |_, _| {
            binary.set(true);
            true
        }),
        Some(&mut |_, hunk| {
            push(DiffLine {
                kind: "hunk",
                old_no: None,
                new_no: None,
                text: String::from_utf8_lossy(hunk.header()).trim_end().to_string(),
            })
        }),
        Some(&mut |_, _, line| {
            let kind = match line.origin() {
                '+' => "add",
                '-' => "del",
                ' ' => "ctx",
                // "\ No newline at end of file" 等は本文に混ぜない
                _ => return true,
            };
            push(DiffLine {
                kind,
                old_no: line.old_lineno(),
                new_no: line.new_lineno(),
                text: String::from_utf8_lossy(line.content())
                    .trim_end_matches(['\n', '\r'])
                    .to_string(),
            })
        }),
    );
    // コールバックが false を返して止めた (行数上限) 時だけ Err を許容する。blob 欠損等は返す
    if let Err(e) = result {
        if !truncated.get() {
            return Err(e.message().to_string());
        }
    }

    Ok(FileDiff {
        path: path.to_string(),
        binary: binary.get(),
        truncated: truncated.get(),
        lines: lines.into_inner(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    /// テスト用の一時リポジトリ。プロセス ID + 連番で一意にする。
    fn temp_repo(label: &str) -> PathBuf {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "dagscape_test_{}_{}_{}",
            label,
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn git(dir: &Path, args: &[&str]) -> String {
        let out = Command::new("git")
            .args(args)
            .current_dir(dir)
            .env("GIT_AUTHOR_NAME", "Tester")
            .env("GIT_AUTHOR_EMAIL", "tester@example.com")
            .env("GIT_COMMITTER_NAME", "Tester")
            .env("GIT_COMMITTER_EMAIL", "tester@example.com")
            .output()
            .expect("git is installed");
        assert!(out.status.success(), "git {:?} failed: {}", args, String::from_utf8_lossy(&out.stderr));
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    fn write(dir: &Path, name: &str, content: &str) {
        std::fs::write(dir.join(name), content).unwrap();
    }

    /// main に 2 コミット、feature で 1 コミット、main にマージ、タグ 1 つ
    fn build_fixture(label: &str) -> PathBuf {
        let dir = temp_repo(label);
        git(&dir, &["init", "-q", "-b", "main"]);
        write(&dir, "a.txt", "one\n");
        git(&dir, &["add", "."]);
        git(&dir, &["commit", "-q", "-m", "first"]);
        write(&dir, "a.txt", "one\ntwo\n");
        git(&dir, &["commit", "-q", "-am", "second"]);
        git(&dir, &["checkout", "-q", "-b", "feature"]);
        write(&dir, "b.txt", "feature\n");
        git(&dir, &["add", "."]);
        git(&dir, &["commit", "-q", "-m", "feature work"]);
        git(&dir, &["checkout", "-q", "main"]);
        write(&dir, "a.txt", "one\ntwo\nthree\n");
        git(&dir, &["commit", "-q", "-am", "third"]);
        git(&dir, &["merge", "-q", "--no-ff", "-m", "merge feature", "feature"]);
        git(&dir, &["tag", "v1"]);
        dir
    }

    #[test]
    fn graph_lists_all_commits_with_refs_and_lanes() {
        let dir = build_fixture("graph");
        let repo = open(&dir).unwrap();
        let graph = load_graph(&repo, true).unwrap();
        let _ = std::fs::remove_dir_all(&dir);

        assert_eq!(graph.rows.len(), 5);
        assert_eq!(graph.max_lanes, 2);
        let head = graph.head.unwrap();
        assert_eq!(head.name.as_deref(), Some("main"));
        assert!(!head.detached);

        let top = &graph.rows[0];
        assert_eq!(top.summary, "merge feature");
        assert_eq!(top.parents.len(), 2);
        let names: Vec<&str> = top.refs.iter().map(|r| r.name.as_str()).collect();
        assert!(names.contains(&"main"));
        assert!(names.contains(&"v1"));
        assert!(top.refs.iter().any(|r| r.kind == RefKind::Branch && r.is_head));

        assert_eq!(graph.rows.last().unwrap().summary, "first");
        assert!(graph.rows.last().unwrap().segments.is_empty());
        assert_eq!(graph.refs.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(), vec!["feature", "main", "v1"]);
        assert_eq!(graph.index_by_oid[&top.oid], 0);
    }

    #[test]
    fn remote_refs_and_upstreams_carry_the_remote_and_branch() {
        let dir = build_fixture("tracking");
        git(&dir, &["remote", "add", "origin", "git@github.com:owner/app.git"]);
        git(&dir, &["remote", "add", "team/fork", "https://gitlab.com/team/app.git"]);
        git(&dir, &["update-ref", "refs/remotes/origin/topic/x", "feature"]);
        git(&dir, &["update-ref", "refs/remotes/team/fork/main", "main"]);
        git(&dir, &["branch", "--set-upstream-to=origin/topic/x", "feature"]);
        git(&dir, &["remote", "add", "mirror", "git@github.com:owner/mirror.git"]);
        git(&dir, &["config", "remote.mirror.fetch", "+refs/heads/*:refs/remotes/mirror/prefix/*"]);
        git(&dir, &["update-ref", "refs/remotes/mirror/prefix/release", "main"]);
        git(&dir, &["config", "branch.main.remote", "mirror"]);
        git(&dir, &["config", "branch.main.merge", "refs/heads/release"]);
        git(&dir, &["branch", "pushed", "main"]);
        git(&dir, &["update-ref", "refs/remotes/origin/pushed", "main"]);
        git(&dir, &["branch", "unpushed", "main"]);
        git(&dir, &["branch", "--track", "local-only", "main"]);
        git(&dir, &["branch", "both", "main"]);
        git(&dir, &["remote", "add", "gitlab", "https://gitlab.com/team/app.git"]);
        git(&dir, &["update-ref", "refs/remotes/gitlab/both", "main"]);
        git(&dir, &["update-ref", "refs/remotes/mirror/prefix/both", "main"]);
        let repo = open(&dir).unwrap();
        let graph = load_graph(&repo, true).unwrap();
        let _ = std::fs::remove_dir_all(&dir);

        let web: Vec<(&str, Option<&str>)> =
            graph.remotes.iter().map(|r| (r.name.as_str(), r.web_url.as_deref())).collect();
        assert!(web.contains(&("origin", Some("https://github.com/owner/app"))));
        assert!(web.contains(&("team/fork", None)));

        let tracking = |name: &str| graph.refs.iter().find(|r| r.name == name).unwrap().tracking.clone();
        let t = |remote: &str, branch: &str| Some(Tracking { remote: remote.into(), branch: branch.into() });
        assert_eq!(tracking("feature"), t("origin", "topic/x"));
        assert_eq!(tracking("main"), t("mirror", "release"));
        assert_eq!(tracking("origin/topic/x"), t("origin", "topic/x"));
        assert_eq!(tracking("team/fork/main"), t("team/fork", "main"));
        assert_eq!(tracking("mirror/prefix/release"), t("mirror", "release"));
        assert_eq!(tracking("pushed"), t("origin", "pushed"));
        assert_eq!(tracking("unpushed"), None);
        assert_eq!(tracking("local-only"), None);
        assert_eq!(tracking("both"), t("mirror", "both"));
    }

    #[test]
    fn detail_and_file_diff_follow_the_first_parent() {
        let dir = build_fixture("detail");
        let repo = open(&dir).unwrap();
        let graph = load_graph(&repo, false).unwrap();
        let third = graph.rows.iter().find(|r| r.summary == "third").unwrap();

        let detail = commit_detail(&repo, &third.oid).unwrap();
        assert_eq!(detail.files.len(), 1);
        assert_eq!(detail.files[0].path, "a.txt");
        assert_eq!(detail.files[0].status, "M");
        assert_eq!((detail.files[0].additions, detail.files[0].deletions), (1, 0));
        assert_eq!(detail.author.name, "Tester");
        assert_eq!(detail.message.trim(), "third");

        let diff = file_diff(&repo, &third.oid, "a.txt", None).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        assert!(!diff.binary);
        assert_eq!(diff.lines[0].kind, "hunk");
        let added: Vec<&str> = diff.lines.iter().filter(|l| l.kind == "add").map(|l| l.text.as_str()).collect();
        assert_eq!(added, vec!["three"]);
        assert_eq!(diff.lines.iter().filter(|l| l.kind == "ctx").count(), 2);
    }

    #[test]
    fn file_diff_matches_the_path_literally_not_as_a_glob() {
        let dir = temp_repo("glob");
        git(&dir, &["init", "-q", "-b", "main"]);
        write(&dir, "a.txt", "a\n");
        write(&dir, "[ab].txt", "bracket\n");
        git(&dir, &["add", "."]);
        git(&dir, &["commit", "-q", "-m", "first"]);
        write(&dir, "a.txt", "a\nchanged a\n");
        write(&dir, "[ab].txt", "bracket\nchanged bracket\n");
        git(&dir, &["commit", "-q", "-am", "second"]);
        let repo = open(&dir).unwrap();
        let graph = load_graph(&repo, false).unwrap();
        let second = &graph.rows[0];

        let diff = file_diff(&repo, &second.oid, "[ab].txt", None).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        let added: Vec<&str> = diff.lines.iter().filter(|l| l.kind == "add").map(|l| l.text.as_str()).collect();
        assert_eq!(added, vec!["changed bracket"]);
    }

    #[test]
    fn search_matches_summary_author_and_oid_prefix() {
        let dir = build_fixture("search");
        let repo = open(&dir).unwrap();
        let graph = load_graph(&repo, false).unwrap();
        let _ = std::fs::remove_dir_all(&dir);

        assert_eq!(search(&graph, "FEATURE").len(), 2);
        assert_eq!(search(&graph, "tester").len(), 5);
        assert_eq!(search(&graph, &graph.rows[2].oid[..6]), vec![2]);
        assert!(search(&graph, "   ").is_empty());
    }

    #[test]
    fn opening_a_plain_directory_fails() {
        let dir = temp_repo("plain");
        let result = open(&dir);
        let _ = std::fs::remove_dir_all(&dir);
        assert!(result.is_err());
    }
}
