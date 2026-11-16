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
  searchCommits: (query: string) => invoke<number[]>("search_commits", { query }),
  rowIndexOf: (oid: string) => invoke<number | null>("row_index_of", { oid }),
  initialPath: () => invoke<string | null>("initial_path"),
};
