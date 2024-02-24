use clap::Parser;
use env_logger::Env;
use minifb::{Key, Window, WindowOptions};
use ps1_core::api::{Ps1Emulator, Renderer};
use std::fs;
use std::path::Path;
use std::time::Duration;

#[derive(Debug, Parser)]
struct Args {
    #[arg(short = 'b', long, required = true)]
    bios_path: String,
    #[arg(short = 'e', long)]
    exe_path: Option<String>,
    #[arg(short = 't', long, default_value_t)]
    tty_enabled: bool,
}

struct MiniFbRenderer<'a> {
    window: &'a mut Window,
    frame_buffer: &'a mut [u32],
}

impl<'a> Renderer for MiniFbRenderer<'a> {
    type Err = anyhow::Error;

    fn render_frame(&mut self, vram: &[u8]) -> Result<(), Self::Err> {
        for y in 0..224 {
            for x in 0..512 {
                let vram_addr = (2048 * y + 2 * x) as usize;
                let color = u16::from_le_bytes([vram[vram_addr], vram[vram_addr + 1]]);

                let r = color & 0x1F;
                let g = (color >> 5) & 0x1F;
                let b = (color >> 10) & 0x1F;

                let color_u32 = rgb_5_to_8(b) | (rgb_5_to_8(g) << 8) | (rgb_5_to_8(r) << 16);

                self.frame_buffer[512 * 2 * y as usize + x as usize] = color_u32;
                self.frame_buffer[512 * (2 * y + 1) as usize + x as usize] = color_u32;
            }
        }

        self.window
            .update_with_buffer(self.frame_buffer, 512, 448)?;

        Ok(())
    }
}

fn rgb_5_to_8(color: u16) -> u32 {
    (255.0 * color as f64 / 31.0).round() as u32
}

fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_env(Env::default().default_filter_or("info")).init();

    let args = Args::parse();

    log::info!("Loading BIOS from '{}'", args.bios_path);

    let bios_rom = fs::read(&args.bios_path)?;
    let mut emulator = Ps1Emulator::builder(bios_rom)
        .tty_enabled(args.tty_enabled)
        .build()?;
    let mut frame_buffer = vec![0; 512 * 448];

    let window_title = match &args.exe_path {
        None => "PS1".into(),
        Some(exe_path) => format!(
            "PS1 - {}",
            Path::new(exe_path).file_name().unwrap().to_str().unwrap()
        ),
    };

    let mut window = Window::new(&window_title, 512, 448, WindowOptions::default())?;

    window.limit_update_rate(Some(Duration::from_micros(16667)));

    if let Some(exe_path) = &args.exe_path {
        log::info!("Sideloading EXE from '{exe_path}'");

        let exe = fs::read(exe_path)?;

        // The BIOS copies its shell to $00030000 in main RAM, and after it's initialized the kernel
        // it jumps to $80030000 to begin shell execution.
        // Sideload EXEs by stealing execution from the BIOS once it reaches this point.
        loop {
            emulator.tick(&mut MiniFbRenderer {
                window: &mut window,
                frame_buffer: &mut frame_buffer,
            })?;
            if emulator.cpu_pc() == 0x80030000 {
                emulator.sideload_exe(&exe)?;
                log::info!("EXE sideloaded");
                break;
            }
        }
    }

    while window.is_open() && !window.is_key_down(Key::Escape) {
        emulator.tick(&mut MiniFbRenderer {
            window: &mut window,
            frame_buffer: &mut frame_buffer,
        })?;
    }

    Ok(())
}
