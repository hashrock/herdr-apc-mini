//! パッドとワークスペースの対応。
//!
//! 8x8 を丸ごと状態表示に使い、**左上から詰める**。
//! 並び順は永続化するので、開いている限り枠は動かない。
//! ワークスペースが閉じたらその枠は消し、**以降を前へ寄せる**。死んだ枠が
//! 生きた枠と同じ見た目で盤面に残るほうが困る、という判断。

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

/// 生きているワークスペースだけを、今の並び順のまま左上から詰め直す。
///
/// 閉じたものは枠ごと消え、以降が前へ寄る。まだ載っていないものは末尾へ足す。
/// `preferred` を先に見るので、初回はエージェントのいるものが前に来る。
pub fn assign(
    slots: &mut Vec<Option<String>>,
    preferred: &[String],
    rest: &[String],
) -> bool {
    let live: Vec<&String> = preferred.iter().chain(rest.iter()).collect();
    let mut next: Vec<Option<String>> = Vec::with_capacity(SLOTS);
    // 保存ファイルが壊れていて同じ id が二度出ても、枠を二重に食わせない。
    fn push(next: &mut Vec<Option<String>>, id: &str) {
        if next.iter().flatten().any(|k| k == id) {
            return;
        }
        next.push(Some(id.to_string()));
    }
    // 既に載っているものは今の順番を保つ。閉じたものはここで落ちる。
    for id in slots.iter().flatten() {
        if live.iter().any(|l| *l == id) {
            push(&mut next, id);
        }
    }
    // まだ載っていないものを末尾へ。
    for id in live {
        push(&mut next, id);
    }
    next.truncate(SLOTS);
    next.resize(SLOTS, None);
    let changed = *slots != next;
    *slots = next;
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    fn placed(slots: &[Option<String>]) -> Vec<&str> {
        slots.iter().flatten().map(String::as_str).collect()
    }

    #[test]
    fn 閉じたワークスペースの枠は消えて以降が前へ寄る() {
        let mut slots = vec![None; SLOTS];
        assign(&mut slots, &[], &ids(&["a", "b", "c", "d"]));
        assert_eq!(placed(&slots), ["a", "b", "c", "d"]);

        assert!(assign(&mut slots, &[], &ids(&["a", "b", "d"])));
        assert_eq!(placed(&slots), ["a", "b", "d"]);
    }

    #[test]
    fn 開いたままなら順番は動かない() {
        let mut slots = vec![None; SLOTS];
        assign(&mut slots, &[], &ids(&["a", "b", "c"]));
        // 一覧の順が変わっても、既に載っているものは今の並びを保つ。
        assert!(!assign(&mut slots, &[], &ids(&["c", "b", "a"])));
        assert_eq!(placed(&slots), ["a", "b", "c"]);
    }

    #[test]
    fn 新しいワークスペースは末尾へ足す() {
        let mut slots = vec![None; SLOTS];
        assign(&mut slots, &[], &ids(&["a", "b"]));
        assert!(assign(&mut slots, &ids(&["z"]), &ids(&["a", "b"])));
        assert_eq!(placed(&slots), ["a", "b", "z"]);
    }

    #[test]
    fn 六十四枠を超えたぶんは載らない() {
        let all: Vec<String> = (0..SLOTS + 3).map(|i| format!("w{i}")).collect();
        let mut slots = vec![None; SLOTS];
        assign(&mut slots, &[], &all);
        assert_eq!(slots.len(), SLOTS);
        assert_eq!(placed(&slots).len(), SLOTS);
    }
}
