//! パッドとワークスペースの対応。
//!
//! 8x8 を丸ごと状態表示に使い、**左上から詰める**。
//! 枠が勝手に動くと筋肉記憶が壊れるので、一度決めた割り当ては永続化する。
//! ワークスペースが消えても**穴を空けたまま**にし、新しいものは空き枠の末尾へ入れる。

use serde_json::{json, Value};
use std::path::PathBuf;

/// 8x8 の全枠。
pub const SLOTS: usize = 64;

pub fn config_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_default();
    PathBuf::from(home).join(".config/herdr/plugins/config/apc-mini")
}

fn path() -> PathBuf {
    config_dir().join("pad-map.json")
}

pub fn load() -> Vec<Option<String>> {
    let mut slots = vec![None; SLOTS];
    let Ok(text) = std::fs::read_to_string(path()) else {
        return slots;
    };
    let Ok(value) = serde_json::from_str::<Value>(&text) else {
        return slots;
    };
    if let Some(list) = value.get("slots").and_then(|v| v.as_array()) {
        for (i, slot) in list.iter().take(SLOTS).enumerate() {
            slots[i] = slot.as_str().map(str::to_string);
        }
    }
    slots
}

pub fn save(slots: &[Option<String>]) {
    let dir = config_dir();
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let body = json!({
        "slots": slots.iter().map(|c| match c {
            Some(id) => json!(id),
            None => Value::Null,
        }).collect::<Vec<_>>()
    });
    let _ = std::fs::write(path(), format!("{body:#}\n"));
}

/// 既存の割り当てを保ったまま、まだ載っていないワークスペースを空き列へ入れる。
///
/// `preferred` を先に詰めるので、初回はエージェントのいるものが前に来る。
/// 二回目以降は保存済みの割り当てが優先され、順序は動かない。
pub fn assign(
    slots: &mut Vec<Option<String>>,
    preferred: &[String],
    rest: &[String],
) -> bool {
    let mut changed = false;
    for id in preferred.iter().chain(rest.iter()) {
        if slots.iter().any(|c| c.as_deref() == Some(id.as_str())) {
            continue;
        }
        let Some(slot) = slots.iter_mut().find(|c| c.is_none()) else {
            break; // 64 枠すべて埋まっている
        };
        *slot = Some(id.clone());
        changed = true;
    }
    changed
}
