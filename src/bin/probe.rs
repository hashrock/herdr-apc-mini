use midir::{Ignore, MidiInput, MidiOutput};
use std::sync::mpsc;
use std::time::{Duration, Instant};

const PORT: &str = "APC mini mk2 Control";

fn find<T: midir::MidiIO>(io: &T, name: &str) -> Option<T::Port> {
    io.ports()
        .into_iter()
        .find(|p| io.port_name(p).map(|n| n == name).unwrap_or(false))
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mo = MidiOutput::new("herdr-apc-probe-out")?;
    let op = find(&mo, PORT).ok_or("Control output port not found")?;
    let mut out = mo.connect(&op, "probe-out")?;

    // すべて消灯
    for n in 0u8..64 {
        out.send(&[0x90, n, 0])?;
    }
    // 四隅を note 番号空間で塗り分ける (channel 6 = 100% brightness)
    // 5=赤 21=緑 45=青 3=白
    for (note, vel, label) in [(0u8, 5u8, "note0=赤"), (7, 21, "note7=緑"), (56, 45, "note56=青"), (63, 3, "note63=白")] {
        out.send(&[0x96, note, vel])?;
        println!("lit {}", label);
    }

    let mut mi = MidiInput::new("herdr-apc-probe-in")?;
    mi.ignore(Ignore::None);
    let ip = find(&mi, PORT).ok_or("Control input port not found")?;
    let (tx, rx) = mpsc::channel::<(u64, Vec<u8>)>();
    let _conn = mi.connect(&ip, "probe-in", move |ts, msg, _| {
        let _ = tx.send((ts, msg.to_vec()));
    }, ())?;

    let secs: u64 = std::env::args().nth(1).and_then(|a| a.parse().ok()).unwrap_or(40);
    println!("--- 入力待ち {} 秒 ---", secs);
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(secs) {
        if let Ok((_, m)) = rx.recv_timeout(Duration::from_millis(200)) {
            let kind = match m[0] & 0xF0 {
                0x90 => "NoteOn ",
                0x80 => "NoteOff",
                0xB0 => "CC     ",
                _ => "other  ",
            };
            println!("{:6.2}s {} ch={} data={:?}", start.elapsed().as_secs_f32(), kind, m[0] & 0x0F, &m[1..]);
        }
    }
    println!("--- 終了 ---");
    Ok(())
}
