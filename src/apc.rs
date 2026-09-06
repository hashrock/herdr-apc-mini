//! APC mini mk2 の MIDI 表現。
//!
//! 実機で確認した割り当て（Control ポート）:
//!   パッド        note 0 = 左下、右へ +1、上へ +8
//!   右列(シーン)  note 112(最上) 〜 119(最下)
//!   下段(トラック) note 100(最左) 〜 107
//!   Shift         note 122（右下隅、下段8個とは別）
//!   フェーダー    CC 48〜55、CC 56 = マスター

use midir::{MidiOutput, MidiOutputConnection};

pub const PORT_NAME: &str = "APC mini mk2 Control";

pub const SCENE_TOP: u8 = 112;
pub const TRACK_LEFT: u8 = 100;
pub const SHIFT: u8 = 122;
pub const FADER_CC_FIRST: u8 = 48;
pub const FADER_CC_MASTER: u8 = 56;

/// 点灯の挙動。MIDI チャンネルで表現される。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Behavior {
    Dim,
    Solid,
    Pulse,
    Blink,
}

impl Behavior {
    fn channel(self) -> u8 {
        match self {
            Behavior::Dim => 1,    // 25%
            Behavior::Solid => 6,  // 100%
            Behavior::Pulse => 9,  // 1/4 パルス
            Behavior::Blink => 14, // 1/4 点滅
        }
    }
}

/// パレット番号。実機で見て調整する前提の暫定値。
pub mod color {
    pub const OFF: u8 = 0;
    pub const GREY: u8 = 1;
    pub const WHITE: u8 = 3;
    pub const RED: u8 = 5;
    pub const ORANGE: u8 = 9;
    pub const GREEN: u8 = 21;
    pub const CYAN: u8 = 37;
    pub const BLUE: u8 = 45;
}

/// 設計上の行番号（0 = 一番上）を note 番号に変換する。
///
/// 実機は note 0 が左下なので上下が反転する。
pub fn pad_note(row: u8, col: u8) -> u8 {
    (7 - row) * 8 + col
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

    pub fn clear_all(&mut self) {
        for n in 0u8..64 {
            self.off(n);
        }
    }
}
