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
│   ├── routes/licenses/           # Third-Party Licenses window (prerendered for WebviewUrl::App)
│   └── lib/
│       ├── api.ts                 # Typed invoke() wrappers and the IPC types
│       ├── rowCache.ts            # Chunked (256 rows) cache of commit rows fetched over IPC
│       ├── graph.ts               # Canvas drawing of lanes (ROW_H, LANE_W, palette)
│       ├── github.ts              # GitHub URLs for refs / commits / files / diffs; native context menu
│       ├── CommitList.svelte      # Virtual scroller + Canvas overlay for the graph
│       ├── CommitDetail.svelte    # Commit header, file list, unified diff
│       └── RefSidebar.svelte      # Branches / remotes / tags
└── src-tauri/src/
    ├── lib.rs                     # Tauri commands, app state (one open repository), menu
    ├── cli.rs                     # `dagscape` command (symlink in /usr/local/bin), --license
    ├── notices_tests.rs           # THIRD-PARTY-NOTICES.txt is in step with the lockfiles
    ├── repo.rs                    # git2: refs, revwalk, rows, commit detail, file diff, search
    ├── github.rs                  # Remote URL (https / ssh / scp-like) → GitHub web URL
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

- `open_repo(path, include_remotes)` → `RepoSummary` (also starts the `.git` watcher; `remotes`
  carries each remote's GitHub web URL, and every ref its `tracking` remote branch)
- `reload_repo(include_remotes?)` → `RepoSummary`
- `get_rows(start, end)` → `Row[]`
- `get_commit(oid)` → `CommitDetail` (diff against the first parent, renames detected)
- `get_file_diff(oid, path, old_path?)` → `FileDiff` (capped at 50k lines)
- `search_commits(query)` → row indices
- `row_index_of(oid)` → index or null
- `initial_path()` → the CLI argument, once
- `third_party_notices()` → Dagscape's LICENSE followed by THIRD-PARTY-NOTICES.txt (embedded)

Events emitted to the webview: `repo-changed` (watcher), `menu` (payload: `open-repo`,
`reload-repo`, `find`).

## Conventions

- Svelte 5 runes only (`$state`, `$derived`, `$effect`, `$props`). Write nullable state as
  `let x = $state<T | null>(null)`; the annotated form `let x: T | null = $state(null)` makes
  TypeScript narrow `x` to `null` forever.
- Keyboard navigation is handled once, on `window`, in `+page.svelte`.
- The three pane separators (sidebar | list, list / detail, file list | diff) resize through
  `lib/resize.ts`: `dragResize` (pointer capture on the handle, ends on up / cancel / lost capture)
  and `keyResize` (focusable `role="separator"`, arrow keys / Home / End, 16 px steps). The sizes are
  persisted in the settings store as `sidebarWidth` / `detailHeight` / `fileListWidth` (px) when a
  drag or key step ends, and clamped to the same bounds on restore and when the window (or, for the
  file list, its pane) shrinks. The file list's width is a `$bindable` prop of `CommitDetail`, so the
  shell owns the store and the component owns the drag.
- File paths in the detail's file list truncate at the *start* (`.truncate-start` in `app.css`:
  outer box `direction: rtl`, inner span isolated back to `ltr`), because the tail is what tells
  files apart.
- Comments explain non-obvious *why* only (hidden constraints, workarounds, surprising behaviour).
- `git2` 0.21: `Reference::name()`, `Commit::summary()` / `Remote::name()` (`Result<Option<&str>>`),
  `Remote::url()`, `Buf::as_str()` and `Signature::name()` / `email()` all return `Result`, not
  `Option`; `StringArray::iter()` yields `Result<Option<&str>>`. `repo.remotes()` is alphabetical.

## Development Commands

```bash
pnpm install
pnpm tauri dev                     # app with HMR
pnpm check                         # svelte-check
pnpm build                         # frontend only (needed before cargo test: tauri-build wants ../build)
cd src-tauri && cargo test         # Rust unit tests (graph layout, repo access, watcher filter)
pnpm tauri build                   # release bundle
pnpm tauri build --debug --bundles app   # quick .app for manual testing
pnpm notices                       # regenerate THIRD-PARTY-NOTICES.txt (`--check` in CI)
pnpm release [patch|minor|major]   # bump version, push, watch the release workflow
```

## Command line

Install Command Line Tools… (macOS app menu) symlinks `/usr/local/bin/dagscape` to the app's
executable. Started through a symlink, `run()` acts as a client instead of the GUI: it resolves
the path against the caller's directory and starts the app with `open -n` (one process per
repository; the bare executable would tie the GUI to the terminal). `dagscape --license` prints
the licenses in either mode.

Launch with a repository from the shell: `src-tauri/target/debug/bundle/macos/Dagscape.app/Contents/MacOS/dagscape /path/to/repo`.

## Release

Same scheme as `ytyng/arcvault`: `.github/workflows/release.yml` runs on every push to `main`,
reads the version from `src-tauri/tauri.conf.json`, and releases only if `v<version>` is not a
published Release yet (test → build matrix macOS universal dmg / Windows NSIS → publish). macOS
builds need the `APPLE_*` repository secrets (Developer ID certificate + notarization). The
Homebrew cask lives in `cyberneura/homebrew-tap` and updates itself from the latest Release.

## Third-party licenses

`scripts/generate-third-party-notices.sh` (`pnpm notices`) writes THIRD-PARTY-NOTICES.txt from
what actually ships: the client bundle's modules (vite.config.js `listBundledModules`, enabled by
the script), cargo-about for the release targets (`src-tauri/about.toml`), and native libraries
shipped by `-sys` crates (vendored libgit2; zlib and the WebView2 loader on Windows). Texts that a
package does not ship live in `vendor-licenses/<name>@<version>/`, copied from that version's
upstream. **Re-run it and commit the result whenever a dependency is added or upgraded, or the
frontend starts importing a package it did not bundle before** (e.g. a plugin used only from Rust); the
tests in `notices_tests.rs` and the CI `--check` fail otherwise. A new `-sys` crate, an unknown
virtual module, or a version bump of a vendored package stops the script until it is checked.

## App icon

Masters are in `resources/app-icons/` (`dagscape-favicon.png` full-bleed, `dagscape-mac-icon.png`
with a 10% margin). `pnpm tauri icon` writes `.icns` smallest-first, which 1Password's dialogs show
blurred, so rebuild it largest-first afterwards and drop the mobile icons:

```bash
pnpm tauri icon "$PWD/resources/app-icons/dagscape-favicon.png"
python3 ~/workspace/astragal/resources/app-icons/build_icns.py \
  resources/app-icons/dagscape-mac-icon.png src-tauri/icons/icon.icns
rm -rf src-tauri/icons/android src-tauri/icons/ios
```

## Testing

- `src-tauri/src/graph.rs` tests cover the lane algorithm with small synthetic DAGs.
- `src-tauri/src/cli.rs` tests cover argument parsing and the quoting of the install script.
- `src-tauri/src/repo.rs` tests build a real repository with the `git` CLI in a temp directory
  (needs `git` on PATH).
- The UI has no automated tests; verify by launching the app against a real repository.
