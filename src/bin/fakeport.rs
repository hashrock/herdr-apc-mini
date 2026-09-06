//! 抜き差しの動きを実機なしで確かめるための使い捨て。仮想ポートを一定時間だけ出す。
use midir::os::unix::VirtualInput;
use midir::MidiInput;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let name = std::env::args().nth(1).unwrap_or_else(|| "存在しないポート".into());
    let secs: u64 = std::env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(8);
    let mi = MidiInput::new("fakeport")?;
    let _conn = mi.create_virtual(&name, |_, _, _| {}, ())?;
    eprintln!("仮想ポート {name} を {secs} 秒だけ出します");
    std::thread::sleep(std::time::Duration::from_secs(secs));
    eprintln!("仮想ポートを消します");
    Ok(())
}
