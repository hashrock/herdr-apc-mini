//! LED の明度・パレット・点滅挙動を実機で見て決めるためのパターン。
use midir::MidiOutput;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mo = MidiOutput::new("herdr-apc-calib")?;
    let port = mo
        .ports()
        .into_iter()
        .find(|p| mo.port_name(p).map(|n| n == "APC mini mk2 Control").unwrap_or(false))
        .ok_or("Control ポートが見つかりません")?;
    let mut out = mo.connect(&port, "calib")?;
    for n in 0u8..64 {
        out.send(&[0x90, n, 0])?;
    }

    // 最下段 (note 0-6): 白(3) を チャンネル 0..6 → 明るさの段階のはず
    for ch in 0u8..7 {
        out.send(&[0x90 | ch, ch, 3])?;
    }

    // 下から2段目 (note 8-15): ch6 で 8 色
    let palette = [1u8, 2, 3, 5, 9, 13, 21, 45];
    for (i, c) in palette.iter().enumerate() {
        out.send(&[0x96, 8 + i as u8, *c])?;
    }

    // 下から3段目 (note 16-20): 赤(5) を ch 6,7,9,11,14 → 点灯/パルス/点滅
    for (i, ch) in [6u8, 7, 9, 11, 14].iter().enumerate() {
        out.send(&[0x90 | ch, 16 + i as u8, 5])?;
    }

    println!("最下段  note 0-6 : 白をチャンネル 0,1,2,3,4,5,6");
    println!("2段目   note 8-15: 色 1,2,3,5,9,13,21,45 を ch6");
    println!("3段目   note16-20: 赤を ch 6,7,9,11,14");
    Ok(())
}
