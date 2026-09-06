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

pub struct Surface {
    out: MidiOutputConnection,
    /// 送信済みの状態。差分だけ送るために持つ。
    shadow: [(u8, u8); 128],
}

impl Surface {
    pub fn open() -> Result<Self, Box<dyn std::error::Error>> {
        let mo = MidiOutput::new("herdr-apc-mini")?;
        let port = mo
            .ports()
            .into_iter()
            .find(|p| mo.port_name(p).map(|n| n == PORT_NAME).unwrap_or(false))
            .ok_or_else(|| format!("MIDI ポート {PORT_NAME} が見つかりません"))?;
        let out = mo.connect(&port, "herdr-apc-mini")?;
        Ok(Surface { out, shadow: [(255, 255); 128] })
    }

    /// パッド 1 枚を塗る。前回と同じなら送らない。
    pub fn set(&mut self, note: u8, color: u8, behavior: Behavior) {
        let ch = behavior.channel();
        let want = (color, ch);
        if self.shadow[note as usize] == want {
            return;
        }
        let _ = self.out.send(&[0x90 | ch, note, color]);
        self.shadow[note as usize] = want;
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
        let _ = self.out.send(&[0x90, note, lamp.velocity()]);
        self.shadow[note as usize] = want;
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
