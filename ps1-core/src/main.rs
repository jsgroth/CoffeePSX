use clap::Parser;
use env_logger::Env;
use ps1_core::api::Ps1Emulator;
use std::error::Error;
use std::fs;

#[derive(Debug, Parser)]
struct Args {
    #[arg(short = 'b', long, required = true)]
    bios_path: String,
    #[arg(short = 'e', long)]
    exe_path: Option<String>,
}

fn main() -> Result<(), Box<dyn Error>> {
    env_logger::Builder::from_env(Env::default().default_filter_or("info")).init();

    let args = Args::parse();

    log::info!("Loading BIOS from '{}'", args.bios_path);

    let bios_rom = fs::read(&args.bios_path)?;
    let mut emulator = Ps1Emulator::new(bios_rom)?;

    if let Some(exe_path) = &args.exe_path {
        log::info!("Sideloading EXE from '{exe_path}'");

        let exe = fs::read(exe_path)?;

        loop {
            emulator.tick();
            if emulator.cpu_pc() == 0x80030000 {
                emulator.sideload_exe(&exe)?;
                log::info!("EXE sideloaded");
                break;
            }
        }
    }

    for _ in 0..1_000_000_000 {
        emulator.tick();
    }

    Ok(())
}
