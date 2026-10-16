//! THIRD-PARTY-NOTICES.txt が依存の更新に追いついているかのテスト。依存を上げたのに
//! `pnpm notices` を流し忘れると落ちる。

const NOTICES: &str = include_str!("../../THIRD-PARTY-NOTICES.txt");
const CARGO_TOML: &str = include_str!("../Cargo.toml");
const CARGO_LOCK: &str = include_str!("../Cargo.lock");
const PNPM_LOCK: &str = include_str!("../../pnpm-lock.yaml");

/// dependencies のうち bundle に入らないもの (ビルドツールと、Rust 側からしか使わない plugin)
const NPM_NOT_BUNDLED: [&str; 2] = ["@tailwindcss/vite", "@tauri-apps/plugin-opener"];
/// devDependencies のまま bundle に入るもの (svelte / kit の runtime と、vite が差し込む helper)
const NPM_BUNDLED_DEV: [&str; 3] = ["svelte", "@sveltejs/kit", "vite"];

/// Windows の checkout は autocrlf で CRLF になるので、行末を揃えてから読む
fn lf(text: &str) -> String {
    text.replace("\r\n", "\n")
}

/// notices の `section` 節 ("JavaScript" / "Rust crates") の "Used by:" に並ぶ (名前, version)。
/// エントリは区切り線 → `License: ...` → 空行 → "Used by:" の並びなので、その並びだけを読む
/// (同梱された本文に同じ字面があっても数えない)
fn listed(section: &str) -> Vec<(String, String)> {
    // 節の見出しは `#` 80 個の行に挟まれている。`\n# ` だけで切ると、同梱された本文の
    // Markdown の見出し (vite の LICENSE.md 等) でも切れて、その後ろのエントリが検査から漏れる
    let notices = lf(NOTICES);
    let rule = format!("\n{}\n# ", "#".repeat(80));
    let text = notices
        .split(rule.as_str())
        .find(|part| part.starts_with(section))
        .unwrap_or_else(|| panic!("the notices file has a {section} section"));
    let separator = "=".repeat(80);
    let lines: Vec<&str> = text.lines().collect();
    let mut packages = Vec::new();
    for i in 0..lines.len() {
        let is_entry = lines[i] == separator
            && lines.get(i + 1).is_some_and(|l| l.starts_with("License: "))
            && lines.get(i + 2) == Some(&"")
            && lines.get(i + 3) == Some(&"Used by:");
        if !is_entry {
            continue;
        }
        for line in &lines[i + 4..] {
            let mut words = line.strip_prefix("  ").unwrap_or("").split(' ');
            match (words.next(), words.next()) {
                (Some(name), Some(version)) if !name.is_empty() => {
                    packages.push((name.to_string(), version.to_string()));
                }
                _ => break,
            }
        }
    }
    packages
}

/// 配布物に入る直接依存。`[dependencies]` と `[target.*.dependencies]` の 1 行書式だけを見る
fn direct_rust_dependencies() -> Vec<String> {
    let mut section = String::new();
    let mut names = Vec::new();
    for line in lf(CARGO_TOML).lines() {
        let line = line.trim();
        if line.starts_with('[') {
            section = line.to_string();
            continue;
        }
        let shipped = section == "[dependencies]"
            || (section.starts_with("[target.") && section.ends_with(".dependencies]"));
        if !shipped || line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((name, _)) = line.split_once('=') {
            names.push(name.trim().trim_end_matches(".workspace").to_string());
        }
    }
    names
}

/// Cargo.lock の [[package]] の (name, version, dependencies の行)
fn locked_rust_packages() -> Vec<(String, String, Vec<String>)> {
    lf(CARGO_LOCK)
        .split("[[package]]")
        .skip(1)
        .map(|block| {
            let field = |key: &str| {
                block
                    .lines()
                    .find_map(|line| line.strip_prefix(key))
                    .map(|rest| rest.trim().trim_matches('"').to_string())
                    .unwrap_or_default()
            };
            let deps = block
                .lines()
                .filter_map(|line| line.strip_prefix(" \""))
                .map(|line| line.trim_end_matches("\",").to_string())
                .collect();
            (field("name = "), field("version = "), deps)
        })
        .collect()
}

/// Cargo.lock が dagscape の直接依存 `name` に選んだ version。同じ crate が複数の version で
/// 入っている時は、dagscape の dependencies に `name version` の形で書かれる
fn resolved_rust_version(lock: &[(String, String, Vec<String>)], name: &str) -> String {
    let root = lock
        .iter()
        .find(|(crate_name, _, _)| crate_name == "dagscape")
        .expect("Cargo.lock has the dagscape package");
    let entry = root
        .2
        .iter()
        .find(|dep| *dep == name || dep.starts_with(&format!("{name} ")))
        .unwrap_or_else(|| panic!("{name} is not a dependency of dagscape in Cargo.lock"));
    match entry.split(' ').nth(1) {
        Some(version) => version.to_string(),
        None => {
            let versions: Vec<&String> = lock
                .iter()
                .filter(|(crate_name, _, _)| crate_name == name)
                .map(|(_, version, _)| version)
                .collect();
            assert_eq!(versions.len(), 1, "{name} has several versions in Cargo.lock");
            versions[0].clone()
        }
    }
}

/// pnpm-lock.yaml の importers の `.` が `group` (dependencies / devDependencies) に選んだ
/// (name, version)。`name:` → `specifier:` → `version:` の 3 行で並ぶ
fn resolved_npm_versions(group: &str) -> Vec<(String, String)> {
    let lock = lf(PNPM_LOCK);
    let importer = lock
        .split("\nimporters:\n")
        .nth(1)
        .and_then(|rest| rest.split("\npackages:\n").next())
        .expect("pnpm-lock.yaml has importers before packages");
    let header = format!("    {group}:");
    let mut resolved = Vec::new();
    let mut name = String::new();
    let mut in_group = false;
    for line in importer.lines() {
        if line.starts_with("    ") && !line.starts_with("     ") {
            in_group = line == header;
        } else if !in_group {
        } else if let Some(key) = line.strip_prefix("      ").filter(|rest| !rest.starts_with(' ')) {
            name = key.trim_end_matches(':').trim_matches('\'').to_string();
        } else if let Some(version) = line.strip_prefix("        version: ") {
            // peer 依存の括弧は notices の version には無い
            let version = version.split('(').next().unwrap_or(version).trim();
            resolved.push((name.clone(), version.to_string()));
        }
    }
    resolved
}

#[test]
fn notices_list_every_direct_rust_dependency_at_the_locked_version() {
    // Arrange
    let deps = direct_rust_dependencies();
    assert!(deps.contains(&"git2".to_string()), "parsed deps: {deps:?}");
    let lock = locked_rust_packages();
    let listed = listed("Rust crates");
    assert!(listed.len() > 100, "parsed notices: {listed:?}");

    // Act
    let missing: Vec<(String, String)> = deps
        .iter()
        .map(|name| (name.clone(), resolved_rust_version(&lock, name)))
        .filter(|entry| !listed.contains(entry))
        .collect();

    // Assert
    assert!(missing.is_empty(), "not in THIRD-PARTY-NOTICES.txt (run `pnpm notices`): {missing:?}");
}

#[test]
fn notices_list_only_rust_crates_in_cargo_lock_and_not_dagscape() {
    // Arrange
    let lock = locked_rust_packages();
    let listed = listed("Rust crates");

    // Act
    let stale: Vec<&(String, String)> = listed
        .iter()
        .filter(|(name, version)| !lock.iter().any(|(n, v, _)| n == name && v == version))
        .collect();

    // Assert
    assert!(stale.is_empty(), "not in Cargo.lock (run `pnpm notices`): {stale:?}");
    assert!(!listed.iter().any(|(name, _)| name == "dagscape"));
}

#[test]
fn notices_list_every_bundled_npm_package_at_the_locked_version() {
    // Arrange
    let mut expected: Vec<(String, String)> = resolved_npm_versions("dependencies")
        .into_iter()
        .filter(|(name, _)| !NPM_NOT_BUNDLED.contains(&name.as_str()))
        .collect();
    assert!(expected.iter().any(|(name, _)| name == "@tauri-apps/api"), "parsed lock: {expected:?}");
    let dev = resolved_npm_versions("devDependencies");
    for name in NPM_BUNDLED_DEV {
        let entry = dev
            .iter()
            .find(|(dev_name, _)| dev_name == name)
            .unwrap_or_else(|| panic!("{name} is not a devDependency in pnpm-lock.yaml"));
        expected.push(entry.clone());
    }
    let listed = listed("JavaScript");

    // Act
    let missing: Vec<&(String, String)> = expected.iter().filter(|entry| !listed.contains(entry)).collect();

    // Assert
    assert!(missing.is_empty(), "not in THIRD-PARTY-NOTICES.txt (run `pnpm notices`): {missing:?}");
}

#[test]
fn notices_list_only_npm_packages_in_pnpm_lock() {
    // Arrange: packages 節のキーは `name@version:` (scope 付きは引用符で囲まれる)
    let lock = lf(PNPM_LOCK);
    let listed = listed("JavaScript");
    assert!(listed.len() >= 5, "parsed notices: {listed:?}");

    // Act
    let stale: Vec<&(String, String)> = listed
        .iter()
        .filter(|(name, version)| {
            let key = format!("{name}@{version}");
            !lock.contains(&format!("\n  {key}:")) && !lock.contains(&format!("\n  '{key}':"))
        })
        .collect();

    // Assert
    assert!(stale.is_empty(), "not in pnpm-lock.yaml (run `pnpm notices`): {stale:?}");
}

#[test]
fn notices_include_the_vendored_libgit2_license() {
    assert!(lf(NOTICES).contains("  libgit2 (bundled by libgit2-sys "));
}
