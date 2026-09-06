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

/// 上流と乖離しているか。上流が無ければ `false`。
pub fn diverged_from_upstream(cwd: &str) -> bool {
    let Some(counts) = run(cwd, &["rev-list", "--left-right", "--count", "@{u}...HEAD"]) else {
        return false; // 上流が無い、git リポジトリでない、など
    };
    counts.split_whitespace().any(|n| n != "0")
}
