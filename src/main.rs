//! APC mini mk2 を herdr の物理コンソールにするデーモン。
//!
//! 現段階は読み取り専用のダッシュボード。状態ランプの行だけを描く。
//! 縦列 = プロジェクト（workspace）、横行 = アクション。

mod apc;
mod herdr;

use apc::{color, Behavior, Surface};
use herdr::{Client, Status};
use serde_json::Value;
use std::collections::HashMap;

/// 状態ランプを置く行。フェーダーの真上に来るよう最下段に置く。
const ROW_STATUS: u8 = 7;
/// 一度に載るプロジェクト数。
const COLUMNS: usize = 8;

fn paint(status: Status) -> (u8, Behavior) {
    match status {
        Status::Blocked => (color::RED, Behavior::Blink),
        Status::Working => (color::ORANGE, Behavior::Pulse),
        Status::Done => (color::GREEN, Behavior::Pulse),
        Status::Idle => (color::GREEN, Behavior::Dim),
        Status::Unknown => (color::GREY, Behavior::Dim),
    }
}

struct Fleet {
    /// 列に割り当てたワークスペース。穴は None。
    columns: Vec<Option<String>>,
    /// pane_id -> (workspace_id, status)。エージェントのいるペインだけ。
    panes: HashMap<String, (String, Status)>,
}

impl Fleet {
    fn status_of(&self, workspace_id: &str) -> Option<Status> {
        let mut acc: Option<Status> = None;
        for (ws, st) in self.panes.values() {
            if ws == workspace_id {
                acc = Some(match acc {
                    Some(prev) => prev.max(*st),
                    None => *st,
                });
            }
        }
        acc
    }

    /// pane_updated を取り込む。描き直しが要るかを返す。
    fn apply_pane(&mut self, pane: &Value) -> bool {
        let Some(pane_id) = pane.get("pane_id").and_then(|v| v.as_str()) else {
            return false;
        };
        let has_agent = pane.get("agent").map(|v| !v.is_null()).unwrap_or(false);
        if !has_agent {
            return self.panes.remove(pane_id).is_some();
        }
        let ws = pane
            .get("workspace_id")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        let status = Status::parse(
            pane.get("agent_status").and_then(|v| v.as_str()).unwrap_or("unknown"),
        );
        let next = (ws, status);
        match self.panes.get(pane_id) {
            Some(prev) if *prev == next => false,
            _ => {
                self.panes.insert(pane_id.to_string(), next);
                true
            }
        }
    }

    fn render(&self, surface: &mut Surface) {
        for col in 0..COLUMNS {
            let note = apc::pad_note(ROW_STATUS, col as u8);
            match self.columns.get(col).and_then(|c| c.as_ref()) {
                Some(ws) => match self.status_of(ws) {
                    Some(status) => {
                        let (c, b) = paint(status);
                        surface.set(note, c, b);
                    }
                    // ワークスペースはあるがエージェントがいない
                    None => surface.set(note, color::GREY, Behavior::Dim),
                },
                None => surface.off(note),
            }
        }
    }
}

fn load_columns() -> Result<Vec<Option<String>>, Box<dyn std::error::Error>> {
    let result = herdr::request("workspace.list", serde_json::json!({}))?;
    let mut list: Vec<(u64, String, String)> = result["workspaces"]
        .as_array()
        .map(|a| a.as_slice())
        .unwrap_or(&[])
        .iter()
        .filter_map(|w| {
            Some((
                w.get("number")?.as_u64()?,
                w.get("workspace_id")?.as_str()?.to_string(),
                w.get("label").and_then(|v| v.as_str()).unwrap_or("").to_string(),
            ))
        })
        .collect();
    list.sort_by_key(|(n, _, _)| *n);

    let mut columns = vec![None; COLUMNS];
    for (i, (_, id, label)) in list.iter().take(COLUMNS).enumerate() {
        eprintln!("col {i}: {id} {label}");
        columns[i] = Some(id.clone());
    }
    if list.len() > COLUMNS {
        eprintln!(
            "note: workspace が {} 個あります。先頭 {} 個だけ載せます（バンク切替は未実装）",
            list.len(),
            COLUMNS
        );
    }
    Ok(columns)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut surface = Surface::open()?;
    surface.clear_all();
    eprintln!("APC mini mk2 に接続しました");

    let columns = load_columns()?;
    let mut fleet = Fleet { columns, panes: HashMap::new() };
    let mut client = Client::connect()?;

    // pane.updated は購読直後に全ペインの現在状態をリプレイしてくれる。
    // 初期化のための snapshot 取得は要らない。
    client.subscribe(&["pane.updated", "pane.closed", "workspace.created", "workspace.closed"])?;
    eprintln!("herdr のイベントを購読しました");

    loop {
        let Some(msg) = client.next()? else {
            eprintln!("herdr との接続が閉じました");
            break;
        };
        let Some(kind) = msg.get("event").and_then(|v| v.as_str()) else {
            continue;
        };
        let data = &msg["data"];
        let dirty = match kind {
            "pane_updated" => fleet.apply_pane(&data["pane"]),
            "pane_closed" => data
                .get("pane_id")
                .and_then(|v| v.as_str())
                .map(|id| fleet.panes.remove(id).is_some())
                .unwrap_or(false),
            // 列の顔ぶれが変わったら組み直す
            "workspace_created" | "workspace_closed" => {
                fleet.columns = load_columns()?;
                true
            }
            _ => false,
        };
        if dirty {
            fleet.render(&mut surface);
        }
    }
    Ok(())
}
