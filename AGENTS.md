# Dagscape - AI Agent Development Guide

## Project Overview

Dagscape is a desktop branch graph browser for Git (a GitX successor). It shows every ref of a
repository as a lane graph. Tauri 2 + SvelteKit (Svelte 5 runes) + Tailwind 4 on the front,
Rust with `git2` (libgit2, vendored) on the back.

The name deliberately does not contain "git": the Git trademark policy forbids "Git◯◯" style
names and even "git" as a syllable in a new word. Use "a branch graph browser for Git" as the
tagline.

## Architecture

```
dagscape/
├── src/
│   ├── routes/+page.svelte        # App shell: toolbar, sidebar, list, detail; keyboard; store
│   └── lib/
│       ├── api.ts                 # Typed invoke() wrappers and the IPC types
│       ├── rowCache.ts            # Chunked (256 rows) cache of commit rows fetched over IPC
│       ├── graph.ts               # Canvas drawing of lanes (ROW_H, LANE_W, palette)
│       ├── CommitList.svelte      # Virtual scroller + Canvas overlay for the graph
│       ├── CommitDetail.svelte    # Commit header, file list, unified diff
│       └── RefSidebar.svelte      # Branches / remotes / tags
└── src-tauri/src/
    ├── lib.rs                     # Tauri commands, app state (one open repository), menu
    ├── repo.rs                    # git2: refs, revwalk, rows, commit detail, file diff, search
    ├── graph.rs                   # Lane layout (pure, unit-tested, no git dependency)
    └── watcher.rs                 # notify watcher on .git → "repo-changed" event
```

## How the graph works

1. `repo::load_graph` pushes every ref tip into a libgit2 revwalk sorted
   `TOPOLOGICAL | TIME` (git's `--date-order`), so children always come before parents.
2. Each commit is fed to `graph::LayoutBuilder::push(id, parents)`. The builder keeps a vector
   of active lanes, each waiting for a specific commit. A commit lands in the leftmost lane that
   waits for it (or a new lane); other lanes waiting for it are merged into that lane by
   redirecting the previous row's segments. The first parent continues the lane; further parents
   get an edge to the lane that already waits for them or a newly allocated lane. Freed lanes are
   reused from the left to keep the graph narrow.
3. Each `Row` carries `lane`, `color` and `segments` (lines between this row and the next). The
   frontend fetches rows in chunks and draws only the visible range (± 1 row) on a Canvas that
   overlays the virtual list. Row text is ordinary DOM, positioned absolutely.

All rows live in the Rust side (`AppState.session.graph`); the webview keeps at most 48 chunks
(12k rows) in an LRU cache (`rowCache.ts`), so browsing the whole history does not accumulate it.

## Tauri commands (`src-tauri/src/lib.rs`)

- `open_repo(path, include_remotes)` → `RepoSummary` (also starts the `.git` watcher)
- `reload_repo(include_remotes?)` → `RepoSummary`
- `get_rows(start, end)` → `Row[]`
- `get_commit(oid)` → `CommitDetail` (diff against the first parent, renames detected)
- `get_file_diff(oid, path, old_path?)` → `FileDiff` (capped at 50k lines)
- `search_commits(query)` → row indices
- `row_index_of(oid)` → index or null
- `initial_path()` → the CLI argument, once

Events emitted to the webview: `repo-changed` (watcher), `menu` (payload: `open-repo`,
`reload-repo`, `find`).

## Conventions

- Svelte 5 runes only (`$state`, `$derived`, `$effect`, `$props`). Write nullable state as
  `let x = $state<T | null>(null)`; the annotated form `let x: T | null = $state(null)` makes
  TypeScript narrow `x` to `null` forever.
- Keyboard navigation is handled once, on `window`, in `+page.svelte`.
- Comments explain non-obvious *why* only (hidden constraints, workarounds, surprising behaviour).
- `git2` 0.21: `Reference::name()`, `Commit::summary()` (`Result<Option<&str>>`) and
  `Signature::name()` / `email()` all return `Result`, not `Option`.

## Development Commands

```bash
pnpm install
pnpm tauri dev                     # app with HMR
pnpm check                         # svelte-check
pnpm build                         # frontend only (needed before cargo test: tauri-build wants ../build)
cd src-tauri && cargo test         # Rust unit tests (graph layout, repo access, watcher filter)
pnpm tauri build                   # release bundle
pnpm tauri build --debug --bundles app   # quick .app for manual testing
pnpm tauri icon path/to/icon.png   # regenerate icons
pnpm release [patch|minor|major]   # bump version, push, watch the release workflow
```

Launch with a repository from the shell: `src-tauri/target/debug/bundle/macos/Dagscape.app/Contents/MacOS/dagscape /path/to/repo`.

## Release

Same scheme as `ytyng/arcvault`: `.github/workflows/release.yml` runs on every push to `main`,
reads the version from `src-tauri/tauri.conf.json`, and releases only if `v<version>` is not a
published Release yet (test → build matrix macOS universal dmg / Windows NSIS → publish). macOS
builds need the `APPLE_*` repository secrets (Developer ID certificate + notarization). The
Homebrew cask lives in `cyberneura/homebrew-tap` and updates itself from the latest Release.

## Testing

- `src-tauri/src/graph.rs` tests cover the lane algorithm with small synthetic DAGs.
- `src-tauri/src/repo.rs` tests build a real repository with the `git` CLI in a temp directory
  (needs `git` on PATH).
- The UI has no automated tests; verify by launching the app against a real repository.
