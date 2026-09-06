//! APC mini mk2 を herdr の物理コンソールにするデーモン。
//!
//! 現段階は読み取り専用のダッシュボード。状態ランプの行だけを描く。
//! 縦列 = プロジェクト（workspace）、横行 = アクション。

mod apc;
mod herdr;
mod input;
mod padmap;

use apc::{color, Behavior, Surface};
use herdr::{Client, Status};
use input::Pad;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::{Duration, Instant};

/// 状態ランプを置く行。フェーダーの真上に来るよう最下段に置く。
const ROW_STATUS: u8 = 7;
/// これ以上押し続けたら長押しとみなす。
const LONG_PRESS: Duration = Duration::from_millis(800);
/// 発火したことを目で分かるようにする白フラッシュの長さ。
const FLASH: Duration = Duration::from_millis(120);

/// main のループが捌くイベント。MIDI と socket を一本に合流させる。
pub enum Ev {
    Pad(Pad),
    Herdr(Value),
    Closed,
}
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

    /// そのワークスペースで一番手が要るエージェントのペイン。
    fn pane_of(&self, workspace_id: &str) -> Option<String> {
        let mut best: Option<(&String, Status)> = None;
        for (pane_id, (ws, st)) in &self.panes {
            if ws != workspace_id {
                continue;
            }
            best = match best {
                Some((_, prev)) if prev.max(*st) == prev => best,
                _ => Some((pane_id, *st)),
            };
        }
        best.map(|(id, _)| id.clone())
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

/// 状態ランプの行の note なら、その列番号。
fn status_column(note: u8) -> Option<usize> {
    (0..COLUMNS)
        .find(|c| apc::pad_note(ROW_STATUS, *c as u8) == note)
}

/// 押した瞬間が分かるように白く光らせ、すぐ元の絵に戻す。
fn flash(surface: &mut Surface, note: u8, fleet: &Fleet) {
    surface.set(note, color::WHITE, Behavior::Solid);
    std::thread::sleep(FLASH);
    fleet.render(surface);
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

    let (tx, rx) = mpsc::channel::<Ev>();
    let _midi = input::listen(tx.clone())?;
    eprintln!("盤面の入力を受け付けます");

    // 購読は別スレッドで回し、届いたものをチャンネルへ流す。
    {
        let tx = tx.clone();
        std::thread::spawn(move || {
            let mut client = match Client::connect() {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("herdr に接続できません: {e}");
                    let _ = tx.send(Ev::Closed);
                    return;
                }
            };
            // pane.updated は購読直後に全ペインをリプレイするが、その agent_status は
            // unknown なので、実状態は起動時の agent.list を種にしてある。
            if let Err(e) = client.subscribe(&[
                "pane.updated",
                "pane.closed",
                "workspace.created",
                "workspace.closed",
            ]) {
                eprintln!("購読に失敗しました: {e}");
                let _ = tx.send(Ev::Closed);
                return;
            }
            eprintln!("herdr のイベントを購読しました");
            loop {
                match client.next() {
                    Ok(Some(v)) => {
                        if tx.send(Ev::Herdr(v)).is_err() {
                            return;
                        }
                    }
                    _ => {
                        let _ = tx.send(Ev::Closed);
                        return;
                    }
                }
            }
        });
    }

    // note -> (押した時刻, 長押しを発火済みか)
    let mut held: HashMap<u8, (Instant, bool)> = HashMap::new();

    loop {
        // 長押しの判定が要るあいだは、その締め切りまでしか待たない。
        let wait = held
            .values()
            .filter(|(_, fired)| !fired)
            .map(|(t, _)| LONG_PRESS.saturating_sub(t.elapsed()))
            .min()
            .unwrap_or(Duration::from_secs(3600));

        match rx.recv_timeout(wait) {
            Ok(Ev::Pad(Pad::Down(note))) => {
                if status_column(note).is_some() {
                    held.insert(note, (Instant::now(), false));
                }
            }
            Ok(Ev::Pad(Pad::Up(note))) => {
                if let Some((at, fired)) = held.remove(&note) {
                    // 長押しで発火済みなら、離しでは何もしない。
                    if !fired && at.elapsed() < LONG_PRESS {
                        if let Some(col) = status_column(note) {
                            if let Some(Some(ws)) = fleet.columns.get(col) {
                                let ws = ws.clone();
                                flash(&mut surface, note, &fleet);
                                if let Err(e) = herdr::request(
                                    "workspace.focus",
                                    json!({ "workspace_id": ws }),
                                ) {
                                    eprintln!("workspace.focus に失敗: {e}");
                                }
                            }
                        }
                    }
                }
            }
            Ok(Ev::Herdr(msg)) => {
                if let Some(kind) = msg.get("event").and_then(|v| v.as_str()) {
                    let data = &msg["data"];
                    let dirty = match kind {
                        "pane_updated" => fleet.apply_pane(&data["pane"]),
                        "pane_closed" => data
                            .get("pane_id")
                            .and_then(|v| v.as_str())
                            .map(|id| fleet.panes.remove(id).is_some())
                            .unwrap_or(false),
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
            }
            Ok(Ev::Closed) => {
                eprintln!("herdr との接続が閉じました");
                break;
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }

        // 指を離す前に長押しを発火させる。押しっぱなしのまま結果が分かるように。
        let due: Vec<u8> = held
            .iter()
            .filter(|(_, (t, fired))| !fired && t.elapsed() >= LONG_PRESS)
            .map(|(n, _)| *n)
            .collect();
        for note in due {
            if let Some(entry) = held.get_mut(&note) {
                entry.1 = true;
            }
            let Some(col) = status_column(note) else { continue };
            let Some(Some(ws)) = fleet.columns.get(col) else { continue };
            let Some(pane) = fleet.pane_of(ws) else {
                eprintln!("col {col}: エージェントのいるペインがありません");
                continue;
            };
            flash(&mut surface, note, &fleet);
            if let Err(e) = herdr::request(
                "pane.zoom",
                json!({ "pane_id": pane, "mode": "toggle" }),
            ) {
                eprintln!("pane.zoom に失敗: {e}");
            }
        }
    }
    Ok(())
}
