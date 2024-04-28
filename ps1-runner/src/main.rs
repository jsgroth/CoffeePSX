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
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, KeyEvent, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowId};

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
    #[arg(long = "no-vsync", default_value_t = true, action = clap::ArgAction::SetFalse)]
    video_sync: bool,
    #[arg(long = "no-audio-sync", default_value_t = true, action = clap::ArgAction::SetFalse)]
    audio_sync: bool,
    #[arg(long = "no-simd", default_value_t = true)]
    simd: bool,
}

impl Args {
    fn present_mode(&self) -> wgpu::PresentMode {
        if self.video_sync { wgpu::PresentMode::Fifo } else { wgpu::PresentMode::Mailbox }
    }
}

const AUDIO_SYNC_THRESHOLD: usize = 2400;

struct CpalAudioOutput {
    audio_queue: Arc<Mutex<VecDeque<(f64, f64)>>>,
}

impl AudioOutput for CpalAudioOutput {
    type Err = anyhow::Error;

    fn queue_samples(&mut self, samples: &[(f64, f64)]) -> Result<(), Self::Err> {
        let mut audio_queue = self.audio_queue.lock().unwrap();

        if audio_queue.len() >= AUDIO_SYNC_THRESHOLD {
            // Drop samples; this should only happen if audio sync is disabled
            return Ok(());
        }

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

struct ApplicationState<Stream> {
    emulator: Ps1Emulator,
    renderer: WgpuRenderer,
    audio_output: CpalAudioOutput,
    audio_stream: Stream,
    audio_sync: bool,
    inputs: Ps1Inputs,
    save_writer: FsSaveWriter,
    display_config: DisplayConfig,
    save_state_path: PathBuf,
    paused: bool,
    step_to_next_frame: bool,
    // SAFETY: The window must outlive the WgpuRenderer
    window: Window,
}

macro_rules! bincode_config {
    () => {
        bincode::config::standard()
            .with_little_endian()
            .with_fixed_int_encoding()
            .with_limit::<1_000_000_000>()
    };
}

impl<Stream: StreamTrait> ApplicationState<Stream> {
    fn handle_key_event(
        &mut self,
        event: KeyEvent,
        event_loop: &ActiveEventLoop,
    ) -> anyhow::Result<()> {
        let pressed = event.state == ElementState::Pressed;

        match event.physical_key {
            PhysicalKey::Code(keycode) => match keycode {
                KeyCode::ArrowUp => self.inputs.p1.set_up(pressed),
                KeyCode::ArrowLeft => self.inputs.p1.set_left(pressed),
                KeyCode::ArrowRight => self.inputs.p1.set_right(pressed),
                KeyCode::ArrowDown => self.inputs.p1.set_down(pressed),
                KeyCode::KeyX => self.inputs.p1.set_cross(pressed),
                KeyCode::KeyS => self.inputs.p1.set_circle(pressed),
                KeyCode::KeyZ => self.inputs.p1.set_square(pressed),
                KeyCode::KeyA => self.inputs.p1.set_triangle(pressed),
                KeyCode::KeyW => self.inputs.p1.set_l1(pressed),
                KeyCode::KeyQ => self.inputs.p1.set_l2(pressed),
                KeyCode::KeyE => self.inputs.p1.set_r1(pressed),
                KeyCode::KeyR => self.inputs.p1.set_r2(pressed),
                KeyCode::Enter => self.inputs.p1.set_start(pressed),
                KeyCode::ShiftRight => self.inputs.p1.set_select(pressed),
                KeyCode::Escape if pressed => event_loop.exit(),
                KeyCode::F5 if pressed => self.save_state()?,
                KeyCode::F6 if pressed => self.load_state(),
                KeyCode::Slash if pressed => self.renderer.toggle_prescaling(),
                KeyCode::KeyP if pressed => self.toggle_pause()?,
                KeyCode::KeyN if pressed => self.step_to_next_frame = true,
                KeyCode::Semicolon if pressed => self.renderer.toggle_filter_mode(),
                KeyCode::Quote if pressed => {
                    self.display_config.dump_vram = !self.display_config.dump_vram;
                    self.emulator.update_display_config(self.display_config);

                    if self.display_config.dump_vram {
                        let _ = self.window.request_inner_size(LogicalSize::new(1024, 512));
                    } else {
                        let _ = self.window.request_inner_size(LogicalSize::new(586, 448));
                    }
                }
                KeyCode::Period if pressed => {
                    self.display_config.crop_vertical_overscan =
                        !self.display_config.crop_vertical_overscan;
                    self.emulator.update_display_config(self.display_config);
                }
                KeyCode::Minus if pressed => {
                    self.display_config.rasterizer_type = RasterizerType::SimdSoftware;
                    self.emulator.update_display_config(self.display_config);

                    log::info!("Using AVX2 software rasterizer");
                }
                KeyCode::Equal if pressed => {
                    self.display_config.rasterizer_type = RasterizerType::NaiveSoftware;
                    self.emulator.update_display_config(self.display_config);

                    log::info!("Using naive software rasterizer");
                }
                _ => {}
            },
            PhysicalKey::Unidentified(_) => {}
        }

        Ok(())
    }

    fn save_state(&mut self) -> anyhow::Result<()> {
        let file = File::create(&self.save_state_path)?;
        let mut writer = BufWriter::new(file);
        bincode::encode_into_std_write(self.emulator.to_state(), &mut writer, bincode_config!())?;

        log::info!("Saved state to '{}'", self.save_state_path.display());

        Ok(())
    }

    fn load_state(&mut self) {
        let file = match File::open(&self.save_state_path) {
            Ok(file) => file,
            Err(err) => {
                log::error!(
                    "Failed to open save state path at '{}': {err}",
                    self.save_state_path.display()
                );
                return;
            }
        };
        let mut reader = BufReader::new(file);

        match bincode::decode_from_std_read::<Ps1EmulatorState, _, _>(
            &mut reader,
            bincode_config!(),
        ) {
            Ok(loaded_state) => {
                let unserialized = self.emulator.take_unserialized_fields();
                self.emulator = Ps1Emulator::from_state(loaded_state, unserialized);
                self.emulator.update_display_config(self.display_config);

                log::info!("Loaded state from '{}'", self.save_state_path.display());
            }
            Err(err) => {
                log::error!(
                    "Failed to load save state from '{}': {err}",
                    self.save_state_path.display()
                );
            }
        }
    }

    fn toggle_pause(&mut self) -> anyhow::Result<()> {
        self.paused = !self.paused;

        self.audio_output.audio_queue.lock().unwrap().clear();

        if self.paused {
            self.audio_stream.pause()?;
        } else {
            self.audio_stream.play()?;
        }

        Ok(())
    }
}

impl<Stream: StreamTrait> ApplicationHandler for ApplicationState<Stream> {
    fn resumed(&mut self, _event_loop: &ActiveEventLoop) {}

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        if self.window.id() != window_id {
            return;
        }

        match event {
            WindowEvent::CloseRequested => {
                event_loop.exit();
            }
            WindowEvent::Resized(size) => {
                self.renderer.handle_resize(size.width, size.height);
            }
            WindowEvent::KeyboardInput { event: key_event, .. } => {
                if let Err(err) = self.handle_key_event(key_event, event_loop) {
                    log::error!("Error handling key presse event: {err}");
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if !self.step_to_next_frame
            && (self.paused
                || (self.audio_sync
                    && self.audio_output.audio_queue.lock().unwrap().len() >= AUDIO_SYNC_THRESHOLD))
        {
            event_loop.set_control_flow(ControlFlow::WaitUntil(
                Instant::now() + Duration::from_millis(1),
            ));
            return;
        }

        loop {
            match self.emulator.tick(
                self.inputs,
                &mut self.renderer,
                &mut self.audio_output,
                &mut self.save_writer,
            ) {
                Ok(TickEffect::None) => {}
                Ok(TickEffect::FrameRendered) => {
                    self.step_to_next_frame = false;
                    break;
                }
                Err(err) => {
                    log::error!("Emulator error, terminating: {err}");
                    event_loop.exit();
                    break;
                }
            }
        }

        event_loop.set_control_flow(ControlFlow::Poll);
    }
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
    #[allow(deprecated)]
    let window = event_loop.create_window(
        Window::default_attributes()
            .with_title(window_title)
            .with_inner_size(LogicalSize::new(586, 448)),
    )?;

    // SAFETY: The renderer does not outlive the window
    let mut renderer = pollster::block_on(unsafe {
        WgpuRenderer::new(
            &window,
            (window.inner_size().width, window.inner_size().height),
            args.present_mode(),
        )
    })?;

    let display_config = DisplayConfig {
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

    let (mut audio_output, audio_stream) = create_audio_output()?;
    audio_stream.play()?;

    if let Some(exe_path) = &args.exe_path {
        log::info!("Sideloading EXE from '{exe_path}'");

        let exe = fs::read(exe_path)?;
        loop {
            emulator.tick(
                Ps1Inputs::default(),
                &mut renderer,
                &mut audio_output,
                &mut save_writer,
            )?;
            if emulator.cpu_pc() == 0x80030000 {
                emulator.sideload_exe(&exe)?;
                log::info!("EXE sideloaded");
                break;
            }
        }
    }

    event_loop.set_control_flow(ControlFlow::Poll);

    event_loop.run_app(&mut ApplicationState {
        emulator,
        renderer,
        audio_output,
        audio_stream,
        audio_sync: args.audio_sync,
        inputs: Ps1Inputs::default(),
        save_writer,
        display_config,
        save_state_path,
        paused: false,
        step_to_next_frame: false,
        window,
    })?;

    Ok(())
}
