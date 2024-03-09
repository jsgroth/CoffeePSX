use anyhow::anyhow;
use clap::Parser;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{BufferSize, OutputCallbackInfo, SampleRate, StreamConfig};
use env_logger::Env;
use minifb::{Window, WindowOptions};
use ps1_core::api::{AudioOutput, Ps1Emulator, Renderer};
use std::collections::VecDeque;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use std::{fs, thread};

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
        for y in 0..512 {
            for x in 0..1024 {
                let vram_addr = (2048 * y + 2 * x) as usize;
                let color = u16::from_le_bytes([vram[vram_addr], vram[vram_addr + 1]]);

                let r = color & 0x1F;
                let g = (color >> 5) & 0x1F;
                let b = (color >> 10) & 0x1F;

                let color_u32 = rgb_5_to_8(b) | (rgb_5_to_8(g) << 8) | (rgb_5_to_8(r) << 16);

                self.frame_buffer[1024 * y as usize + x as usize] = color_u32;
            }
        }

        self.window
            .update_with_buffer(self.frame_buffer, 1024, 512)?;

        Ok(())
    }
}

fn rgb_5_to_8(color: u16) -> u32 {
    (255.0 * f64::from(color) / 31.0).round() as u32
}

struct CpalAudioOutput {
    audio_queue: Arc<Mutex<VecDeque<(f64, f64)>>>,
}

impl AudioOutput for CpalAudioOutput {
    type Err = anyhow::Error;

    fn queue_samples(&mut self, samples: &[(f64, f64)]) -> Result<(), Self::Err> {
        let wait_for_audio = {
            let mut audio_queue = self.audio_queue.lock().unwrap();
            for &sample in samples {
                audio_queue.push_back(sample);
            }
            audio_queue.len() >= 1200
        };

        if wait_for_audio {
            loop {
                if self.audio_queue.lock().unwrap().len() < 1200 {
                    break;
                }
                thread::sleep(Duration::from_micros(250));
            }
        }

        Ok(())
    }
}

fn create_audio_output() -> anyhow::Result<(CpalAudioOutput, impl StreamTrait)> {
    let audio_queue = Arc::new(Mutex::new(VecDeque::with_capacity(1600)));
    let audio_output = CpalAudioOutput {
        audio_queue: Arc::clone(&audio_queue),
    };

    let audio_host = cpal::default_host();
    let audio_device = audio_host
        .default_output_device()
        .ok_or_else(|| anyhow!("No audio output device found"))?;
    let audio_stream = audio_device.build_output_stream(
        &StreamConfig {
            channels: 2,
            sample_rate: SampleRate(44100),
            buffer_size: BufferSize::Fixed(1024),
        },
        move |data: &mut [f32], _: &OutputCallbackInfo| {
            let mut audio_queue = audio_queue.lock().unwrap();
            for chunk in data.chunks_exact_mut(2) {
                let Some((sample_l, sample_r)) = audio_queue.pop_front() else {
                    return;
                };
                chunk[0] = sample_l as f32;
                chunk[1] = sample_r as f32;
            }
        },
        move |err| {
            log::error!("CPAL audio stream error: {err}");
        },
        None,
    )?;

    Ok((audio_output, audio_stream))
}

fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_env(Env::default().default_filter_or("info")).init();

    let args = Args::parse();

    log::info!("Loading BIOS from '{}'", args.bios_path);

    let bios_rom = fs::read(&args.bios_path)?;
    let mut emulator = Ps1Emulator::builder(bios_rom)
        .tty_enabled(args.tty_enabled)
        .build()?;
    let mut frame_buffer = vec![0; 1024 * 512];

    let window_title = match &args.exe_path {
        None => "PS1 - (BIOS only)".into(),
        Some(exe_path) => format!(
            "PS1 - {}",
            Path::new(exe_path).file_name().unwrap().to_str().unwrap()
        ),
    };

    let mut window = Window::new(&window_title, 1024, 512, WindowOptions::default())?;

    let (mut audio_output, audio_stream) = create_audio_output()?;
    audio_stream.play()?;

    if let Some(exe_path) = &args.exe_path {
        log::info!("Sideloading EXE from '{exe_path}'");

        let exe = fs::read(exe_path)?;

        // The BIOS copies its shell to $00030000 in main RAM, and after it's initialized the kernel
        // it jumps to $80030000 to begin shell execution.
        // Sideload EXEs by stealing execution from the BIOS once it reaches this point.
        loop {
            emulator.tick(
                &mut MiniFbRenderer {
                    window: &mut window,
                    frame_buffer: &mut frame_buffer,
                },
                &mut audio_output,
            )?;
            if emulator.cpu_pc() == 0x80030000 {
                emulator.sideload_exe(&exe)?;
                log::info!("EXE sideloaded");
                break;
            }
        }
    }

    while window.is_open() {
        emulator.tick(
            &mut MiniFbRenderer {
                window: &mut window,
                frame_buffer: &mut frame_buffer,
            },
            &mut audio_output,
        )?;
    }

    Ok(())
}
