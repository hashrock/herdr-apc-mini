//! soft key に割り当てる動詞と、その「実行の余地」。
//!
//! 余地 = **理由がある**（git の状態）かつ **今すぐ実行できる**（agent が idle/done）。
//! 理由の判定だけがここ。実行可否は agent の状態を持っている側で合わせる。

use std::collections::HashMap;

use crate::git;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Verb {
    Simplify,
    SyncRemote,
    /// PR を作る／既にあればコメントを取りにいく。押した先の状態で文言が変わる。
    Pr,
}

/// 右列の上から順。空きは `None`。
pub const SOFT_KEYS: [Option<Verb>; 8] = [
    Some(Verb::Simplify),
    Some(Verb::SyncRemote),
    Some(Verb::Pr),
    None,
    None,
    None,
    None,
    None,
];

impl Verb {
    /// agent に投入する文言。
    ///
    /// PR は「まだ無いなら作る、もうあるなら見にいく」で文言が変わるので、
    /// そのワークスペースの理由を見て決める。
    pub fn prompt(self, reasons: &Reasons) -> &'static str {
        match self {
            Verb::Simplify => "/simplify",
            Verb::SyncRemote => "リモートと同期して",
            Verb::Pr if reasons.has_pr => "PRコメントを取得",
            Verb::Pr => "PR作成",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Verb::Simplify => "simplify",
            Verb::SyncRemote => "sync remote",
            Verb::Pr => "PR",
        }
    }
}

/// 下段ボタンの割り当て。左から順、空きは `None`。
///
/// soft key と違って宛先を選ばない。**フォーカス中の agent** にそのまま送る、
/// 会話の相づちのようなもの。`(ログに出す名前, 送る文言)`。
pub const TRACK_KEYS: [Option<(&str, &str)>; 8] = [
    Some(("ok", "OK")),
    Some(("推奨案", "推奨案で進めて")),
    Some(("かみくだく", "中学生にわかるように解説")),
    Some(("長い", "長い")),
    None,
    None,
    None,
    None,
];

/// そのワークスペースに、どの動詞の理由があるか。
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct Reasons {
    pub simplify: bool,
    pub sync_remote: bool,
    /// GitHub のリポジトリか。PR ボタンが意味を持つ条件。
    pub github: bool,
    /// 今のブランチに PR があるか。PR ボタンの文言がこれで変わる。
    pub has_pr: bool,
}

impl Reasons {
    pub fn has(&self, verb: Verb) -> bool {
        match verb {
            Verb::Simplify => self.simplify,
            Verb::SyncRemote => self.sync_remote,
            Verb::Pr => self.github,
        }
    }
}

/// simplify を済ませた時点の HEAD。`workspace_id -> sha`。
pub type Simplified = HashMap<String, String>;

pub fn load_simplified() -> Simplified {
    let path = crate::padmap::config_dir().join("simplify.json");
    let Ok(text) = std::fs::read_to_string(path) else {
        return Simplified::new();
    };
    serde_json::from_str::<HashMap<String, serde_json::Value>>(&text)
        .map(|m| {
            m.into_iter()
                .filter_map(|(k, v)| Some((k, v.get("head")?.as_str()?.to_string())))
                .collect()
        })
        .unwrap_or_default()
}

pub fn save_simplified(ws: &str, head: &str) {
    let dir = crate::padmap::config_dir();
    let path = dir.join("simplify.json");
    let mut all: HashMap<String, serde_json::Value> = std::fs::read_to_string(&path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default();
    all.insert(
        ws.to_string(),
        serde_json::json!({
            "head": head,
            "at": std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0),
        }),
    );
    if std::fs::create_dir_all(&dir).is_ok() {
        if let Ok(body) = serde_json::to_string_pretty(&all) {
            let _ = std::fs::write(path, format!("{body}\n"));
        }
    }
}

/// simplify を投入して、まだ終わっていないワークスペース。
///
/// デーモンが落ちても記録待ちを失わないよう、メモリだけでなくファイルにも置く。
pub fn load_pending() -> Vec<String> {
    let path = crate::padmap::config_dir().join("pending.json");
    std::fs::read_to_string(path)
        .ok()
        .and_then(|t| serde_json::from_str::<Vec<String>>(&t).ok())
        .unwrap_or_default()
}

pub fn save_pending(pending: &[String]) {
    let dir = crate::padmap::config_dir();
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    if let Ok(body) = serde_json::to_string(pending) {
        let _ = std::fs::write(dir.join("pending.json"), format!("{body}\n"));
    }
}

/// 各ワークスペースの理由を git から作る。`cwds` は workspace_id -> 作業ディレクトリ。
pub fn scan(cwds: &HashMap<String, String>, simplified: &Simplified) -> HashMap<String, Reasons> {
    let mut out = HashMap::new();
    for (ws, cwd) in cwds {
        let Some(head) = git::head(cwd) else { continue };
        let github = git::has_github_remote(cwd);
        out.insert(
            ws.clone(),
            Reasons {
                // 記録が無い（未実施）か、記録した時点から HEAD が進んでいる（陳腐化）
                simplify: simplified.get(ws) != Some(&head),
                sync_remote: git::diverged_from_upstream(cwd),
                github,
                // gh はネットワークに出るので、GitHub のリポジトリにだけ聞く。
                has_pr: github && git::has_pr(cwd),
            },
        );
    }
    out
}
