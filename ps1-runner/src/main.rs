use anyhow::anyhow;
use cdrom::reader::{CdRom, CdRomFileFormat};
use clap::Parser;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{BufferSize, OutputCallbackInfo, SampleRate, StreamConfig};
use env_logger::Env;
use minifb::{Key, Window, WindowOptions};
use ps1_core::api::{AudioOutput, Ps1Emulator, Renderer};
use ps1_core::input::Ps1Inputs;
use rayon::prelude::*;
use std::collections::VecDeque;
use std::ffi::OsStr;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};
use std::{fs, iter, thread};

#[derive(Debug, Parser)]
struct Args {
    #[arg(short = 'b', long, required = true)]
    bios_path: String,
    #[arg(short = 'e', long)]
    exe_path: Option<String>,
    #[arg(short = 'd', long)]
    disc_path: Option<String>,
    #[arg(short = 't', long, default_value_t)]
    tty_enabled: bool,
}

struct MiniFbRenderer<'a> {
    window: &'a mut Window,
    frame_buffer: &'a mut [u32],
    inputs: &'a mut Ps1Inputs,
    frame_count: &'a mut u64,
    last_fps_log: &'a mut SystemTime,
}

impl<'a> Renderer for MiniFbRenderer<'a> {
    type Err = anyhow::Error;

    fn render_frame(&mut self, vram: &[u8]) -> Result<(), Self::Err> {
        self.frame_buffer.par_chunks_exact_mut(1024).enumerate().for_each(|(y, row)| {
            for (x, fb_color) in row.iter_mut().enumerate() {
                let vram_addr = 2048 * y + 2 * x;
                let color = u16::from_le_bytes([vram[vram_addr], vram[vram_addr + 1]]);

                let r = color & 0x1F;
                let g = (color >> 5) & 0x1F;
                let b = (color >> 10) & 0x1F;

                *fb_color = rgb_5_to_8(b) | (rgb_5_to_8(g) << 8) | (rgb_5_to_8(r) << 16);
            }
        });

        self.window.update_with_buffer(self.frame_buffer, 1024, 512)?;

        update_inputs(self.window, self.inputs);

        *self.frame_count += 1;

        let elapsed = SystemTime::now().duration_since(*self.last_fps_log).unwrap();
        if elapsed >= Duration::from_secs(5) {
            log::info!("FPS: {}", *self.frame_count as f64 / elapsed.as_secs_f64());
            *self.frame_count = 0;
            *self.last_fps_log = SystemTime::now();
        }

        Ok(())
    }
}

fn update_inputs(window: &Window, inputs: &mut Ps1Inputs) {
    inputs.p1 = inputs
        .p1
        .with_up(window.is_key_down(Key::Up))
        .with_left(window.is_key_down(Key::Left))
        .with_right(window.is_key_down(Key::Right))
        .with_down(window.is_key_down(Key::Down))
        .with_cross(window.is_key_down(Key::X))
        .with_circle(window.is_key_down(Key::S))
        .with_square(window.is_key_down(Key::Z))
        .with_triangle(window.is_key_down(Key::A))
        .with_l1(window.is_key_down(Key::W))
        .with_l2(window.is_key_down(Key::Q))
        .with_r1(window.is_key_down(Key::E))
        .with_r2(window.is_key_down(Key::R))
        .with_start(window.is_key_down(Key::Enter))
        .with_select(window.is_key_down(Key::RightShift));
}

const RGB_5_TO_8: &[u32; 32] = &[
    0, 8, 16, 25, 33, 41, 49, 58, 66, 74, 82, 90, 99, 107, 115, 123, 132, 140, 148, 156, 165, 173,
    181, 189, 197, 206, 214, 222, 230, 239, 247, 255,
];

fn rgb_5_to_8(color: u16) -> u32 {
    RGB_5_TO_8[color as usize]
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
            audio_queue.len() >= 2400
        };

        if wait_for_audio {
            loop {
                if self.audio_queue.lock().unwrap().len() < 2400 {
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
    let audio_output = CpalAudioOutput { audio_queue: Arc::clone(&audio_queue) };

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
    assert!(
        args.disc_path.is_none() || args.exe_path.is_none(),
        "Disc path and EXE path cannot both be set"
    );

    log::info!("Loading BIOS from '{}'", args.bios_path);

    let bios_rom = fs::read(&args.bios_path)?;
    let mut emulator_builder = Ps1Emulator::builder(bios_rom).tty_enabled(args.tty_enabled);
    if let Some(disc_path) = &args.disc_path {
        log::info!("Loading CD-ROM image from '{disc_path}'");

        let format = match Path::new(disc_path).extension().and_then(OsStr::to_str) {
            Some("cue") => CdRomFileFormat::CueBin,
            Some("chd") => CdRomFileFormat::Chd,
            _ => panic!("Unknown CD-ROM image format: '{disc_path}'"),
        };

        let disc = CdRom::open(disc_path, format)?;
        emulator_builder = emulator_builder.with_disc(disc);
    }

    let mut emulator = emulator_builder.build()?;

    let mut frame_buffer = vec![0; 1024 * 512];

    let window_title = match (&args.disc_path, &args.exe_path) {
        (None, None) => "PS1 - (BIOS only)".into(),
        (Some(disc_path), None) => {
            format!(
                "PS1 - {}",
                Path::new(disc_path).with_extension("").file_name().unwrap().to_str().unwrap()
            )
        }
        (None, Some(exe_path)) => {
            format!("PS1 - {}", Path::new(exe_path).file_name().unwrap().to_str().unwrap())
        }
        (Some(_), Some(_)) => unreachable!(),
    };

    let mut window = Window::new(&window_title, 1024, 512, WindowOptions::default())?;

    let (mut audio_output, audio_stream) = create_audio_output()?;
    audio_stream.play()?;

    let mut inputs = Ps1Inputs::default();

    if let Some(exe_path) = &args.exe_path {
        log::info!("Sideloading EXE from '{exe_path}'");

        let exe = fs::read(exe_path)?;

        // The BIOS copies its shell to $00030000 in main RAM, and after it's initialized the kernel
        // it jumps to $80030000 to begin shell execution.
        // Sideload EXEs by stealing execution from the BIOS once it reaches this point.
        loop {
            emulator.tick(
                inputs,
                &mut MiniFbRenderer {
                    window: &mut window,
                    frame_buffer: &mut frame_buffer,
                    inputs: &mut inputs,
                    frame_count: &mut 0,
                    last_fps_log: &mut SystemTime::now(),
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

    let mut frame_count = 0;
    let mut last_fps_log = SystemTime::now();
    let mut paused = false;
    let mut pause_pressed = false;
    while window.is_open() && !window.is_key_down(Key::Escape) {
        if !paused {
            emulator.tick(
                inputs,
                &mut MiniFbRenderer {
                    window: &mut window,
                    frame_buffer: &mut frame_buffer,
                    inputs: &mut inputs,
                    frame_count: &mut frame_count,
                    last_fps_log: &mut last_fps_log,
                },
                &mut audio_output,
            )?;
        } else {
            thread::sleep(Duration::from_micros(16667));
            window.update_with_buffer(&frame_buffer, 1024, 512)?;
        }

        if !pause_pressed && window.is_key_down(Key::P) {
            paused = !paused;
            pause_pressed = true;

            if paused {
                *audio_output.audio_queue.lock().unwrap() =
                    iter::once((0.0, 0.0)).cycle().take(1024 * 1024).collect();
            } else {
                audio_output.audio_queue.lock().unwrap().clear();
            }
        }

        if !window.is_key_down(Key::P) {
            pause_pressed = false;
        }
    }

    Ok(())
}
