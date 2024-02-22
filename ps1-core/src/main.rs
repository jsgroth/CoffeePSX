use env_logger::Env;
use ps1_core::api::Ps1Emulator;
use std::error::Error;
use std::{env, fs};

fn main() -> Result<(), Box<dyn Error>> {
    env_logger::Builder::from_env(Env::default().default_filter_or("info")).init();

    let mut args = env::args();
    args.next();

    let filename = args.next().expect("Missing BIOS ROM filename arg");

    let bios_rom = fs::read(&filename)?;
    let mut emulator = Ps1Emulator::new(bios_rom)?;

    for _ in 0..10_000_000 {
        emulator.tick();
    }

    Ok(())
}
