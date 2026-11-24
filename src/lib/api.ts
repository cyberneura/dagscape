import { invoke } from "@tauri-apps/api/core";

export type RefKind = "head" | "branch" | "remote" | "tag";

/** For a remote ref, itself; for a local branch, its upstream. */
export interface Tracking {
  remote: string;
  branch: string;
}

export interface RefLabel {
  name: string;
  kind: RefKind;
  is_head: boolean;
  tracking: Tracking | null;
}

export interface RefInfo {
  full_name: string;
  name: string;
  kind: RefKind;
  oid: string;
  is_head: boolean;
  tracking: Tracking | null;
}

export interface RemoteInfo {
  name: string;
  web_url: string | null;
}

export interface Segment {
  from: number;
  to: number;
  color: number;
}

export interface Row {
  index: number;
  oid: string;
  short: string;
  summary: string;
  author: string;
  email: string;
  time: number;
  parents: string[];
  lane: number;
  color: number;
  segments: Segment[];
  refs: RefLabel[];
}

export interface HeadInfo {
  name: string | null;
  oid: string;
  detached: boolean;
}

export interface RepoSummary {
  path: string;
  name: string;
  total: number;
  max_lanes: number;
  refs: RefInfo[];
  head: HeadInfo | null;
  remotes: RemoteInfo[];
  include_remotes: boolean;
  load_ms: number;
}

export interface Signature {
  name: string;
  email: string;
  time: number;
  offset_minutes: number;
}

export interface FileChange {
  path: string;
  old_path: string | null;
  status: string;
  additions: number;
  deletions: number;
  binary: boolean;
  /** A submodule (gitlink): its diff is only the two "Subproject commit" lines. */
  submodule: boolean;
}

export interface CommitDetail {
  oid: string;
  short: string;
  parents: string[];
  author: Signature;
  committer: Signature;
  message: string;
  files: FileChange[];
  diff_error: string | null;
}

export interface DiffLine {
  kind: "ctx" | "add" | "del" | "hunk";
  old_no: number | null;
  new_no: number | null;
  text: string;
}

export interface FileDiff {
  path: string;
  binary: boolean;
  truncated: boolean;
  lines: DiffLine[];
}

export interface BlobSide {
  oid: string;
  size: number;
}

export interface CommitStamp {
  oid: string;
  short: string;
  time: number;
}

export interface FileInfo {
  /** The blob at this commit; null when the file was deleted. */
  new: BlobSide | null;
  /** The blob at the first parent (old path for a rename); null when the file was added. */
  old: BlobSide | null;
  /** The commit that first added the path; null when the history walk hit its limit. */
  added: CommitStamp | null;
  modified: CommitStamp;
}

export type BlobSideName = "new" | "old";

export interface SubmoduleRef {
  name: string;
  kind: RefKind;
}

/** A commit inside the submodule: the one the superproject pointed at, or points at now. */
export interface SubmoduleCommit {
  oid: string;
  short: string;
  author: Signature;
  committer: Signature;
  message: string;
  /** Branches and tags that point at this very commit. */
  refs: SubmoduleRef[];
  /** Branches that have it in their history without pointing at it (capped). */
  contained_in: string[];
  contained_in_truncated: boolean;
}

export interface SubmoduleLogEntry {
  oid: string;
  short: string;
  summary: string;
  author: string;
  time: number;
}

export interface SubmoduleFile {
  path: string;
  old_path: string | null;
  status: string;
  additions: number;
  deletions: number;
  binary: boolean;
  lines: DiffLine[];
}

export interface SubmoduleChange {
  /** Recorded by the superproject; old is null when the submodule was added, new when removed. */
  old_oid: string | null;
  new_oid: string | null;
  /** Where the submodule's repository is; null when it is not checked out (everything below is then empty). */
  repo_path: string | null;
  /** null when the commit is not in the submodule's repository (not fetched). */
  old: SubmoduleCommit | null;
  new: SubmoduleCommit | null;
  ahead: number | null;
  behind: number | null;
  /** Commits gained from old to new, newest first (capped). */
  commits: SubmoduleLogEntry[];
  commits_truncated: boolean;
  /** Diff between the two commits' trees, when both are present. */
  files: SubmoduleFile[];
  diff_truncated: boolean;
  diff_error: string | null;
}

export const api = {
  openRepo: (path: string, includeRemotes: boolean) =>
    invoke<RepoSummary>("open_repo", { path, includeRemotes }),
  reloadRepo: (includeRemotes?: boolean) =>
    invoke<RepoSummary>("reload_repo", { includeRemotes: includeRemotes ?? null }),
  closeRepo: () => invoke<void>("close_repo"),
  getRows: (start: number, end: number) => invoke<Row[]>("get_rows", { start, end }),
  getCommit: (oid: string) => invoke<CommitDetail>("get_commit", { oid }),
  getFileDiff: (oid: string, path: string, oldPath: string | null) =>
    invoke<FileDiff>("get_file_diff", { oid, path, oldPath }),
  getFileInfo: (oid: string, path: string, oldPath: string | null) =>
    invoke<FileInfo>("get_file_info", { oid, path, oldPath }),
  /** Raw bytes (an ArrayBuffer over IPC, not a JSON array). */
  getBlob: (oid: string, path: string, oldPath: string | null, side: BlobSideName) =>
    invoke<ArrayBuffer>("get_blob", { oid, path, oldPath, side }),
  getSubmoduleChange: (oid: string, path: string, oldPath: string | null) =>
    invoke<SubmoduleChange>("get_submodule_change", { oid, path, oldPath }),
  /** Starts another Dagscape process on the submodule that commit `oid` moved at `path`. */
  openSubmoduleWindow: (oid: string, path: string, oldPath: string | null) =>
    invoke<void>("open_submodule_window", { oid, path, oldPath }),
  searchCommits: (query: string) => invoke<number[]>("search_commits", { query }),
  rowIndexOf: (oid: string) => invoke<number | null>("row_index_of", { oid }),
  initialPath: () => invoke<string | null>("initial_path"),
};
