//! git の状態を見る。herdr のイベントには現れないので、ここだけポーリングになる。

use std::process::Command;

fn run(cwd: &str, args: &[&str]) -> Option<String> {
    let out = Command::new("git").arg("-C").arg(cwd).args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

pub fn head(cwd: &str) -> Option<String> {
    run(cwd, &["rev-parse", "HEAD"])
}

/// origin が GitHub か。PR という概念が成立するワークスペースかの判定。
pub fn has_github_remote(cwd: &str) -> bool {
    run(cwd, &["remote", "get-url", "origin"])
        .map(|url| url.contains("github.com"))
        .unwrap_or(false)
}

/// 今のブランチに PR があるか。
///
/// gh が無い・認証が無い・PR が無い、のどれでも `false`。区別しても盤面では
/// 同じ（「PR は無い」として PR 作成を投げる）ので、まとめてしまう。
pub fn has_pr(cwd: &str) -> bool {
    Command::new("gh")
        .current_dir(cwd)
        .args(["pr", "view", "--json", "number"])
        .output()
        .map(|out| out.status.success())
        .unwrap_or(false)
}

/// 上流と乖離しているか。上流が無ければ `false`。
pub fn diverged_from_upstream(cwd: &str) -> bool {
    let Some(counts) = run(cwd, &["rev-list", "--left-right", "--count", "@{u}...HEAD"]) else {
        return false; // 上流が無い、git リポジトリでない、など
    };
    counts.split_whitespace().any(|n| n != "0")
}
