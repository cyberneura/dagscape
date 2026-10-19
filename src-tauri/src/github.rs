//! リモートの URL から GitHub の Web URL (https://host/owner/repo) を作る。

/// GitHub でないホスト、owner/repo の形でないパスは None。
pub fn web_url(remote_url: &str) -> Option<String> {
    let (host, path, ssh) = split_host_path(remote_url.trim())?;
    let host = normalize_host(host, ssh)?;
    let path = path.trim_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);
    let mut parts = path.split('/');
    let (owner, repo) = (parts.next()?, parts.next()?);
    if owner.is_empty() || repo.is_empty() || parts.next().is_some() {
        return None;
    }
    Some(format!("https://{host}/{owner}/{repo}"))
}

/// `scheme://[user@]host[:port]/path` と scp 形式の `[user@]host:path` の両方を受ける。
/// 3 つ目は ssh で接続する URL か (ホスト名が ~/.ssh/config の別名であり得るか)
fn split_host_path(url: &str) -> Option<(&str, &str, bool)> {
    if let Some((scheme, rest)) = url.split_once("://") {
        if !matches!(scheme, "https" | "http" | "ssh" | "git" | "git+ssh" | "ssh+git") {
            return None;
        }
        let (authority, path) = rest.split_once('/')?;
        let host = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
        // ssh のポートは Web とは無関係。http(s) のポートは GitHub Enterprise の Web でもそのまま使う
        let host = match scheme {
            "https" | "http" => host,
            _ => host.split_once(':').map_or(host, |(h, _)| h),
        };
        return Some((host, path, matches!(scheme, "ssh" | "git+ssh" | "ssh+git")));
    }
    let (authority, path) = url.split_once(':')?;
    // Windows のドライブレター (C:\...) やローカルパスを除く
    if authority.len() < 2 || authority.contains('/') || authority.contains('\\') {
        return None;
    }
    let host = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    Some((host, path, true))
}

/// ssh では ~/.ssh/config で複数アカウントを使い分ける `github.com-work` / `github-work` のような
/// 別名を github.com とみなす。`github-` 始まりでもドットを含めば実在のホスト (GitHub Enterprise)。
/// それ以外で "github" を含むホストも GitHub Enterprise とみなしてそのまま使う。
fn normalize_host(host: &str, ssh: bool) -> Option<String> {
    let host = host.to_ascii_lowercase();
    let alias = ssh && (host.starts_with("github.com-") || (host.starts_with("github-") && !host.contains('.')));
    if host == "github.com" || host == "ssh.github.com" || alias {
        return Some("github.com".to_string());
    }
    host.contains("github").then_some(host)
}

#[cfg(test)]
mod tests {
    use super::web_url;

    #[test]
    fn accepts_the_usual_remote_forms() {
        let expected = Some("https://github.com/cyberneura/dagscape".to_string());
        for url in [
            "https://github.com/cyberneura/dagscape.git",
            "https://github.com/cyberneura/dagscape",
            "https://github.com/cyberneura/dagscape/",
            "https://user:token@github.com/cyberneura/dagscape.git",
            "git@github.com:cyberneura/dagscape.git",
            "github.com:cyberneura/dagscape",
            "ssh://git@github.com/cyberneura/dagscape.git",
            "ssh://git@ssh.github.com:443/cyberneura/dagscape.git",
            "git://github.com/cyberneura/dagscape.git",
            "git@github.com-work:cyberneura/dagscape.git",
            "git@github-work:cyberneura/dagscape.git",
        ] {
            assert_eq!(web_url(url), expected, "{url}");
        }
    }

    #[test]
    fn keeps_a_github_enterprise_host() {
        for (url, expected) in [
            ("git@github.example.com:team/app.git", "https://github.example.com/team/app"),
            ("https://github-ent.example.com/team/app.git", "https://github-ent.example.com/team/app"),
            ("https://github.example.com:8443/team/app.git", "https://github.example.com:8443/team/app"),
            ("ssh://git@github.example.com:2222/team/app.git", "https://github.example.com/team/app"),
            ("https://github.com-work.example.com/team/app.git", "https://github.com-work.example.com/team/app"),
        ] {
            assert_eq!(web_url(url).as_deref(), Some(expected), "{url}");
        }
    }

    #[test]
    fn rejects_other_hosts_and_local_paths() {
        for url in [
            "https://gitlab.com/group/project.git",
            "git@bitbucket.org:team/repo.git",
            "/srv/git/repo.git",
            "../repo",
            "C:\\repos\\app",
            "file:///srv/git/github/repo.git",
            "https://github.com/only-owner",
            "https://github.com/a/b/c",
        ] {
            assert_eq!(web_url(url), None, "{url}");
        }
    }
}
