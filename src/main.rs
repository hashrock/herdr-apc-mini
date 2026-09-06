//! APC mini mk2 を herdr の物理コンソールにするデーモン。
//!
//! 現段階は読み取り専用のダッシュボード。状態ランプの行だけを描く。
//! 縦列 = プロジェクト（workspace）、横行 = アクション。

mod apc;
mod herdr;
mod padmap;

use apc::{color, Behavior, Surface};
use herdr::{Client, Status};
use serde_json::Value;
use std::collections::HashMap;

/// 状態ランプを置く行。フェーダーの真上に来るよう最下段に置く。
const ROW_STATUS: u8 = 7;
use padmap::COLUMNS;

fn paint(status: Status) -> (u8, Behavior) {
    match status {
        Status::Blocked => (color::RED, Behavior::Blink),
        Status::Working => (color::ORANGE, Behavior::Pulse),
        Status::Done => (color::GREEN, Behavior::Pulse),
        Status::Idle => (color::GREEN, Behavior::Half),
        Status::Unknown => (color::WHITE, Behavior::Dim),
    }
}

struct Fleet {
    /// 列に割り当てたワークスペース。穴は None。
    columns: Vec<Option<String>>,
    /// pane_id -> (workspace_id, status)。エージェントのいるペインだけ。
    panes: HashMap<String, (String, Status)>,
}

impl Fleet {
    /// `agent.list` の内容で状態を作り直す。
    ///
    /// `pane.updated` の購読は直後に全ペインをリプレイしてくれるが、
    /// そこに載る `agent_status` は `unknown` なので、実際の idle / blocked は
    /// この問い合わせでしか分からない。
    fn seed(&mut self, agents: &Value) {
        self.panes.clear();
        for a in agents["agents"].as_array().map(|v| v.as_slice()).unwrap_or(&[]) {
            let (Some(pane_id), Some(ws)) = (
                a.get("pane_id").and_then(|v| v.as_str()),
                a.get("workspace_id").and_then(|v| v.as_str()),
            ) else {
                continue;
            };
            let status = Status::parse(
                a.get("agent_status").and_then(|v| v.as_str()).unwrap_or("unknown"),
            );
            self.panes.insert(pane_id.to_string(), (ws.to_string(), status));
        }
    }

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
        // 購読直後のリプレイは agent_status を unknown で寄越す。
        // 既に確かな状態を持っているペインを、それで塗り潰さない。
        if status == Status::Unknown {
            if let Some((_, prev)) = self.panes.get(pane_id) {
                if *prev != Status::Unknown {
                    return false;
                }
            }
        }
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
                    None => surface.set(note, color::WHITE, Behavior::Dim),
                },
                None => surface.off(note),
            }
        }
    }
}

/// ワークスペースの一覧を取り、ピン留めを保ったまま列へ割り当てる。
/// あわせて `agent.list` の内容を返す。
fn sync_columns(
    columns: &mut Vec<Option<String>>,
) -> Result<Value, Box<dyn std::error::Error>> {
    let result = herdr::request("workspace.list", serde_json::json!({}))?;
    let mut all: Vec<(u64, String, String)> = result["workspaces"]
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
    all.sort_by_key(|(n, _, _)| *n);

    // 初回に前へ来てほしいのはエージェントが居るワークスペース。
    let agents = herdr::request("agent.list", serde_json::json!({}))?;
    let with_agent: Vec<String> = agents["agents"]
        .as_array()
        .map(|a| a.as_slice())
        .unwrap_or(&[])
        .iter()
        .filter_map(|a| a.get("workspace_id")?.as_str().map(str::to_string))
        .collect();

    let preferred: Vec<String> = all
        .iter()
        .filter(|(_, id, _)| with_agent.contains(id))
        .map(|(_, id, _)| id.clone())
        .collect();
    let rest: Vec<String> = all
        .iter()
        .filter(|(_, id, _)| !with_agent.contains(id))
        .map(|(_, id, _)| id.clone())
        .collect();

    if padmap::assign(columns, &preferred, &rest) {
        padmap::save(columns);
    }

    let label_of = |id: &str| -> String {
        all.iter()
            .find(|(_, w, _)| w == id)
            .map(|(_, _, l)| l.clone())
            .unwrap_or_else(|| "(消えたワークスペース)".into())
    };
    for (i, slot) in columns.iter().enumerate() {
        match slot {
            Some(id) => eprintln!("col {i}: {id} {}", label_of(id)),
            None => eprintln!("col {i}: (空き)"),
        }
    }
    let placed = columns.iter().filter(|c| c.is_some()).count();
    if all.len() > placed {
        eprintln!(
            "note: workspace {} 個のうち {} 個を載せています（バンク切替は未実装）",
            all.len(),
            placed
        );
    }
    Ok(agents)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut surface = Surface::open()?;
    surface.clear_all();
    eprintln!("APC mini mk2 に接続しました");

    let mut columns = padmap::load();
    let agents = sync_columns(&mut columns)?;
    let mut fleet = Fleet { columns, panes: HashMap::new() };
    fleet.seed(&agents);
    fleet.render(&mut surface);
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
                let agents = sync_columns(&mut fleet.columns)?;
                fleet.seed(&agents);
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
