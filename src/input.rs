//! APC からの入力を受け取り、チャンネルへ流す。
//!
//! midir のコールバックは専用スレッドで回るので、socket 側のイベントと
//! 一本のチャンネルに合流させて main 側で順に捌く。

use midir::{Ignore, MidiInput, MidiInputConnection};
use std::sync::mpsc::Sender;

use crate::apc::PORT_NAME;

/// 盤面から来る生のイベント。
#[derive(Clone, Copy, Debug)]
pub enum Pad {
    Down(u8),
    Up(u8),
}

pub fn listen(tx: Sender<crate::Ev>) -> Result<MidiInputConnection<()>, Box<dyn std::error::Error>> {
    let mut mi = MidiInput::new("herdr-apc-mini-in")?;
    mi.ignore(Ignore::None);
    let port = mi
        .ports()
        .into_iter()
        .find(|p| mi.port_name(p).map(|n| n == PORT_NAME).unwrap_or(false))
        .ok_or_else(|| format!("MIDI 入力ポート {PORT_NAME} が見つかりません"))?;

    let conn = mi.connect(
        &port,
        "herdr-apc-mini-in",
        move |_, msg, _| {
            if msg.len() < 3 {
                return;
            }
            let pad = match (msg[0] & 0xF0, msg[2]) {
                (0x90, 0) => Pad::Up(msg[1]), // velocity 0 の Note On は離しと同じ
                (0x90, _) => Pad::Down(msg[1]),
                (0x80, _) => Pad::Up(msg[1]),
                _ => return,
            };
            let _ = tx.send(crate::Ev::Pad(pad));
        },
        (),
    )?;
    Ok(conn)
}
