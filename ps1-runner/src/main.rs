mod renderer;

use crate::renderer::WgpuRenderer;
use anyhow::{anyhow, Context};
use cdrom::reader::{CdRom, CdRomFileFormat};
use clap::Parser;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{BufferSize, OutputCallbackInfo, SampleRate, StreamConfig};
use env_logger::Env;
use ps1_core::api::{
    AudioOutput, DisplayConfig, Ps1Emulator, Ps1EmulatorBuilder, Ps1EmulatorState, SaveWriter,
    TickEffect,
};
use ps1_core::input::Ps1Inputs;
use ps1_core::RasterizerType;
use std::collections::VecDeque;
use std::ffi::OsStr;
use std::fs;
use std::fs::File;
use std::io::{BufReader, BufWriter};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use winit::dpi::LogicalSize;
use winit::event::{ElementState, Event, KeyEvent, WindowEvent};
use winit::event_loop::{ControlFlow, EventLoop, EventLoopWindowTarget};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowBuilder};

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
    #[arg(long = "no-audio-sync", default_value_t = true, action = clap::ArgAction::SetFalse)]
    audio_sync: bool,
    #[arg(long = "no-simd", default_value_t = true)]
    simd: bool,
}

struct CpalAudioOutput {
    audio_queue: Arc<Mutex<VecDeque<(f64, f64)>>>,
}

impl AudioOutput for CpalAudioOutput {
    type Err = anyhow::Error;

    fn queue_samples(&mut self, samples: &[(f64, f64)]) -> Result<(), Self::Err> {
        let mut audio_queue = self.audio_queue.lock().unwrap();
        for &sample in samples {
            audio_queue.push_back(sample);
        }

        Ok(())
    }
}

struct FsSaveWriter {
    path: PathBuf,
}

impl FsSaveWriter {
    fn load_memory_card(&self) -> anyhow::Result<Vec<u8>> {
        fs::read(&self.path)
            .context(format!("Error reading memory card 1 from '{}'", self.path.display()))
    }
}

impl SaveWriter for FsSaveWriter {
    type Err = anyhow::Error;

    fn save_memory_card_1(&mut self, card_data: &[u8]) -> Result<(), Self::Err> {
        fs::write(&self.path, card_data)
            .context(format!("Error saving memory card 1 to '{}'", self.path.display()))
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

struct HandleKeyEventArgs<'a, Stream> {
    emulator: &'a mut Ps1Emulator,
    window: &'a Window,
    renderer: &'a mut WgpuRenderer,
    audio_output: &'a CpalAudioOutput,
    audio_stream: &'a Stream,
    elwt: &'a EventLoopWindowTarget<()>,
    inputs: &'a mut Ps1Inputs,
    display_config: &'a mut DisplayConfig,
    save_state_path: &'a PathBuf,
    paused: &'a mut bool,
    step_to_next_frame: &'a mut bool,
}

fn handle_key_event<Stream: StreamTrait>(
    HandleKeyEventArgs {
        emulator,
        window,
        renderer,
        audio_output,
        audio_stream,
        elwt,
        inputs,
        display_config,
        save_state_path,
        paused,
        step_to_next_frame,
    }: HandleKeyEventArgs<'_, Stream>,
    event: KeyEvent,
) -> anyhow::Result<()> {
    let pressed = event.state == ElementState::Pressed;

    match event.physical_key {
        PhysicalKey::Code(keycode) => match keycode {
            KeyCode::ArrowUp => inputs.p1.set_up(pressed),
            KeyCode::ArrowLeft => inputs.p1.set_left(pressed),
            KeyCode::ArrowRight => inputs.p1.set_right(pressed),
            KeyCode::ArrowDown => inputs.p1.set_down(pressed),
            KeyCode::KeyX => inputs.p1.set_cross(pressed),
            KeyCode::KeyS => inputs.p1.set_circle(pressed),
            KeyCode::KeyZ => inputs.p1.set_square(pressed),
            KeyCode::KeyA => inputs.p1.set_triangle(pressed),
            KeyCode::KeyW => inputs.p1.set_l1(pressed),
            KeyCode::KeyQ => inputs.p1.set_l2(pressed),
            KeyCode::KeyE => inputs.p1.set_r1(pressed),
            KeyCode::KeyR => inputs.p1.set_r2(pressed),
            KeyCode::Enter => inputs.p1.set_start(pressed),
            KeyCode::ShiftRight => inputs.p1.set_select(pressed),
            KeyCode::Escape if pressed => elwt.exit(),
            KeyCode::F5 if pressed => save_state(save_state_path, emulator)?,
            KeyCode::F6 if pressed => load_state(save_state_path, emulator),
            KeyCode::Slash if pressed => renderer.toggle_prescaling(),
            KeyCode::KeyP if pressed => toggle_pause(paused, audio_output, audio_stream)?,
            KeyCode::KeyN if pressed => *step_to_next_frame = true,
            KeyCode::Semicolon if pressed => renderer.toggle_filter_mode(),
            KeyCode::Quote if pressed => {
                display_config.dump_vram = !display_config.dump_vram;
                emulator.update_display_config(*display_config);

                if display_config.dump_vram {
                    let _ = window.request_inner_size(LogicalSize::new(1024, 512));
                } else {
                    let _ = window.request_inner_size(LogicalSize::new(586, 448));
                }
            }
            KeyCode::Period if pressed => {
                display_config.crop_vertical_overscan = !display_config.crop_vertical_overscan;
                emulator.update_display_config(*display_config);
            }
            KeyCode::Minus if pressed => {
                display_config.rasterizer_type = RasterizerType::SimdSoftware;
                emulator.update_display_config(*display_config);

                log::info!("Using AVX2 software rasterizer");
            }
            KeyCode::Equal if pressed => {
                display_config.rasterizer_type = RasterizerType::NaiveSoftware;
                emulator.update_display_config(*display_config);

                log::info!("Using naive software rasterizer");
            }
            _ => {}
        },
        PhysicalKey::Unidentified(_) => {}
    }

    Ok(())
}

macro_rules! bincode_config {
    () => {
        bincode::config::standard()
            .with_little_endian()
            .with_fixed_int_encoding()
            .with_limit::<1_000_000_000>()
    };
}

fn save_state(path: &PathBuf, emulator: &Ps1Emulator) -> anyhow::Result<()> {
    let file = File::create(path)?;
    let mut writer = BufWriter::new(file);
    bincode::encode_into_std_write(emulator.to_state(), &mut writer, bincode_config!())?;

    log::info!("Saved state to '{}'", path.display());

    Ok(())
}

fn load_state(path: &PathBuf, emulator: &mut Ps1Emulator) {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(err) => {
            log::error!("Failed to open save state path at '{}': {err}", path.display());
            return;
        }
    };
    let mut reader = BufReader::new(file);

    match bincode::decode_from_std_read::<Ps1EmulatorState, _, _>(&mut reader, bincode_config!()) {
        Ok(loaded_state) => {
            let unserialized = emulator.take_unserialized_fields();
            *emulator = Ps1Emulator::from_state(loaded_state, unserialized);

            log::info!("Loaded state from '{}'", path.display());
        }
        Err(err) => {
            log::error!("Failed to load save state from '{}': {err}", path.display());
        }
    }
}

fn toggle_pause<Stream: StreamTrait>(
    paused: &mut bool,
    audio_output: &CpalAudioOutput,
    audio_stream: &Stream,
) -> anyhow::Result<()> {
    *paused = !*paused;

    audio_output.audio_queue.lock().unwrap().clear();

    if *paused {
        audio_stream.pause()?;
    } else {
        audio_stream.play()?;
    }

    Ok(())
}

fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_env(Env::default().default_filter_or("info")).init();

    let args = Args::parse();
    assert!(
        args.disc_path.is_none() || args.exe_path.is_none(),
        "Disc path and EXE path cannot both be set"
    );

    let save_state_path = match (&args.exe_path, &args.disc_path) {
        (Some(path), None) | (None, Some(path)) => Path::new(path).with_extension("ss0"),
        (None, None) => Path::new(&args.bios_path).with_extension("ss0"),
        (Some(_), Some(_)) => unreachable!(),
    };

    log::info!("Loading BIOS from '{}'", args.bios_path);

    let mut save_writer = FsSaveWriter { path: "card1.mcd".into() };
    let memory_card_1 = save_writer.load_memory_card().ok();

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

    let event_loop = EventLoop::new()?;
    let window = WindowBuilder::new()
        .with_title(window_title)
        .with_inner_size(LogicalSize::new(586, 448))
        .build(&event_loop)?;

    // SAFETY: The renderer does not outlive the window
    let mut renderer = pollster::block_on(unsafe {
        WgpuRenderer::new(&window, (window.inner_size().width, window.inner_size().height))
    })?;

    let mut display_config = DisplayConfig {
        rasterizer_type: if !args.simd {
            RasterizerType::NaiveSoftware
        } else {
            RasterizerType::default()
        },
        ..DisplayConfig::default()
    };

    let bios_rom = fs::read(&args.bios_path)?;
    let mut emulator_builder =
        Ps1EmulatorBuilder::new(bios_rom, renderer.device(), renderer.queue())
            .tty_enabled(args.tty_enabled)
            .with_display_config(display_config);
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

    if let Some(memory_card_1) = memory_card_1 {
        log::info!("Loaded memory card 1 from '{}'", save_writer.path.display());
        emulator_builder = emulator_builder.with_memory_card_1(memory_card_1);
    }

    let mut emulator = emulator_builder.build()?;

    let mut inputs = Ps1Inputs::default();

    let (mut audio_output, audio_stream) = create_audio_output()?;
    audio_stream.play()?;

    if let Some(exe_path) = &args.exe_path {
        log::info!("Sideloading EXE from '{exe_path}'");

        let exe = fs::read(exe_path)?;
        loop {
            emulator.tick(inputs, &mut renderer, &mut audio_output, &mut save_writer)?;
            if emulator.cpu_pc() == 0x80030000 {
                emulator.sideload_exe(&exe)?;
                log::info!("EXE sideloaded");
                break;
            }
        }
    }

    let mut paused = false;
    let mut step_to_next_frame = false;

    event_loop.set_control_flow(ControlFlow::Poll);

    event_loop.run(move |event, elwt| match event {
        Event::WindowEvent { event: WindowEvent::CloseRequested, .. } => {
            elwt.exit();
        }
        Event::WindowEvent {
            event: WindowEvent::KeyboardInput { event: key_event, .. }, ..
        } => {
            if let Err(err) = handle_key_event(
                HandleKeyEventArgs {
                    emulator: &mut emulator,
                    window: &window,
                    renderer: &mut renderer,
                    audio_output: &audio_output,
                    audio_stream: &audio_stream,
                    elwt,
                    inputs: &mut inputs,
                    display_config: &mut display_config,
                    save_state_path: &save_state_path,
                    paused: &mut paused,
                    step_to_next_frame: &mut step_to_next_frame,
                },
                key_event,
            ) {
                log::error!("Error handling key press: {err}");
            };
        }
        Event::WindowEvent { event: WindowEvent::Resized(size), .. } => {
            renderer.handle_resize(size.width, size.height);
        }
        Event::AboutToWait => {
            if !step_to_next_frame
                && (paused
                    || (args.audio_sync && audio_output.audio_queue.lock().unwrap().len() >= 2400))
            {
                elwt.set_control_flow(ControlFlow::WaitUntil(
                    Instant::now() + Duration::from_millis(1),
                ));
                return;
            }

            loop {
                match emulator.tick(inputs, &mut renderer, &mut audio_output, &mut save_writer) {
                    Ok(TickEffect::None) => {}
                    Ok(TickEffect::FrameRendered) => {
                        step_to_next_frame = false;
                        break;
                    }
                    Err(err) => {
                        log::error!("Emulator error, terminating: {err}");
                        elwt.exit();
                        break;
                    }
                }
            }

            elwt.set_control_flow(ControlFlow::Poll);
        }
        _ => {}
    })?;

    Ok(())
}
