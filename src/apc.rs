#![allow(dead_code)] // ハードウェアの記述なので、まだ使っていない定数も持っておく
//! APC mini mk2 の MIDI 表現。
//!
//! 割り当ては AKAI の "APC mini mk2 Communication Protocol v1.0" と実機の
//! 両方で確認済み（Control ポート）:
//!
//!   パッド         note 0x00-0x3F。note 0 = 左下、右へ +1、上へ +8（向きは実機で確認）
//!   下段(トラック) note 0x64-0x6B (100-107)、最左が 1
//!   右列(シーン)   note 0x70-0x77 (112-119)、最上が 1
//!   Shift          note 0x7A (122)。LED は無い
//!   フェーダー     CC 0x30-0x37 (48-55)、CC 0x38 (56) = マスター

use midir::{MidiOutput, MidiOutputConnection};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

pub const PORT_NAME: &str = "APC mini mk2 Control";

pub const SCENE_TOP: u8 = 0x70;
pub const TRACK_LEFT: u8 = 0x64;
pub const SHIFT: u8 = 0x7A;
pub const FADER_CC_FIRST: u8 = 0x30;
pub const FADER_CC_MASTER: u8 = 0x38;

/// 点灯の挙動。MIDI チャンネルで表現される。実機で確認済み。
///
///   ch 0-6  : 明度。**ch 0 と 1 は暗すぎて見えない**。ch 2 が視認できる下限
///   ch 7-10 : なめらかなパルス（7 が速く、10 が遅い）
///   ch 11-15: 二値の点滅（11 が速く、15 が遅い）
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Behavior {
    /// 見える下限。背景として置くもの向け。
    Dim,
    Half,
    Solid,
    /// なめらかな明滅。作業中を表す。
    Pulse,
    /// 二値の速い点滅。手が要ることを表す。
    Blink,
}

impl Behavior {
    fn channel(self) -> u8 {
        match self {
            Behavior::Dim => 2,
            Behavior::Half => 4,
            Behavior::Solid => 6,
            Behavior::Pulse => 9,
            Behavior::Blink => 11,
        }
    }
}

/// パレット番号（velocity）。仕様書の 128 色表から。
///
/// 仕様上 1 は #1E1E1E、2 は #7F7F7F の灰だが、実機では 1・2・3 のいずれも
/// 白っぽく見える。くすんだ表現は色ではなく明度（`Behavior::Dim`）で作る。
pub mod color {
    pub const OFF: u8 = 0;
    pub const WHITE: u8 = 3;
    pub const RED: u8 = 5;
    pub const ORANGE: u8 = 9;
    pub const YELLOW: u8 = 13;
    pub const GREEN: u8 = 21;
    pub const BLUE: u8 = 45;
}

/// 単色 LED ボタン（下段・右列）の点灯。
///
/// RGB パッドとは別のメッセージ形式で、**チャンネルは常に 0**、
/// velocity は 3 値しかない。色は選べない（下段=赤、右列=緑で固定）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Lamp {
    Off,
    On,
    Blink,
}

impl Lamp {
    fn velocity(self) -> u8 {
        match self {
            Lamp::Off => 0x00,
            Lamp::On => 0x01,
            Lamp::Blink => 0x02,
        }
    }
}

/// 左上から数えた通し番号を note 番号に変換する。
///
/// 実機は note 0 が左下なので上下が反転する。
pub fn pad_note(slot: usize) -> u8 {
    let row = slot / 8;
    let col = slot % 8;
    ((7 - row) * 8 + col) as u8
}

/// note 番号を左上から数えた通し番号に戻す。パッド以外なら `None`。
pub fn pad_slot(note: u8) -> Option<usize> {
    if note >= 64 {
        return None;
    }
    let row = 7 - (note / 8);
    let col = note % 8;
    Some((row * 8 + col) as usize)
}

/// 盤面が今どこにも繋がっていないことを表す影の値。実在しない色・チャンネル。
const UNKNOWN: (u8, u8) = (255, 255);

/// 抜き差しを見張り続ける。`report(ポートがあるか, 構成が変わったか)` を呼び、
/// `false` が返ったら終わる。**必ず専用スレッドで呼ぶこと**（run loop を回すため）。
///
/// CoreMIDI のポート一覧は**プロセス内にキャッシュ**されていて、run loop を
/// 回さない限り更新されない。クライアントを作り直しても駄目で、素直に sleep で
/// 待つと挿し直しに永久に気づけない（仮想ポートを出して確かめた）。
///
/// 有無を見るだけだと、**間隔より速い抜き差しを取りこぼす**。抜けたことに
/// 気づかないまま繋がっている扱いが続き、実機は消灯しているのに差分しか
/// 送られない。そこで構成変更の通知も一緒に見る。
pub fn watch_ports(interval: Duration, mut report: impl FnMut(bool, bool) -> bool) {
    // 通知の宛先になるクライアント。run loop を回すこのスレッドで作る。
    let _notify = notify_client();
    loop {
        if !report(port_present(), took_change()) {
            return;
        }
        wait(interval);
    }
}

/// 構成が変わった、という印。通知スレッドが立て、見張りが降ろす。
static CHANGED: AtomicBool = AtomicBool::new(false);

fn took_change() -> bool {
    CHANGED.swap(false, Ordering::Relaxed)
}

/// 構成変更を受け取るクライアント。**掴んだままにしないと通知が来ない。**
#[cfg(target_os = "macos")]
fn notify_client() -> Option<coremidi::Client> {
    coremidi::Client::new_with_notifications("herdr-apc-mini-watch", |n: &coremidi::Notification| {
        if matches!(n, coremidi::Notification::SetupChanged) {
            CHANGED.store(true, Ordering::Relaxed);
        }
    })
    .map_err(|e| eprintln!("MIDI の構成変更を受け取れません: {e}"))
    .ok()
}

#[cfg(not(target_os = "macos"))]
fn notify_client() -> Option<()> {
    None
}

fn port_present() -> bool {
    let Ok(mo) = MidiOutput::new("herdr-apc-mini-probe") else {
        return false;
    };
    mo.ports()
        .iter()
        .any(|p| mo.port_name(p).map(|n| n == PORT_NAME).unwrap_or(false))
}

#[cfg(target_os = "macos")]
#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    static kCFRunLoopDefaultMode: *const std::ffi::c_void;
    fn CFRunLoopRunInMode(mode: *const std::ffi::c_void, seconds: f64, after: u8) -> i32;
}

/// 溜まっている CoreMIDI の通知を、このスレッドで捌く。
///
/// **キャッシュはスレッドごと**で、run loop を回したスレッドしか更新されない。
/// 見張りスレッドがポートの出現に気づいても、`connect` するスレッドが古い一覧を
/// 見ていたら繋げない。繋ぎにいく前にここを通すこと。
#[cfg(target_os = "macos")]
pub fn pump() {
    unsafe { CFRunLoopRunInMode(kCFRunLoopDefaultMode, 0.0, 0) };
}

#[cfg(not(target_os = "macos"))]
pub fn pump() {}

/// CoreMIDI の通知を受け取りながら待つ。
#[cfg(target_os = "macos")]
fn wait(dur: Duration) {
    let start = Instant::now();
    unsafe { CFRunLoopRunInMode(kCFRunLoopDefaultMode, dur.as_secs_f64(), 0) };
    // ソースが無いと即返る。空回りさせないよう、残りは素直に寝る。
    if let Some(rest) = dur.checked_sub(start.elapsed()) {
        std::thread::sleep(rest);
    }
}

#[cfg(not(target_os = "macos"))]
fn wait(dur: Duration) {
    std::thread::sleep(dur);
}

pub struct Surface {
    /// 繋がっていなければ `None`。抜かれているあいだは何も送らない。
    out: Option<MidiOutputConnection>,
    /// 送信済みの状態。差分だけ送るために持つ。
    shadow: [(u8, u8); 128],
}

impl Surface {
    /// まだ繋いでいない盤面。実機が無くてもデーモンは動く。
    pub fn new() -> Self {
        Surface { out: None, shadow: [UNKNOWN; 128] }
    }

    pub fn connected(&self) -> bool {
        self.out.is_some()
    }

    /// ポートを探して繋ぐ。繋げたら `true`。
    ///
    /// 実機は抜かれているあいだに消灯しているので、影は必ず捨てる。
    /// そうしないと「前回と同じ」と判断して描き直しを飛ばしてしまう。
    pub fn connect(&mut self) -> bool {
        let Ok(mo) = MidiOutput::new("herdr-apc-mini") else {
            return false;
        };
        let Some(port) = mo
            .ports()
            .into_iter()
            .find(|p| mo.port_name(p).map(|n| n == PORT_NAME).unwrap_or(false))
        else {
            return false;
        };
        let Ok(out) = mo.connect(&port, "herdr-apc-mini") else {
            return false;
        };
        self.out = Some(out);
        self.shadow = [UNKNOWN; 128];
        true
    }

    pub fn disconnect(&mut self) {
        self.out = None;
        self.shadow = [UNKNOWN; 128];
    }

    /// 1 メッセージ送る。送れたら `true`。
    ///
    /// 失敗は抜かれたものとして扱う。挿し直しは見張り側が拾う。
    fn send(&mut self, msg: &[u8]) -> bool {
        let Some(out) = self.out.as_mut() else {
            return false;
        };
        if out.send(msg).is_err() {
            eprintln!("盤面への送信に失敗しました。抜かれたものとして扱います");
            self.disconnect();
            return false;
        }
        true
    }

    /// パッド 1 枚を塗る。前回と同じなら送らない。
    pub fn set(&mut self, note: u8, color: u8, behavior: Behavior) {
        let ch = behavior.channel();
        let want = (color, ch);
        if self.shadow[note as usize] == want {
            return;
        }
        if std::env::var("APC_DEBUG").is_ok() {
            eprintln!("pad note={note} color={color} ch={ch}");
        }
        if self.send(&[0x90 | ch, note, color]) {
            self.shadow[note as usize] = want;
        }
    }

    pub fn off(&mut self, note: u8) {
        self.set(note, color::OFF, Behavior::Solid);
    }

    /// 下段・右列の単色 LED ボタンを点ける。パッドとは形式が違う。
    pub fn set_lamp(&mut self, note: u8, lamp: Lamp) {
        let want = (lamp.velocity(), 0);
        if self.shadow[note as usize] == want {
            return;
        }
        if std::env::var("APC_DEBUG").is_ok() {
            eprintln!("lamp note={note} {lamp:?}");
        }
        if self.send(&[0x90, note, lamp.velocity()]) {
            self.shadow[note as usize] = want;
        }
    }

    pub fn clear_all(&mut self) {
        for n in 0u8..64 {
            self.off(n);
        }
        for n in TRACK_LEFT..TRACK_LEFT + 8 {
            self.set_lamp(n, Lamp::Off);
        }
        for n in SCENE_TOP..SCENE_TOP + 8 {
            self.set_lamp(n, Lamp::Off);
        }
    }
}
