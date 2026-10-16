import { invoke } from "@tauri-apps/api/core";

export type RefKind = "head" | "branch" | "remote" | "tag";

export interface RefLabel {
  name: string;
  kind: RefKind;
  is_head: boolean;
}

export interface RefInfo {
  full_name: string;
  name: string;
  kind: RefKind;
  oid: string;
  is_head: boolean;
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
  searchCommits: (query: string) => invoke<number[]>("search_commits", { query }),
  rowIndexOf: (oid: string) => invoke<number | null>("row_index_of", { oid }),
  initialPath: () => invoke<string | null>("initial_path"),
};
