mod cli;
mod github;
mod graph;
#[cfg(test)]
mod notices_tests;
mod repo;
mod watcher;

use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Instant;

use serde::Serialize;
use tauri::menu::{Menu, MenuItem, Submenu};
use tauri::{AppHandle, Emitter, Manager, Runtime, State, WebviewUrl, WebviewWindowBuilder};

use repo::{CommitDetail, FileDiff, Graph, HeadInfo, RefInfo, RemoteInfo, Row};

struct Session {
    repo: git2::Repository,
    graph: Graph,
    include_remotes: bool,
    /// drop で監視が止まるので持っているだけ
    _watcher: notify::RecommendedWatcher,
}

#[derive(Default)]
struct AppState {
    session: Mutex<Option<Session>>,
    /// コマンドライン引数で渡されたパス。フロントが起動時に 1 回だけ取りに来る
    initial_path: Mutex<Option<String>>,
    /// open_repo の受付番号。大きいリポジトリ A の読込中に B を開き、A が後から終わっても
    /// A で上書きしないよう、受付順で新しい方だけを採用する
    open_ticket: AtomicU64,
    installed_ticket: AtomicU64,
}

#[derive(Serialize)]
struct RepoSummary {
    path: String,
    name: String,
    total: usize,
    max_lanes: usize,
    refs: Vec<RefInfo>,
    head: Option<HeadInfo>,
    remotes: Vec<RemoteInfo>,
    include_remotes: bool,
    load_ms: u128,
}

fn summarize(repo: &git2::Repository, graph: &Graph, include_remotes: bool, started: Instant) -> RepoSummary {
    let path = repo::display_path(repo);
    RepoSummary {
        name: path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.to_string_lossy().into_owned()),
        path: path.to_string_lossy().into_owned(),
        total: graph.rows.len(),
        max_lanes: graph.max_lanes,
        refs: graph.refs.clone(),
        head: graph.head.clone(),
        remotes: graph.remotes.clone(),
        include_remotes,
        load_ms: started.elapsed().as_millis(),
    }
}

fn lock_session(state: &AppState) -> std::sync::MutexGuard<'_, Option<Session>> {
    // 中で panic することは無いので poison は無視して続ける
    state.session.lock().unwrap_or_else(|e| e.into_inner())
}

#[tauri::command]
async fn open_repo(
    app: AppHandle,
    state: State<'_, AppState>,
    path: String,
    include_remotes: bool,
) -> Result<RepoSummary, String> {
    let started = Instant::now();
    let ticket = state.open_ticket.fetch_add(1, Ordering::SeqCst) + 1;
    let repository = repo::open(Path::new(&path))?;
    let graph = repo::load_graph(&repository, include_remotes)?;
    let watcher = watcher::watch(&repository, app)?;
    let summary = summarize(&repository, &graph, include_remotes, started);
    let mut guard = lock_session(&state);
    if ticket < state.installed_ticket.load(Ordering::SeqCst) {
        return Err("Superseded by a newer open request".to_string());
    }
    state.installed_ticket.store(ticket, Ordering::SeqCst);
    *guard = Some(Session {
        repo: repository,
        graph,
        include_remotes,
        _watcher: watcher,
    });
    Ok(summary)
}

#[tauri::command]
async fn reload_repo(
    state: State<'_, AppState>,
    include_remotes: Option<bool>,
) -> Result<RepoSummary, String> {
    let started = Instant::now();
    let mut guard = lock_session(&state);
    let session = guard.as_mut().ok_or("No repository is open")?;
    let include_remotes = include_remotes.unwrap_or(session.include_remotes);
    session.graph = repo::load_graph(&session.repo, include_remotes)?;
    session.include_remotes = include_remotes;
    Ok(summarize(&session.repo, &session.graph, include_remotes, started))
}

#[tauri::command]
async fn close_repo(state: State<'_, AppState>) -> Result<(), String> {
    *lock_session(&state) = None;
    Ok(())
}

#[tauri::command]
async fn get_rows(state: State<'_, AppState>, start: usize, end: usize) -> Result<Vec<Row>, String> {
    let guard = lock_session(&state);
    let session = guard.as_ref().ok_or("No repository is open")?;
    let rows = &session.graph.rows;
    let end = end.min(rows.len());
    let start = start.min(end);
    Ok(rows[start..end].to_vec())
}

#[tauri::command]
async fn get_commit(state: State<'_, AppState>, oid: String) -> Result<CommitDetail, String> {
    let guard = lock_session(&state);
    let session = guard.as_ref().ok_or("No repository is open")?;
    repo::commit_detail(&session.repo, &oid)
}

#[tauri::command]
async fn get_file_diff(
    state: State<'_, AppState>,
    oid: String,
    path: String,
    old_path: Option<String>,
) -> Result<FileDiff, String> {
    let guard = lock_session(&state);
    let session = guard.as_ref().ok_or("No repository is open")?;
    repo::file_diff(&session.repo, &oid, &path, old_path.as_deref())
}

#[tauri::command]
async fn search_commits(state: State<'_, AppState>, query: String) -> Result<Vec<usize>, String> {
    let guard = lock_session(&state);
    let session = guard.as_ref().ok_or("No repository is open")?;
    Ok(repo::search(&session.graph, &query))
}

#[tauri::command]
async fn row_index_of(state: State<'_, AppState>, oid: String) -> Result<Option<usize>, String> {
    let guard = lock_session(&state);
    let session = guard.as_ref().ok_or("No repository is open")?;
    Ok(session.graph.index_by_oid.get(&oid).copied())
}

#[tauri::command]
fn initial_path(state: State<'_, AppState>) -> Option<String> {
    state
        .initial_path
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .take()
}

#[tauri::command]
fn third_party_notices() -> String {
    cli::license_text()
}

/// メニュー項目の id と、Third-Party Licenses のウインドウの label
const LICENSES_ID: &str = "licenses";
#[cfg(target_os = "macos")]
const MENU_INSTALL_CLI: &str = "install-cli";

/// 開いていれば前面に出す。閉じると破棄されるので、開くたびに作り直す
fn show_licenses_window<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    if let Some(window) = app.get_webview_window(LICENSES_ID) {
        window.show()?;
        return window.set_focus();
    }
    WebviewWindowBuilder::new(app, LICENSES_ID, WebviewUrl::App("licenses".into()))
        .title("Third-Party Licenses")
        .inner_size(720.0, 600.0)
        .min_inner_size(400.0, 300.0)
        .center()
        .build()?;
    Ok(())
}

/// 管理者のパスワードを尋ねることがあるので、メニューのイベントを処理するスレッドを塞がない
#[cfg(target_os = "macos")]
fn install_cli<R: Runtime>(app: AppHandle<R>) {
    use tauri_plugin_dialog::{DialogExt, MessageDialogKind};
    std::thread::spawn(move || {
        let (kind, title, message) = match cli::install() {
            Ok(installed) => (MessageDialogKind::Info, "Installed the dagscape command", installed),
            Err(e) => (MessageDialogKind::Error, "Failed to install the dagscape command", e),
        };
        app.dialog().message(message).title(title).kind(kind).show(|_| {});
    });
}

const MENU_OPEN: &str = "open-repo";
const MENU_RELOAD: &str = "reload-repo";
const MENU_FIND: &str = "find";

/// 既定メニューの File に Open / Reload / Find を足す。File が無いプラットフォームでは先頭に File を作る。
/// Third-Party Licenses は macOS ではアプリメニューの About の直下、ほかでは Help の About の直下。
fn build_menu<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<Menu<R>> {
    let open = MenuItem::with_id(app, MENU_OPEN, "Open Repository…", true, Some("CmdOrCtrl+O"))?;
    let reload = MenuItem::with_id(app, MENU_RELOAD, "Reload", true, Some("CmdOrCtrl+R"))?;
    let find = MenuItem::with_id(app, MENU_FIND, "Find…", true, Some("CmdOrCtrl+F"))?;

    let menu = Menu::default(app)?;
    let file = menu
        .items()?
        .into_iter()
        .filter_map(|item| item.as_submenu().cloned())
        .find(|submenu| submenu.text().map(|t| t == "File").unwrap_or(false));
    match file {
        Some(file) => {
            file.insert(&open, 0)?;
            file.insert(&reload, 1)?;
            file.insert(&find, 2)?;
        }
        None => {
            let file = Submenu::with_items(app, "File", true, &[&open, &reload, &find])?;
            menu.insert(&file, 0)?;
        }
    }

    let licenses = MenuItem::with_id(app, LICENSES_ID, "Third-Party Licenses", true, None::<&str>)?;
    #[cfg(target_os = "macos")]
    {
        use tauri::menu::PredefinedMenuItem;
        let install = MenuItem::with_id(
            app,
            MENU_INSTALL_CLI,
            "Install Command Line Tools…",
            true,
            None::<&str>,
        )?;
        // Menu::default はアプリメニューを先頭に置き、その 0 番目が About
        if let Some(app_menu) = menu.items()?.first().and_then(|item| item.as_submenu().cloned()) {
            app_menu.insert(&licenses, 1)?;
            app_menu.insert(&PredefinedMenuItem::separator(app)?, 2)?;
            app_menu.insert(&install, 3)?;
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        match menu.get(tauri::menu::HELP_SUBMENU_ID).and_then(|item| item.as_submenu().cloned()) {
            Some(help) => help.append(&licenses)?,
            None => menu.append(&Submenu::with_items(app, "Help", true, &[&licenses])?)?,
        }
    }
    Ok(menu)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    if cli::invoked_via_symlink() {
        std::process::exit(cli::run_client());
    }
    if let Some(code) = cli::print_license_if_requested() {
        std::process::exit(code);
    }
    // Finder から起動すると -psn_... のような引数が付くことがあるので、ダッシュ始まりは無視する
    let initial = std::env::args().nth(1).filter(|a| !a.starts_with('-'));
    let state = AppState {
        initial_path: Mutex::new(initial),
        ..Default::default()
    };

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_store::Builder::default().build())
        .manage(state)
        .menu(build_menu)
        .on_menu_event(|app, event| {
            let id = event.id().as_ref();
            if matches!(id, MENU_OPEN | MENU_RELOAD | MENU_FIND) {
                let _ = app.emit("menu", id.to_string());
            } else if id == LICENSES_ID {
                // メニューのハンドラーからウインドウを作ると Windows でデッドロックするので別スレッドで
                let app = app.clone();
                std::thread::spawn(move || {
                    if let Err(e) = show_licenses_window(&app) {
                        eprintln!("dagscape: failed to show the licenses window: {e}");
                    }
                });
            } else {
                #[cfg(target_os = "macos")]
                if id == MENU_INSTALL_CLI {
                    install_cli(app.clone());
                }
            }
        })
        // licenses が残るとアプリが終了しないので、main と一緒に閉じる
        .on_window_event(|window, event| {
            if window.label() == "main" && matches!(event, tauri::WindowEvent::Destroyed) {
                if let Some(licenses) = window.app_handle().get_webview_window(LICENSES_ID) {
                    let _ = licenses.close();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            open_repo,
            reload_repo,
            close_repo,
            get_rows,
            get_commit,
            get_file_diff,
            search_commits,
            row_index_of,
            initial_path,
            third_party_notices,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
