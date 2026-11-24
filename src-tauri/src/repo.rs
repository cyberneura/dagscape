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
    /// サブモジュール (gitlink)。差分は「Subproject commit <hash>」の 2 行にしかならないので、
    /// フロントは get_submodule_change で中身を取りに行く
    pub submodule: bool,
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
                submodule: delta.new_file().mode() == git2::FileMode::Commit
                    || delta.old_file().mode() == git2::FileMode::Commit,
            }
        })
        .collect();

    Ok(header(files, None))
}

fn hunk_line(hunk: &git2::DiffHunk<'_>) -> DiffLine {
    DiffLine {
        kind: "hunk",
        old_no: None,
        new_no: None,
        text: String::from_utf8_lossy(hunk.header()).trim_end().to_string(),
    }
}

/// 追加 / 削除 / 文脈の行。"\ No newline at end of file" 等は本文に混ぜないので None
fn content_line(line: &git2::DiffLine<'_>) -> Option<DiffLine> {
    let kind = match line.origin() {
        '+' => "add",
        '-' => "del",
        ' ' => "ctx",
        _ => return None,
    };
    Some(DiffLine {
        kind,
        old_no: line.old_lineno(),
        new_no: line.new_lineno(),
        text: String::from_utf8_lossy(line.content())
            .trim_end_matches(['\n', '\r'])
            .to_string(),
    })
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
            push(hunk_line(&hunk))
        }),
        Some(&mut |_, _, line| {
            match content_line(&line) {
                Some(line) => push(line),
                None => true,
            }
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

/// プレビューのために webview へ渡す blob の上限。画像・動画・PDF の実用的な大きさは入り、
/// 巨大な成果物をうっかり選んでも IPC とメモリを食い潰さない値
pub const MAX_PREVIEW_BYTES: u64 = 64 * 1024 * 1024;
/// 「このパスを足したコミット」を探す時にたどるコミット数の上限
const MAX_HISTORY_COMMITS: usize = 100_000;

#[derive(Clone, Debug, Serialize)]
pub struct BlobSide {
    pub oid: String,
    pub size: u64,
}

#[derive(Clone, Debug, Serialize)]
pub struct CommitStamp {
    pub oid: String,
    pub short: String,
    /// author の時刻 (一覧の DATE 列と同じ)
    pub time: i64,
}

#[derive(Clone, Debug, Serialize)]
pub struct FileInfo {
    /// このコミットでの blob。削除なら None
    pub new: Option<BlobSide>,
    /// 第一親での blob (rename なら旧パス)。追加なら None
    pub old: Option<BlobSide>,
    /// このパスが最初に現れたコミット。たどる上限を超えた・親が読めない (shallow clone の境界) なら None
    pub added: Option<CommitStamp>,
    /// このコミット (このファイルを変更した最新のコミット)
    pub modified: CommitStamp,
}

fn stamp(commit: &Commit<'_>) -> CommitStamp {
    let oid = commit.id().to_string();
    CommitStamp {
        short: oid[..7].to_string(),
        oid,
        time: commit.author().when().seconds(),
    }
}

/// tree の中の path にある blob の id。無い・blob でない (サブモジュール等) なら None
fn blob_id_at(tree: &git2::Tree<'_>, path: &str) -> Option<Oid> {
    let entry = tree.get_path(Path::new(path)).ok()?;
    (entry.kind() == Some(git2::ObjectType::Blob)).then(|| entry.id())
}

fn blob_side(repo: &Repository, tree: Option<&git2::Tree<'_>>, path: &str) -> Result<Option<BlobSide>, String> {
    let Some(id) = tree.and_then(|t| blob_id_at(t, path)) else {
        return Ok(None);
    };
    // 中身を読まずに大きさだけ取る
    let odb = repo.odb().map_err(|e| e.message().to_string())?;
    let (size, _) = odb.read_header(id).map_err(|e| e.message().to_string())?;
    Ok(Some(BlobSide {
        oid: id.to_string(),
        size: size as u64,
    }))
}

fn first_parent_tree<'r>(commit: &Commit<'r>) -> Result<Option<git2::Tree<'r>>, String> {
    if commit.parent_count() == 0 {
        return Ok(None);
    }
    let err = |e: git2::Error| e.message().to_string();
    Ok(Some(commit.parent(0).map_err(err)?.tree().map_err(err)?))
}

/// start から祖先をたどり、path を持たない親しか無いコミット (= path を足したコミット) のうち
/// 最も古いものを返す。path を持つコミットだけを進むので、たどる範囲はそのファイルの生存期間に限られる。
/// 親が読めない (shallow clone の境界) 時は、境界のコミットを「足した」と取り違えないよう None。
/// should_stop が true を返したら (別のファイルが選ばれた) Err で打ち切る
fn introduced_by<'r>(
    repo: &'r Repository,
    start: Commit<'r>,
    path: &str,
    should_stop: &dyn Fn() -> bool,
) -> Result<Option<Commit<'r>>, String> {
    let mut seen = std::collections::HashSet::new();
    seen.insert(start.id());
    let mut stack = vec![start];
    let mut oldest: Option<Commit<'r>> = None;
    while let Some(commit) = stack.pop() {
        if should_stop() {
            return Err("Cancelled".to_string());
        }
        let mut continues = false;
        for pid in commit.parent_ids() {
            let Ok(parent) = repo.find_commit(pid) else { return Ok(None) };
            let Ok(tree) = parent.tree() else { return Ok(None) };
            let has = blob_id_at(&tree, path).is_some();
            if !has {
                continue;
            }
            continues = true;
            if seen.insert(pid) {
                if seen.len() > MAX_HISTORY_COMMITS {
                    return Ok(None);
                }
                stack.push(parent);
            }
        }
        if !continues && oldest.as_ref().is_none_or(|o| commit.author().when().seconds() < o.author().when().seconds()) {
            oldest = Some(commit);
        }
    }
    Ok(oldest)
}

/// 差分ペインの上に出すファイルの情報: 両側の大きさ、足されたコミット、このコミット。
pub fn file_info(
    repo: &Repository,
    oid: &str,
    path: &str,
    old_path: Option<&str>,
    should_stop: &dyn Fn() -> bool,
) -> Result<FileInfo, String> {
    let commit = find_commit(repo, oid)?;
    let tree = commit.tree().map_err(|e| e.message().to_string())?;
    let parent_tree = first_parent_tree(&commit)?;
    let old_path = old_path.unwrap_or(path);
    let new = blob_side(repo, Some(&tree), path)?;
    let old = blob_side(repo, parent_tree.as_ref(), old_path)?;
    // 削除されたファイルは、親から旧パスでさかのぼる
    let added = if new.is_some() {
        introduced_by(repo, commit.clone(), path, should_stop)?
    } else if old.is_some() {
        introduced_by(repo, commit.parent(0).map_err(|e| e.message().to_string())?, old_path, should_stop)?
    } else {
        None
    };
    Ok(FileInfo {
        new,
        old,
        added: added.as_ref().map(stamp),
        modified: stamp(&commit),
    })
}

/// プレビュー用に blob の中身を返す。side は "new" (このコミット) か "old" (第一親、rename なら旧パス)
pub fn blob_bytes(
    repo: &Repository,
    oid: &str,
    path: &str,
    old_path: Option<&str>,
    side: &str,
) -> Result<Vec<u8>, String> {
    let commit = find_commit(repo, oid)?;
    let (tree, path) = match side {
        "new" => (Some(commit.tree().map_err(|e| e.message().to_string())?), path),
        "old" => (first_parent_tree(&commit)?, old_path.unwrap_or(path)),
        _ => return Err(format!("Unknown side: {side}")),
    };
    let id = tree
        .as_ref()
        .and_then(|t| blob_id_at(t, path))
        .ok_or_else(|| format!("{path} does not exist on the {side} side"))?;
    let odb = repo.odb().map_err(|e| e.message().to_string())?;
    let (size, _) = odb.read_header(id).map_err(|e| e.message().to_string())?;
    if size as u64 > MAX_PREVIEW_BYTES {
        return Err(format!("Too large to preview ({size} bytes)"));
    }
    let blob = repo.find_blob(id).map_err(|e| e.message().to_string())?;
    Ok(blob.content().to_vec())
}

/// サブモジュールの変更で列挙するコミット数の上限 (旧 → 新の間)
const MAX_SUBMODULE_COMMITS: usize = 200;
/// サブモジュールの差分の行数の上限 (全ファイルの合計)。1 ファイルの MAX_DIFF_LINES より小さいのは、
/// 選んだだけで全ファイル分を 1 回の IPC で運ぶため
const MAX_SUBMODULE_DIFF_LINES: usize = 20_000;
/// 行数が少なくても 1 行が巨大 (生成物・minify 済み) だと IPC が膨らむので、本文の合計バイト数でも切る
const MAX_SUBMODULE_DIFF_BYTES: usize = 4 * 1024 * 1024;
const MAX_SUBMODULE_FILES: usize = 2_000;
/// 「このコミットを含むブランチ」を調べる ref の数の上限。ref ごとに祖先判定が走る
const MAX_SUBMODULE_REFS: usize = 300;
const MAX_CONTAINING_BRANCHES: usize = 20;

#[derive(Clone, Debug, Serialize)]
pub struct SubmoduleRef {
    pub name: String,
    pub kind: RefKind,
}

/// サブモジュール側のコミット 1 つ (親リポジトリが指していた / 指すようになったコミット)
#[derive(Clone, Debug, Serialize)]
pub struct SubmoduleCommit {
    pub oid: String,
    pub short: String,
    pub author: Signature,
    pub committer: Signature,
    pub message: String,
    /// このコミットを直接指しているブランチ・タグ
    pub refs: Vec<SubmoduleRef>,
    /// 先端ではないが、このコミットを履歴に含むブランチ (MAX_CONTAINING_BRANCHES まで)
    pub contained_in: Vec<String>,
    pub contained_in_truncated: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct SubmoduleLogEntry {
    pub oid: String,
    pub short: String,
    pub summary: String,
    pub author: String,
    pub time: i64,
}

#[derive(Clone, Debug, Serialize)]
pub struct SubmoduleFile {
    pub path: String,
    pub old_path: Option<String>,
    pub status: String,
    pub additions: usize,
    pub deletions: usize,
    pub binary: bool,
    pub lines: Vec<DiffLine>,
}

#[derive(Clone, Debug, Serialize)]
pub struct SubmoduleChange {
    /// 親リポジトリが記録しているコミット。足された時は old、消された時は new が None
    pub old_oid: Option<String>,
    pub new_oid: Option<String>,
    /// サブモジュールのリポジトリを開けた時の、その場所 (新しいウインドウで開く先)。
    /// init されていない等で開けなければ None で、以降の項目は全部空
    pub repo_path: Option<String>,
    /// コミットがサブモジュールのリポジトリに無い (fetch されていない) 時は None
    pub old: Option<SubmoduleCommit>,
    pub new: Option<SubmoduleCommit>,
    /// old から new で増えた / 減ったコミット数。どちらかのコミットが無ければ None
    pub ahead: Option<usize>,
    pub behind: Option<usize>,
    /// 増えたコミット (新しい順、MAX_SUBMODULE_COMMITS まで)
    pub commits: Vec<SubmoduleLogEntry>,
    pub commits_truncated: bool,
    /// old と new のツリーの差分。両方のコミットがある時だけ
    pub files: Vec<SubmoduleFile>,
    pub diff_truncated: bool,
    pub diff_error: Option<String>,
}

/// tree の中の path にある gitlink (サブモジュール) が指すコミット
fn gitlink_at(tree: &git2::Tree<'_>, path: &str) -> Option<Oid> {
    let entry = tree.get_path(Path::new(path)).ok()?;
    (entry.filemode() == i32::from(git2::FileMode::Commit)).then(|| entry.id())
}

/// IPC で受けたパスを作業ツリーや .git/modules に連結する前の検査。ツリーオブジェクトは生のバイト列なので、
/// 細工したリポジトリなら ".." という名前のエントリを持てる
fn is_plain_relative(path: &str) -> bool {
    !path.is_empty() && Path::new(path).components().all(|c| matches!(c, std::path::Component::Normal(_)))
}

/// .gitmodules の中身から、path に置かれたサブモジュールの名前を引く。
/// .git/modules の下は名前で保存されていて、`git mv` した後やクローン時に --name を付けた時はパスと違う
fn submodule_name(gitmodules: &str, path: &str) -> Option<String> {
    let mut name: Option<String> = None;
    for line in gitmodules.lines() {
        let line = line.trim();
        if let Some(section) = line.strip_prefix('[') {
            name = section
                .strip_prefix("submodule")
                .map(|rest| rest.trim().trim_end_matches(']').trim().trim_matches('"').to_string());
        } else if let Some((key, value)) = line.split_once('=') {
            if key.trim() == "path" && value.trim().trim_matches('"') == path {
                return name.filter(|n| !n.is_empty());
            }
        }
    }
    None
}

/// 開けたサブモジュールと、それを新しいウインドウで開く時に渡す場所
struct OpenedSubmodule {
    repo: Repository,
    location: PathBuf,
}

/// サブモジュールのリポジトリ。作業ツリーにチェックアウトされていればそれを、無ければ
/// (履歴の途中で消されたサブモジュール、deinit 済み等) .git/modules に残っているものを開く。
/// tree は path をサブモジュールとして持つ側のツリーで、その .gitmodules から名前を引く。
/// Repository::open は上の階層を探さないので、init されていない空ディレクトリから親リポジトリ自身を
/// 開いてしまうことは無い
fn open_submodule(repo: &Repository, tree: &git2::Tree<'_>, path: &str) -> Option<OpenedSubmodule> {
    if !is_plain_relative(path) {
        return None;
    }
    if let Some(workdir) = repo.workdir() {
        let dir = workdir.join(path);
        // 途中が symlink / junction で、作業ツリーの外の無関係なリポジトリを指していたら開かない
        let inside = match (dir.canonicalize(), workdir.canonicalize()) {
            (Ok(dir), Ok(workdir)) => dir.starts_with(workdir),
            _ => false,
        };
        if inside {
            if let Ok(sub) = Repository::open(&dir) {
                let location = display_path(&sub);
                return Some(OpenedSubmodule { repo: sub, location });
            }
        }
    }
    let name = tree
        .get_path(Path::new(".gitmodules"))
        .ok()
        .and_then(|entry| repo.find_blob(entry.id()).ok())
        .and_then(|blob| submodule_name(&String::from_utf8_lossy(blob.content()), path))
        .filter(|name| is_plain_relative(name))
        .unwrap_or_else(|| path.to_string());
    // linked worktree のサブモジュールは、その worktree 用の git dir の下にある。無ければ共通の方
    let mut git_dirs = vec![repo.path().to_path_buf()];
    if repo.commondir() != repo.path() {
        git_dirs.push(repo.commondir().to_path_buf());
    }
    git_dirs.into_iter().find_map(|git_dir| {
        // core.worktree は消えた作業ツリーを指したまま (rm) か、設定ごと無い (deinit) ので、作業ツリー無しで開く。
        // 後者で libgit2 に作業ツリーを推測させると git dir の親になり、それを渡した新しいウインドウが
        // 親リポジトリを開いてしまう。場所も git dir そのものにする
        let sub = Repository::open_bare(git_dir.join("modules").join(&name)).ok()?;
        let location = PathBuf::from(sub.path().to_string_lossy().trim_end_matches(['/', '\\']));
        Some(OpenedSubmodule { repo: sub, location })
    })
}

fn submodule_commit(sub: &Repository, oid: Oid, should_stop: &dyn Fn() -> bool) -> Option<SubmoduleCommit> {
    let commit = sub.find_commit(oid).ok()?;
    let hex = oid.to_string();
    let author = signature(&commit.author());
    let committer = signature(&commit.committer());
    let mut refs = Vec::new();
    let mut contained_in = Vec::new();
    let mut contained_in_truncated = false;
    if let Ok(references) = sub.references() {
        let mut checked = 0usize;
        for reference in references.flatten() {
            let Ok(full_name) = reference.name() else { continue };
            let (kind, name) = if let Some(name) = full_name.strip_prefix("refs/heads/") {
                (RefKind::Branch, name)
            } else if let Some(name) = full_name.strip_prefix("refs/remotes/") {
                if name.ends_with("/HEAD") {
                    continue;
                }
                (RefKind::Remote, name)
            } else if let Some(name) = full_name.strip_prefix("refs/tags/") {
                (RefKind::Tag, name)
            } else {
                continue;
            };
            let Ok(tip) = reference.peel_to_commit() else { continue };
            if tip.id() == oid {
                refs.push(SubmoduleRef { name: name.to_string(), kind });
            } else if kind != RefKind::Tag {
                if contained_in.len() >= MAX_CONTAINING_BRANCHES || checked >= MAX_SUBMODULE_REFS {
                    contained_in_truncated = true;
                    continue;
                }
                // 別のファイルが選ばれたら、残りの祖先判定はしない (結果は捨てられる)
                if should_stop() {
                    break;
                }
                checked += 1;
                if sub.graph_descendant_of(tip.id(), oid).unwrap_or(false) {
                    contained_in.push(name.to_string());
                }
            }
        }
    }
    refs.sort_by(|a, b| a.name.cmp(&b.name));
    contained_in.sort();
    Some(SubmoduleCommit {
        short: hex[..7].to_string(),
        oid: hex,
        author,
        committer,
        message: commit.message().unwrap_or("").to_string(),
        refs,
        contained_in,
        contained_in_truncated,
    })
}

/// old..new のコミット (新しい順)。上限を超えたら truncated
fn submodule_log(sub: &Repository, old: Oid, new: Oid) -> Result<(Vec<SubmoduleLogEntry>, bool), String> {
    let err = |e: git2::Error| e.message().to_string();
    let mut walk = sub.revwalk().map_err(err)?;
    walk.set_sorting(Sort::TOPOLOGICAL | Sort::TIME).map_err(err)?;
    walk.push(new).map_err(err)?;
    walk.hide(old).map_err(err)?;
    let mut commits = Vec::new();
    for oid in walk {
        let oid = oid.map_err(err)?;
        if commits.len() >= MAX_SUBMODULE_COMMITS {
            return Ok((commits, true));
        }
        let commit = sub.find_commit(oid).map_err(err)?;
        let hex = oid.to_string();
        commits.push(SubmoduleLogEntry {
            short: hex[..7].to_string(),
            oid: hex,
            summary: commit.summary().ok().flatten().unwrap_or("").to_string(),
            author: commit.author().name().unwrap_or("").to_string(),
            time: commit.committer().when().seconds(),
        });
    }
    Ok((commits, false))
}

/// サブモジュール内の old → new のツリーの差分を、ファイルごとの行にして返す
fn submodule_diff(
    sub: &Repository,
    old: Oid,
    new: Oid,
    should_stop: &dyn Fn() -> bool,
) -> Result<(Vec<SubmoduleFile>, bool), String> {
    let err = |e: git2::Error| e.message().to_string();
    let old_tree = sub.find_commit(old).map_err(err)?.tree().map_err(err)?;
    let new_tree = sub.find_commit(new).map_err(err)?.tree().map_err(err)?;
    let mut diff = sub
        .diff_tree_to_tree(Some(&old_tree), Some(&new_tree), None)
        .map_err(err)?;
    diff.find_similar(Some(DiffFindOptions::new().renames(true)))
        .map_err(err)?;

    let files: RefCell<Vec<SubmoduleFile>> = RefCell::new(Vec::new());
    let total_lines = Cell::new(0usize);
    let total_bytes = Cell::new(0usize);
    let truncated = Cell::new(false);
    let push_line = |line: DiffLine| {
        if total_lines.get() >= MAX_SUBMODULE_DIFF_LINES
            || total_bytes.get() + line.text.len() > MAX_SUBMODULE_DIFF_BYTES
            || should_stop()
        {
            truncated.set(true);
            return false;
        }
        total_lines.set(total_lines.get() + 1);
        total_bytes.set(total_bytes.get() + line.text.len());
        if let Some(file) = files.borrow_mut().last_mut() {
            match line.kind {
                "add" => file.additions += 1,
                "del" => file.deletions += 1,
                _ => {}
            }
            file.lines.push(line);
        }
        true
    };
    let result = diff.foreach(
        &mut |delta, _| {
            let mut files = files.borrow_mut();
            if files.len() >= MAX_SUBMODULE_FILES {
                truncated.set(true);
                return false;
            }
            let status = delta.status();
            files.push(SubmoduleFile {
                path: if status == Delta::Deleted {
                    delta_path(delta.old_file())
                } else {
                    delta_path(delta.new_file())
                },
                old_path: matches!(status, Delta::Renamed | Delta::Copied).then(|| delta_path(delta.old_file())),
                status: status_letter(status).to_string(),
                additions: 0,
                deletions: 0,
                binary: delta.flags().is_binary(),
                lines: Vec::new(),
            });
            true
        },
        Some(&mut |_, _| {
            if let Some(file) = files.borrow_mut().last_mut() {
                file.binary = true;
            }
            true
        }),
        Some(&mut |_, hunk| {
            push_line(hunk_line(&hunk))
        }),
        Some(&mut |_, _, line| {
            match content_line(&line) {
                Some(line) => push_line(line),
                None => true,
            }
        }),
    );
    // file_diff と同じく、上限で止めた時の Err だけ許容する
    if let Err(e) = result {
        if !truncated.get() {
            return Err(e.message().to_string());
        }
    }
    Ok((files.into_inner(), truncated.get()))
}

/// コミットが動かしたサブモジュールの、旧 / 新コミットと、サブモジュールのリポジトリ (開けなければ None)
struct LocatedSubmodule {
    old_id: Option<Oid>,
    new_id: Option<Oid>,
    opened: Option<OpenedSubmodule>,
}

/// Err にするのは path がこのコミットでサブモジュールでない時だけ
fn locate_submodule(
    repo: &Repository,
    oid: &str,
    path: &str,
    old_path: Option<&str>,
) -> Result<LocatedSubmodule, String> {
    let commit = find_commit(repo, oid)?;
    let tree = commit.tree().map_err(|e| e.message().to_string())?;
    let parent_tree = first_parent_tree(&commit)?;
    let old_path = old_path.unwrap_or(path);
    let new_id = gitlink_at(&tree, path);
    let old_id = parent_tree.as_ref().and_then(|t| gitlink_at(t, old_path));
    let opened = match (new_id, old_id, parent_tree.as_ref()) {
        (Some(_), _, _) => open_submodule(repo, &tree, path),
        // 消されたサブモジュールは、親の側の旧パスと .gitmodules にある
        (None, Some(_), Some(parent_tree)) => open_submodule(repo, parent_tree, old_path),
        _ => return Err(format!("{path} is not a submodule in this commit")),
    };
    Ok(LocatedSubmodule { old_id, new_id, opened })
}

/// 新しいウインドウで開く先。webview から任意のパスを受けず、開いているリポジトリのコミットから引き直す
pub fn submodule_location(repo: &Repository, oid: &str, path: &str, old_path: Option<&str>) -> Result<PathBuf, String> {
    locate_submodule(repo, oid, path, old_path)?
        .opened
        .map(|sub| sub.location)
        .ok_or_else(|| format!("{path} is not checked out"))
}

/// コミット oid が path のサブモジュールをどう動かしたか: 旧 / 新コミットの情報、間のコミット、差分。
/// サブモジュールのリポジトリが手元に無い・コミットが fetch されていない時は、分かる範囲だけを返す。
/// should_stop が true を返したら (別のファイルが選ばれた) Err で打ち切る
pub fn submodule_change(
    repo: &Repository,
    oid: &str,
    path: &str,
    old_path: Option<&str>,
    should_stop: &dyn Fn() -> bool,
) -> Result<SubmoduleChange, String> {
    let cancelled = || Err("Cancelled".to_string());
    let LocatedSubmodule { old_id, new_id, opened } = locate_submodule(repo, oid, path, old_path)?;
    let mut change = SubmoduleChange {
        old_oid: old_id.map(|id| id.to_string()),
        new_oid: new_id.map(|id| id.to_string()),
        repo_path: None,
        old: None,
        new: None,
        ahead: None,
        behind: None,
        commits: Vec::new(),
        commits_truncated: false,
        files: Vec::new(),
        diff_truncated: false,
        diff_error: None,
    };
    let Some(OpenedSubmodule { repo: sub, location }) = opened else {
        return Ok(change);
    };
    change.repo_path = Some(location.to_string_lossy().into_owned());
    change.old = old_id.and_then(|id| submodule_commit(&sub, id, should_stop));
    change.new = new_id.and_then(|id| submodule_commit(&sub, id, should_stop));
    if should_stop() {
        return cancelled();
    }
    if let (Some(old), Some(new), true, true) = (old_id, new_id, change.old.is_some(), change.new.is_some()) {
        if let Ok((ahead, behind)) = sub.graph_ahead_behind(new, old) {
            change.ahead = Some(ahead);
            change.behind = Some(behind);
        }
        // 一覧が取れなくても増減の数と差分は出せるので、失敗は一覧が空になるだけにする
        if let Ok((commits, truncated)) = submodule_log(&sub, old, new) {
            change.commits = commits;
            change.commits_truncated = truncated;
        }
        if should_stop() {
            return cancelled();
        }
        match submodule_diff(&sub, old, new, should_stop) {
            Ok((files, truncated)) => {
                change.files = files;
                change.diff_truncated = truncated;
            }
            Err(e) => change.diff_error = Some(e),
        }
        if should_stop() {
            return cancelled();
        }
    }
    Ok(change)
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

    #[test]
    fn file_info_reports_sizes_and_the_adding_commit() {
        let dir = build_fixture("file_info");
        let repo = open(&dir).unwrap();
        let first = git(&dir, &["rev-parse", "main~3"]);
        let third = git(&dir, &["rev-parse", "main~1"]);
        let feature = git(&dir, &["rev-parse", "feature"]);
        let merge = git(&dir, &["rev-parse", "main"]);

        let a = file_info(&repo, &third, "a.txt", None, &|| false).unwrap();
        assert_eq!(a.new.as_ref().unwrap().size, "one\ntwo\nthree\n".len() as u64);
        assert_eq!(a.old.as_ref().unwrap().size, "one\ntwo\n".len() as u64);
        assert_eq!(a.added.unwrap().oid, first);
        assert_eq!(a.modified.oid, third);

        // マージで入った b.txt は第一親に無いので old は None、足したのは feature 側のコミット
        let b = file_info(&repo, &merge, "b.txt", None, &|| false).unwrap();
        assert!(b.old.is_none());
        assert_eq!(b.added.unwrap().oid, feature);

        // 削除は親から旧パスでさかのぼる
        git(&dir, &["rm", "-q", "a.txt"]);
        git(&dir, &["commit", "-q", "-m", "remove a"]);
        let removed = git(&dir, &["rev-parse", "main"]);
        let gone = file_info(&repo, &removed, "a.txt", None, &|| false).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        assert!(gone.new.is_none());
        assert!(gone.old.is_some());
        assert_eq!(gone.added.unwrap().oid, first);
    }

    #[test]
    fn blob_bytes_reads_either_side() {
        let dir = build_fixture("blob_bytes");
        let repo = open(&dir).unwrap();
        let third = git(&dir, &["rev-parse", "main~1"]);
        let new = blob_bytes(&repo, &third, "a.txt", None, "new").unwrap();
        let old = blob_bytes(&repo, &third, "a.txt", None, "old").unwrap();
        let missing = blob_bytes(&repo, &third, "b.txt", None, "new");
        let bad_side = blob_bytes(&repo, &third, "a.txt", None, "both");
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(new, b"one\ntwo\nthree\n");
        assert_eq!(old, b"one\ntwo\n");
        assert!(missing.is_err());
        assert!(bad_side.is_err());
    }

    #[test]
    fn file_info_stops_when_asked() {
        let dir = build_fixture("file_info_stop");
        let repo = open(&dir).unwrap();
        let head = git(&dir, &["rev-parse", "main"]);
        let result = file_info(&repo, &head, "a.txt", None, &|| true);
        let _ = std::fs::remove_dir_all(&dir);
        assert!(result.is_err());
    }

    /// lib (サブモジュールの元) と、それを `vendor/lib` に持つ親リポジトリ。
    /// 親は「サブモジュールを足す」「2 コミット進める」の 2 コミット。返すのは (親, 進める前, 進めた後)
    fn build_submodule_fixture(label: &str) -> (PathBuf, String, String) {
        let lib = temp_repo(&format!("{label}_lib"));
        git(&lib, &["init", "-q", "-b", "main"]);
        write(&lib, "lib.txt", "one\n");
        git(&lib, &["add", "."]);
        git(&lib, &["commit", "-q", "-m", "lib first"]);
        let old = git(&lib, &["rev-parse", "HEAD"]);

        let dir = temp_repo(label);
        git(&dir, &["init", "-q", "-b", "main"]);
        git(
            &dir,
            &["-c", "protocol.file.allow=always", "submodule", "-q", "add", lib.to_str().unwrap(), "vendor/lib"],
        );
        git(&dir, &["commit", "-q", "-m", "add submodule"]);

        let sub = dir.join("vendor/lib");
        write(&sub, "lib.txt", "one\ntwo\n");
        git(&sub, &["commit", "-q", "-am", "lib second"]);
        write(&sub, "new.txt", "new\n");
        git(&sub, &["add", "."]);
        git(&sub, &["commit", "-q", "-m", "lib third"]);
        git(&sub, &["tag", "lib-v2"]);
        let new = git(&sub, &["rev-parse", "HEAD"]);
        git(&dir, &["commit", "-q", "-am", "bump submodule"]);
        (dir, old, new)
    }

    #[test]
    fn submodule_change_reports_commits_and_diff() {
        let (dir, old, new) = build_submodule_fixture("submodule");
        let repo = open(&dir).unwrap();
        let head = git(&dir, &["rev-parse", "HEAD"]);

        let detail = commit_detail(&repo, &head).unwrap();
        assert_eq!(detail.files.len(), 1);
        assert!(detail.files[0].submodule);
        assert_eq!(detail.files[0].path, "vendor/lib");

        let change = submodule_change(&repo, &head, "vendor/lib", None, &|| false).unwrap();
        assert_eq!(change.old_oid.as_deref(), Some(old.as_str()));
        assert_eq!(change.new_oid.as_deref(), Some(new.as_str()));
        assert!(change.repo_path.as_deref().unwrap().ends_with("lib"));
        assert_eq!((change.ahead, change.behind), (Some(2), Some(0)));
        let summaries: Vec<&str> = change.commits.iter().map(|c| c.summary.as_str()).collect();
        assert_eq!(summaries, ["lib third", "lib second"]);
        assert!(!change.commits_truncated);

        let new_commit = change.new.unwrap();
        assert_eq!(new_commit.message.trim(), "lib third");
        assert_eq!(new_commit.author.email, "tester@example.com");
        assert!(new_commit.refs.iter().any(|r| r.name == "lib-v2" && r.kind == RefKind::Tag));
        let old_commit = change.old.unwrap();
        assert_eq!(old_commit.message.trim(), "lib first");
        // 旧コミットは先端ではないが、進んだ先のブランチに含まれる
        assert!(old_commit.refs.iter().all(|r| r.kind != RefKind::Tag));
        assert!(!old_commit.contained_in.is_empty());

        let paths: Vec<&str> = change.files.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(paths, ["lib.txt", "new.txt"]);
        assert_eq!((change.files[0].additions, change.files[0].deletions), (1, 0));
        assert_eq!(change.files[1].status, "A");
        assert!(change.files[0].lines.iter().any(|l| l.kind == "add" && l.text == "two"));
        assert!(!change.diff_truncated);
        assert!(change.diff_error.is_none());
    }

    #[test]
    fn submodule_change_for_an_added_submodule_has_no_old_side() {
        let (dir, old, _) = build_submodule_fixture("submodule_added");
        let repo = open(&dir).unwrap();
        let added = git(&dir, &["rev-parse", "HEAD~1"]);
        let change = submodule_change(&repo, &added, "vendor/lib", None, &|| false).unwrap();
        assert_eq!(change.old_oid, None);
        assert_eq!(change.new_oid.as_deref(), Some(old.as_str()));
        assert!(change.old.is_none());
        assert_eq!(change.new.unwrap().message.trim(), "lib first");
        // 足された時は差分 (= サブモジュールの全ファイル) もコミット一覧 (= 全履歴) も出さない
        assert!(change.commits.is_empty() && change.files.is_empty());
    }

    #[test]
    fn submodule_change_falls_back_to_hashes_without_a_checkout() {
        let (dir, old, new) = build_submodule_fixture("submodule_bare");
        // サブモジュールを init していない clone
        let clone = temp_repo("submodule_clone");
        git(&clone, &["clone", "-q", dir.to_str().unwrap(), "."]);
        let repo = open(&clone).unwrap();
        let head = git(&clone, &["rev-parse", "HEAD"]);
        let change = submodule_change(&repo, &head, "vendor/lib", None, &|| false).unwrap();
        assert_eq!(change.old_oid.as_deref(), Some(old.as_str()));
        assert_eq!(change.new_oid.as_deref(), Some(new.as_str()));
        assert!(change.repo_path.is_none());
        assert!(change.old.is_none() && change.new.is_none());
        assert!(change.files.is_empty());
    }

    #[test]
    fn submodule_change_rejects_ordinary_files() {
        let dir = build_fixture("submodule_plain");
        let repo = open(&dir).unwrap();
        let head = git(&dir, &["rev-parse", "HEAD"]);
        assert!(submodule_change(&repo, &head, "a.txt", None, &|| false).is_err());
        assert!(!commit_detail(&repo, &head).unwrap().files.iter().any(|f| f.submodule));
    }

    #[test]
    fn submodule_name_is_read_from_gitmodules() {
        let text = "[submodule \"libs/renamed\"]\n\tpath = vendor/lib\n\turl = ../lib.git\n[submodule \"other\"]\n  path=other/dir\n";
        assert_eq!(submodule_name(text, "vendor/lib").as_deref(), Some("libs/renamed"));
        assert_eq!(submodule_name(text, "other/dir").as_deref(), Some("other"));
        assert_eq!(submodule_name(text, "missing"), None);
    }

    #[test]
    fn submodule_paths_must_stay_inside_the_repository() {
        assert!(is_plain_relative("vendor/lib"));
        for path in ["", "../lib", "vendor/../../lib", "/etc", "./lib"] {
            assert!(!is_plain_relative(path), "{path}");
        }
    }

    #[test]
    fn a_removed_submodule_is_read_from_the_module_cache() {
        let (dir, _, new) = build_submodule_fixture("submodule_removed");
        // 名前とパスが違う状態にしてから消す。.git/modules は名前 (vendor/lib) のまま残る
        git(&dir, &["mv", "vendor/lib", "third_party"]);
        git(&dir, &["commit", "-q", "-m", "move submodule"]);
        git(&dir, &["rm", "-q", "third_party"]);
        git(&dir, &["commit", "-q", "-m", "remove submodule"]);
        let repo = open(&dir).unwrap();
        let head = git(&dir, &["rev-parse", "HEAD"]);

        let change = submodule_change(&repo, &head, "third_party", None, &|| false).unwrap();
        assert_eq!(change.old_oid.as_deref(), Some(new.as_str()));
        assert_eq!(change.new_oid, None);
        assert_eq!(change.old.unwrap().message.trim(), "lib third");
        // 開く先は git dir そのもの。推測された作業ツリー (= 親リポジトリの中) にしない
        let location = submodule_location(&repo, &head, "third_party", None).unwrap();
        assert!(location.ends_with("modules/vendor/lib"), "{}", location.display());
        assert_eq!(change.repo_path.as_deref(), location.to_str());
    }

    #[test]
    fn submodule_change_stops_when_asked() {
        let (dir, _, _) = build_submodule_fixture("submodule_stop");
        let repo = open(&dir).unwrap();
        let head = git(&dir, &["rev-parse", "HEAD"]);
        assert_eq!(
            submodule_change(&repo, &head, "vendor/lib", None, &|| true).unwrap_err(),
            "Cancelled"
        );
    }
}
