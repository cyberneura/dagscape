//! `dagscape` コマンド (メニューの Install Command Line Tools… が置く /usr/local/bin の symlink)。
//! コマンドを置くのは macOS だけで、ほかのプラットフォームで使うのは --license の表示だけ。
#![cfg_attr(not(target_os = "macos"), allow(dead_code))]

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

pub const CLI_LINK_PATH: &str = "/usr/local/bin/dagscape";

const USAGE: &str = "\
Usage: dagscape [PATH]

Opens the Git repository that contains PATH (default: the current directory)
in a new Dagscape window.

Options:
      --license   Show the licenses of Dagscape and the libraries it bundles
  -v, --version   Show the version
  -h, --help      Show this help";

const LICENSE: &str = include_str!("../../LICENSE");
const THIRD_PARTY_NOTICES: &str = include_str!("../../THIRD-PARTY-NOTICES.txt");

/// アプリ自身の MIT と、同梱しているライブラリのライセンス。.app には LICENSE ファイルが
/// 入らないので、アプリ自身の分も埋め込んで先頭に出す
pub fn license_text() -> String {
    format!(
        "Dagscape is licensed under the MIT License:\n\n{}\n\n{}",
        LICENSE.trim_end(),
        THIRD_PARTY_NOTICES
    )
}

#[derive(Debug, PartialEq)]
enum Action {
    Help,
    Version,
    License,
    Open(Option<String>),
}

// args は argv[0] を含まない
fn parse_args<I: IntoIterator<Item = String>>(args: I) -> Result<Action, String> {
    let mut path = None;
    let mut options_ended = false;
    for arg in args {
        match arg.as_str() {
            _ if options_ended => {}
            "-h" | "--help" => return Ok(Action::Help),
            "-v" | "--version" => return Ok(Action::Version),
            "--license" => return Ok(Action::License),
            "--" => {
                options_ended = true;
                continue;
            }
            _ if arg.starts_with('-') && arg.len() > 1 => return Err(format!("Unknown option: {arg}")),
            _ => {}
        }
        if path.is_some() {
            return Err(format!("Unexpected argument: {arg}"));
        }
        path = Some(arg);
    }
    Ok(Action::Open(path))
}

/// `--license` だけで起動された時は本文を出し、終了コードを返す。GUI として起動された時
/// (Finder からの起動や、パスを付けた起動) は None
pub fn print_license_if_requested() -> Option<i32> {
    let mut args = std::env::args().skip(1);
    match (args.next().as_deref(), args.next()) {
        (Some("--license"), None) => Some(write_stdout(&license_text())),
        _ => None,
    }
}

// print! は書き込みに失敗すると panic するので、`| head` で閉じられた時 (BrokenPipe) を
// 成功として扱えるよう結果を見る
fn write_stdout(text: &str) -> i32 {
    let mut stdout = std::io::stdout().lock();
    match stdout.write_all(text.as_bytes()).and_then(|()| stdout.flush()) {
        Ok(()) => 0,
        Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => 0,
        Err(e) => {
            eprintln!("dagscape: failed to write to standard output: {e}");
            1
        }
    }
}

/// /usr/local/bin の symlink のように、symlink 経由で実行された時はコマンドラインから使われている。
/// .app の実行ファイルをそのまま動かすと GUI がターミナルにぶら下がる (閉じると落ちる) ので、
/// その時は LaunchServices (open) に起動させる
pub fn invoked_via_symlink() -> bool {
    std::env::current_exe()
        .and_then(|path| path.symlink_metadata())
        .map(|metadata| metadata.file_type().is_symlink())
        .unwrap_or(false)
}

/// 終了コードを返す
pub fn run_client() -> i32 {
    match parse_args(std::env::args().skip(1)) {
        Ok(Action::Help) => write_stdout(&format!("{USAGE}\n")),
        Ok(Action::Version) => write_stdout(&format!("dagscape {}\n", env!("CARGO_PKG_VERSION"))),
        Ok(Action::License) => write_stdout(&license_text()),
        Ok(Action::Open(path)) => match resolve_path(path.as_deref()).and_then(|p| launch_app(&p)) {
            Ok(()) => 0,
            Err(e) => {
                eprintln!("dagscape: {e}");
                1
            }
        },
        Err(e) => {
            eprintln!("dagscape: {e}\n\n{USAGE}");
            2
        }
    }
}

// 起動されるアプリは cwd が / になるので、呼び出し元で絶対パスにしてから渡す
fn resolve_path(path: Option<&str>) -> Result<PathBuf, String> {
    let path = path.unwrap_or(".");
    std::fs::canonicalize(path).map_err(|e| format!("{path}: {e}"))
}

fn launch_app(path: &Path) -> Result<(), String> {
    let executable = std::env::current_exe()
        .and_then(|path| path.canonicalize())
        .map_err(|e| format!("Failed to locate the application: {e}"))?;

    let Some(bundle) = app_bundle(&executable) else {
        // 開発中のビルド (.app の外)
        Command::new(&executable)
            .arg(path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("Failed to start {}: {e}", executable.display()))?;
        return Ok(());
    };

    // 1 つのプロセスが開けるリポジトリは 1 つなので、起動済みでも -n で新しいプロセスにする
    // (-n が無いと、open は起動済みのアプリを前面に出すだけで引数を渡さない)
    let status = Command::new("/usr/bin/open")
        .arg("-n")
        .arg(bundle)
        .arg("--args")
        .arg(path)
        .status()
        .map_err(|e| format!("Failed to run open: {e}"))?;
    if !status.success() {
        return Err(format!("open exited with {status}"));
    }
    Ok(())
}

// <名前>.app/Contents/MacOS/<実行ファイル> の <名前>.app
fn app_bundle(executable: &Path) -> Option<PathBuf> {
    let macos = executable.parent()?;
    let contents = macos.parent()?;
    let bundle = contents.parent()?;
    let is_bundle = macos.file_name()? == "MacOS"
        && contents.file_name()? == "Contents"
        && bundle.extension()? == "app";
    is_bundle.then(|| bundle.to_path_buf())
}

/// メニューの Install Command Line Tools…。/usr/local/bin は root の持ち物のことが多いので、
/// 書けなければ管理者のパスワードを尋ねて作る
pub fn install() -> Result<String, String> {
    let executable = std::env::current_exe()
        .and_then(|path| path.canonicalize())
        .map_err(|e| format!("Failed to locate the application: {e}"))?;
    let link = Path::new(CLI_LINK_PATH);
    // 別のものを上書きしない (symlink なら、以前に作ったものとして張り替える)
    if let Ok(metadata) = link.symlink_metadata() {
        if !metadata.file_type().is_symlink() {
            return Err(format!("{CLI_LINK_PATH} already exists and is not a symlink."));
        }
    }

    let script = install_script(&executable);
    let as_user = Command::new("/bin/sh")
        .arg("-c")
        .arg(&script)
        .stderr(Stdio::null())
        .status()
        .map_err(|e| format!("Failed to run sh: {e}"))?;
    if !as_user.success() {
        let output = Command::new("/usr/bin/osascript")
            .arg("-e")
            .arg(format!(
                "do shell script \"{}\" with administrator privileges",
                applescript_escape(&script)
            ))
            .output()
            .map_err(|e| format!("Failed to run osascript: {e}"))?;
        if !output.status.success() {
            return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
        }
    }
    Ok(format!(
        "{CLI_LINK_PATH} -> {}\n\nRun `dagscape` in a repository to open it.",
        executable.display()
    ))
}

fn install_script(executable: &Path) -> String {
    let directory = Path::new(CLI_LINK_PATH).parent().unwrap().to_string_lossy().to_string();
    format!(
        "mkdir -p {} && ln -sfn {} {}",
        sh_quote(&directory),
        sh_quote(&executable.to_string_lossy()),
        sh_quote(CLI_LINK_PATH)
    )
}

fn sh_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn applescript_escape(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|v| v.to_string()).collect()
    }

    #[test]
    fn parse_args_opens_the_current_directory_by_default() {
        assert_eq!(parse_args(args(&[])), Ok(Action::Open(None)));
        assert_eq!(parse_args(args(&["../repo"])), Ok(Action::Open(Some("../repo".into()))));
        assert_eq!(parse_args(args(&["--", "-repo"])), Ok(Action::Open(Some("-repo".into()))));
    }

    #[test]
    fn parse_args_handles_options_and_rejects_extra_arguments() {
        assert_eq!(parse_args(args(&["--license"])), Ok(Action::License));
        assert_eq!(parse_args(args(&["-h"])), Ok(Action::Help));
        assert_eq!(parse_args(args(&["--version"])), Ok(Action::Version));
        assert!(parse_args(args(&["--unknown"])).is_err());
        assert!(parse_args(args(&["a", "b"])).is_err());
    }

    #[test]
    fn resolve_path_makes_it_absolute() {
        let resolved = resolve_path(None).unwrap();
        assert!(resolved.is_absolute());
        assert_eq!(resolved, std::env::current_dir().unwrap().canonicalize().unwrap());
        assert!(resolve_path(Some("/dagscape-missing-directory")).is_err());
    }

    #[test]
    fn app_bundle_finds_the_enclosing_app() {
        assert_eq!(
            app_bundle(Path::new("/Applications/Dagscape.app/Contents/MacOS/dagscape")),
            Some(PathBuf::from("/Applications/Dagscape.app"))
        );
        assert_eq!(app_bundle(Path::new("/repo/src-tauri/target/debug/dagscape")), None);
    }

    #[test]
    fn install_script_quotes_paths_for_applescript_and_sh() {
        // Arrange: パスに空白と引用符がある
        let executable = Path::new("/Applications/My \"Git\" Tools's/Dagscape.app/Contents/MacOS/dagscape");

        // Act
        let script = install_script(executable);
        let escaped = applescript_escape(&script);

        // Assert: AppleScript の文字列の中で、" と \ がそのまま終端や逃がしとして読まれない
        assert_eq!(
            script,
            "mkdir -p '/usr/local/bin' && ln -sfn '/Applications/My \"Git\" Tools'\\''s/Dagscape.app/Contents/MacOS/dagscape' '/usr/local/bin/dagscape'"
        );
        assert!(!escaped.replace("\\\\", "").replace("\\\"", "").contains(['"', '\\']));
    }

    #[test]
    fn license_text_starts_with_dagscape_own_license() {
        // .gitattributes で LF に固定してあるが、手元の checkout 設定によらず読めるようにする
        let text = license_text().replace("\r\n", "\n");
        assert!(text.starts_with("Dagscape is licensed under the MIT License:\n\nMIT License"));
        assert!(text.contains("\nTHIRD-PARTY NOTICES\n"));
    }
}
