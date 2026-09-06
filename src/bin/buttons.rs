//! 下段・右列の単色 LED が実際どう光るかを見る。
use midir::MidiOutput;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mo = MidiOutput::new("herdr-apc-buttons")?;
    let port = mo
        .ports()
        .into_iter()
        .find(|p| mo.port_name(p).map(|n| n == "APC mini mk2 Control").unwrap_or(false))
        .ok_or("Control ポートが見つかりません")?;
    let mut out = mo.connect(&port, "buttons")?;
    for n in 0u8..64 {
        out.send(&[0x90, n, 0])?;
    }
    // 右列(シーン) 0x70-0x77: 上4つ点灯、下4つ点滅
    for i in 0u8..8 {
        let v = if i < 4 { 0x01 } else { 0x02 };
        out.send(&[0x90, 0x70 + i, v])?;
    }
    // 下段(トラック) 0x64-0x6B: 左4つ点灯、右4つ点滅
    for i in 0u8..8 {
        let v = if i < 4 { 0x01 } else { 0x02 };
        out.send(&[0x90, 0x64 + i, v])?;
    }
    println!("右列: 上4つ=点灯, 下4つ=点滅");
    println!("下段: 左4つ=点灯, 右4つ=点滅");
    Ok(())
}
