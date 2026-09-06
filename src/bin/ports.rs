use midir::{MidiInput, MidiOutput};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mi = MidiInput::new("herdr-apc-probe")?;
    println!("inputs:");
    for p in mi.ports().iter() {
        println!("  {}", mi.port_name(p)?);
    }
    let mo = MidiOutput::new("herdr-apc-probe")?;
    println!("outputs:");
    for p in mo.ports().iter() {
        println!("  {}", mo.port_name(p)?);
    }
    Ok(())
}
