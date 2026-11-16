//! 用系统浏览器打开外部说明链接（教程里的 mathlib 维基百科链接）。
//!
//! 只放行白名单里的 https 主机，不读写工作区。

use std::process::Command;

/// 允许打开的主机：只有维基百科。
const ALLOWED_HOSTS: &[&str] = &["en.wikipedia.org", "zh.wikipedia.org"];

/// 校验链接：必须是 https、主机在白名单里、没有用户名密码。
pub fn checked_external_url(raw: &str) -> Result<url::Url, String> {
    let url = url::Url::parse(raw).map_err(|error| format!("invalid_url：{error}"))?;
    let host_ok = url
        .host_str()
        .is_some_and(|host| ALLOWED_HOSTS.contains(&host));
    if url.scheme() != "https" || !host_ok || !url.username().is_empty() || url.password().is_some()
    {
        return Err(format!("external_link_not_allowed：{raw}"));
    }
    Ok(url)
}

fn opener(url: &str) -> Command {
    if cfg!(target_os = "macos") {
        let mut command = Command::new("open");
        command.arg(url);
        command
    } else if cfg!(target_os = "windows") {
        let mut command = Command::new("rundll32");
        command.args(["url.dll,FileProtocolHandler", url]);
        command
    } else {
        let mut command = Command::new("xdg-open");
        command.arg(url);
        command
    }
}

#[tauri::command]
pub fn open_external_link(url: String) -> Result<(), String> {
    let url = checked_external_url(&url)?;
    opener(url.as_str())
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("open_failed：{error}"))
}

#[cfg(test)]
mod tests {
    use super::checked_external_url;

    #[test]
    fn only_https_wikipedia_links_open() {
        assert!(
            checked_external_url("https://en.wikipedia.org/wiki/Lean_(proof_assistant)#Libraries")
                .is_ok()
        );
        for bad in [
            "http://en.wikipedia.org/wiki/Lean",
            "https://example.com/",
            "file:///etc/passwd",
            "https://user:pw@en.wikipedia.org/",
            "https://en.wikipedia.org.evil.com/",
            "not a url",
        ] {
            assert!(checked_external_url(bad).is_err(), "{bad}");
        }
    }
}
