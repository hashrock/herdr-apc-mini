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
}

/// 右列の上から順。空きは `None`。
pub const SOFT_KEYS: [Option<Verb>; 8] = [
    Some(Verb::Simplify),
    Some(Verb::SyncRemote),
    None,
    None,
    None,
    None,
    None,
    None,
];

impl Verb {
    /// agent に投入する文言。
    pub fn prompt(self) -> &'static str {
        match self {
            Verb::Simplify => "/simplify",
            Verb::SyncRemote => "リモートと同期して",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Verb::Simplify => "simplify",
            Verb::SyncRemote => "sync remote",
        }
    }
}

/// そのワークスペースに、どの動詞の理由があるか。
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct Reasons {
    pub simplify: bool,
    pub sync_remote: bool,
}

impl Reasons {
    pub fn has(&self, verb: Verb) -> bool {
        match verb {
            Verb::Simplify => self.simplify,
            Verb::SyncRemote => self.sync_remote,
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
        out.insert(
            ws.clone(),
            Reasons {
                // 記録が無い（未実施）か、記録した時点から HEAD が進んでいる（陳腐化）
                simplify: simplified.get(ws) != Some(&head),
                sync_remote: git::diverged_from_upstream(cwd),
            },
        );
    }
    out
}
