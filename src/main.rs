//! APC mini mk2 を herdr の物理コンソールにするデーモン。
//!
//! 現段階は読み取り専用のダッシュボード。状態ランプの行だけを描く。
//! 縦列 = プロジェクト（workspace）、横行 = アクション。

mod apc;
mod git;
mod herdr;
mod input;
mod padmap;
mod verbs;

use apc::{color, Behavior, Lamp, Surface};
use herdr::{Client, Status};
use input::Pad;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::mpsc;
use std::time::Duration;

use padmap::SLOTS;
use verbs::{Reasons, Simplified, Verb, SOFT_KEYS};

/// 発火したことを目で分かるようにする白フラッシュの長さ。
const FLASH: Duration = Duration::from_millis(120);
/// git の状態を見にいく間隔。herdr のイベントでは分からないのでここだけポーリング。
const GIT_POLL: Duration = Duration::from_secs(30);

/// main のループが捌くイベント。MIDI と socket を一本に合流させる。
pub enum Ev {
    Pad(Pad),
    Herdr(Value),
    /// git を見て回った結果。
    Reasons(HashMap<String, Reasons>),
    Closed,
}

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
    /// 枠に割り当てたワークスペース（左上から）。穴は None。
    slots: Vec<Option<String>>,
    /// pane_id -> (workspace_id, status)。エージェントのいるペインだけ。
    panes: HashMap<String, (String, Status)>,
    /// git 由来の「理由がある」。ポーリングで更新される。
    reasons: HashMap<String, Reasons>,
    /// simplify を済ませた時点の HEAD。
    simplified: Simplified,
    /// simplify を投入して、まだ終わっていないワークスペース。
    pending_simplify: HashMap<String, ()>,
}

impl Fleet {
    /// その動詞に実行の余地があるか。
    ///
    /// 理由があることと、今すぐ実行できること（agent がいて idle/done）の両方。
    fn opportunity(&self, ws: &str, verb: Verb) -> bool {
        if !self.reasons.get(ws).map(|r| r.has(verb)).unwrap_or(false) {
            return false;
        }
        matches!(self.status_of(ws), Some(Status::Idle) | Some(Status::Done))
    }

    fn any_opportunity(&self, verb: Verb) -> bool {
        self.slots
            .iter()
            .flatten()
            .any(|ws| self.opportunity(ws, verb))
    }

    /// soft key を押している間の地図。余地のあるところだけ緑にする。
    fn render_map(&self, surface: &mut Surface, verb: Verb) {
        for slot in 0..SLOTS {
            let note = apc::pad_note(slot);
            match self.slots.get(slot).and_then(|c| c.as_ref()) {
                Some(ws) if self.opportunity(ws, verb) => {
                    surface.set(note, color::GREEN, Behavior::Solid)
                }
                Some(_) => surface.set(note, color::WHITE, Behavior::Dim),
                None => surface.off(note),
            }
        }
    }

    /// 右列の LED。余地がどこかにあれば点灯。
    fn render_soft_keys(&self, surface: &mut Surface, held: Option<usize>) {
        for (i, verb) in SOFT_KEYS.iter().enumerate() {
            let note = apc::SCENE_TOP + i as u8;
            let lamp = match verb {
                _ if held == Some(i) => Lamp::Blink,
                Some(v) if self.any_opportunity(*v) => Lamp::On,
                _ => Lamp::Off,
            };
            surface.set_lamp(note, lamp);
        }
    }
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
    /// soft key の verb を撃つときの宛先になる。
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
        for slot in 0..SLOTS {
            let note = apc::pad_note(slot);
            match self.slots.get(slot).and_then(|c| c.as_ref()) {
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
fn sync_slots(
    slots: &mut Vec<Option<String>>,
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

    if padmap::assign(slots, &preferred, &rest) {
        padmap::save(slots);
    }

    let label_of = |id: &str| -> String {
        all.iter()
            .find(|(_, w, _)| w == id)
            .map(|(_, _, l)| l.clone())
            .unwrap_or_else(|| "(消えたワークスペース)".into())
    };
    for (i, slot) in slots.iter().enumerate() {
        if let Some(id) = slot {
            eprintln!("pad {i}: {id} {}", label_of(id));
        }
    }
    let placed = slots.iter().filter(|c| c.is_some()).count();
    if all.len() > placed {
        eprintln!("note: workspace {} 個のうち {placed} 個しか載りません", all.len());
    }
    Ok(agents)
}



/// 押した瞬間が分かるように白く光らせ、すぐ元の絵に戻す。
fn flash(surface: &mut Surface, note: u8, fleet: &Fleet, held: Option<usize>) {
    surface.set(note, color::WHITE, Behavior::Solid);
    std::thread::sleep(FLASH);
    redraw(surface, fleet, held);
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut surface = Surface::open()?;
    surface.clear_all();
    eprintln!("APC mini mk2 に接続しました");

    let mut slots = padmap::load();
    let agents = sync_slots(&mut slots)?;
    let mut fleet = Fleet {
        slots,
        panes: HashMap::new(),
        reasons: HashMap::new(),
        simplified: verbs::load_simplified(),
        pending_simplify: HashMap::new(),
    };
    fleet.seed(&agents);
    fleet.render(&mut surface);
    fleet.render_soft_keys(&mut surface, None);

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

    // git は herdr のイベントに現れないので、ここだけ定期的に見にいく。
    {
        let tx = tx.clone();
        std::thread::spawn(move || loop {
            let cwds = workspace_cwds();
            let simplified = verbs::load_simplified();
            let reasons = verbs::scan(&cwds, &simplified);
            if tx.send(Ev::Reasons(reasons)).is_err() {
                return;
            }
            std::thread::sleep(GIT_POLL);
        });
    }

    // 押している soft key。押している間は 8x8 の意味が変わる。
    let mut held_soft: Option<usize> = None;

    loop {
        let Ok(ev) = rx.recv() else { break };
        match ev {
            Ev::Pad(Pad::Down(note)) => {
                if let Some(slot) = apc::pad_slot(note) {
                    on_pad(&mut surface, &mut fleet, slot, held_soft);
                } else if let Some(i) = soft_index(note) {
                    held_soft = Some(i);
                    redraw(&mut surface, &fleet, held_soft);
                } else if note == apc::TRACK_LEFT {
                    // Volume = OK。フォーカス中のエージェントに Enter を送る。
                    lamp_flash(&mut surface, note);
                    act_on_focused("ok", |target| {
                        herdr::request(
                            "agent.send_keys",
                            json!({ "target": target, "keys": ["enter"] }),
                        )
                    });
                } else if note == apc::TRACK_LEFT + 1 {
                    // Pan = 推奨案で進めて。
                    lamp_flash(&mut surface, note);
                    act_on_focused("推奨案", |target| {
                        herdr::request(
                            "agent.prompt",
                            json!({ "target": target, "text": "推奨案で進めて" }),
                        )
                    });
                }
            }
            Ev::Pad(Pad::Up(note)) => {
                if let Some(i) = soft_index(note) {
                    if held_soft == Some(i) {
                        held_soft = None;
                        redraw(&mut surface, &fleet, held_soft);
                    }
                }
            }
            Ev::Herdr(msg) => {
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
                            let agents = sync_slots(&mut fleet.slots)?;
                            fleet.seed(&agents);
                            true
                        }
                        _ => false,
                    };
                    if dirty {
                        // simplify を投げた相手が手を止めたら、その時点の HEAD を記録する。
                        settle_simplify(&mut fleet);
                        redraw(&mut surface, &fleet, held_soft);
                    }
                }
            }
            Ev::Reasons(reasons) => {
                if fleet.reasons != reasons {
                    fleet.reasons = reasons;
                    fleet.simplified = verbs::load_simplified();
                    redraw(&mut surface, &fleet, held_soft);
                }
            }
            Ev::Closed => {
                eprintln!("herdr との接続が閉じました");
                break;
            }
        }
    }
    Ok(())
}

/// 右列 soft key の note なら、上から数えた番号。
fn soft_index(note: u8) -> Option<usize> {
    (note >= apc::SCENE_TOP && note < apc::SCENE_TOP + 8)
        .then(|| (note - apc::SCENE_TOP) as usize)
}

/// パッドを押したとき。soft key を押していなければ、そこへ飛ぶだけ。
fn on_pad(surface: &mut Surface, fleet: &mut Fleet, slot: usize, held_soft: Option<usize>) {
    let Some(Some(ws)) = fleet.slots.get(slot) else { return };
    let ws = ws.clone();
    let note = apc::pad_note(slot);

    let Some(i) = held_soft else {
        flash(surface, note, fleet, held_soft);
        if let Err(e) = herdr::request("workspace.focus", json!({ "workspace_id": ws })) {
            eprintln!("workspace.focus に失敗: {e}");
        }
        return;
    };

    let Some(Some(verb)) = SOFT_KEYS.get(i) else { return };
    let verb = *verb;
    if !fleet.opportunity(&ws, verb) {
        refuse(surface, note, fleet, held_soft);
        return;
    }
    let Some(target) = fleet.pane_of(&ws) else {
        refuse(surface, note, fleet, held_soft);
        return;
    };
    flash(surface, note, fleet, held_soft);
    eprintln!("{}: {ws} へ投入します", verb.label());
    match herdr::request(
        "agent.prompt",
        json!({ "target": target, "text": verb.prompt() }),
    ) {
        Ok(_) => {
            if verb == Verb::Simplify {
                fleet.pending_simplify.insert(ws, ());
            }
        }
        Err(e) => eprintln!("{} の投入に失敗: {e}", verb.label()),
    }
}

/// 撃てないときの拒否。赤く一瞬光らせる。
fn refuse(surface: &mut Surface, note: u8, fleet: &Fleet, held: Option<usize>) {
    surface.set(note, color::RED, Behavior::Solid);
    std::thread::sleep(FLASH);
    redraw(surface, fleet, held);
}

/// 盤面ぜんぶを描き直す。soft key を押している間は地図に変わる。
fn redraw(surface: &mut Surface, fleet: &Fleet, held: Option<usize>) {
    match held.and_then(|i| SOFT_KEYS.get(i).copied().flatten()) {
        Some(verb) => fleet.render_map(surface, verb),
        None => fleet.render(surface),
    }
    fleet.render_soft_keys(surface, held);
}

/// simplify を投げた相手が idle/done に戻ったら、その時点の HEAD を記録する。
///
/// 「済み」は投入した時点ではなく**終わった時点の HEAD**に対して成り立つ。
fn settle_simplify(fleet: &mut Fleet) {
    let done: Vec<String> = fleet
        .pending_simplify
        .keys()
        .filter(|ws| matches!(fleet.status_of(ws), Some(Status::Idle) | Some(Status::Done)))
        .cloned()
        .collect();
    if done.is_empty() {
        return;
    }
    let cwds = workspace_cwds();
    for ws in done {
        fleet.pending_simplify.remove(&ws);
        let Some(cwd) = cwds.get(&ws) else { continue };
        let Some(head) = git::head(cwd) else { continue };
        eprintln!("simplify 済みとして記録します: {ws} @ {head}");
        verbs::save_simplified(&ws, &head);
        fleet.simplified.insert(ws, head);
    }
    fleet.reasons = verbs::scan(&cwds, &fleet.simplified);
}

/// workspace_id -> 作業ディレクトリ。git を見るのに要る。
fn workspace_cwds() -> HashMap<String, String> {
    let mut out = HashMap::new();
    let Ok(result) = herdr::request("pane.list", json!({})) else {
        return out;
    };
    for p in result["panes"].as_array().map(|v| v.as_slice()).unwrap_or(&[]) {
        let (Some(ws), Some(cwd)) = (
            p.get("workspace_id").and_then(|v| v.as_str()),
            p.get("cwd").and_then(|v| v.as_str()),
        ) else {
            continue;
        };
        out.entry(ws.to_string()).or_insert_with(|| cwd.to_string());
    }
    out
}

/// 単色 LED のボタンを一瞬光らせる。
fn lamp_flash(surface: &mut Surface, note: u8) {
    surface.set_lamp(note, Lamp::On);
    std::thread::sleep(FLASH);
    surface.set_lamp(note, Lamp::Off);
}

/// フォーカス中のエージェントに対して何かする。
///
/// 宛先を状態として持たず、その都度 `agent.list` から引く。取り違えが起きない。
fn act_on_focused<F>(what: &str, f: F)
where
    F: FnOnce(&str) -> Result<Value, Box<dyn std::error::Error>>,
{
    let agents = match herdr::request("agent.list", json!({})) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("agent.list に失敗: {e}");
            return;
        }
    };
    let target = agents["agents"]
        .as_array()
        .map(|v| v.as_slice())
        .unwrap_or(&[])
        .iter()
        .find(|a| a.get("focused").and_then(|v| v.as_bool()).unwrap_or(false))
        .and_then(|a| a.get("pane_id").and_then(|v| v.as_str()));
    let Some(target) = target else {
        eprintln!("{what}: フォーカス中のエージェントがいません");
        return;
    };
    if let Err(e) = f(target) {
        eprintln!("{what} の送信に失敗: {e}");
    }
}
