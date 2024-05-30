use crate::config::{AppConfig, VideoConfig};
use crate::emuthread::audio::{AudioQueue, QueueAudioCallback, QueueAudioOutput};
use crate::emuthread::renderer::{SurfaceRenderer, SwapChainRenderer};
use crate::Never;
use anyhow::{anyhow, Context};
use cdrom::reader::{CdRom, CdRomFileFormat};
use cfg_if::cfg_if;
use ps1_core::api::{
    DisplayConfig, Ps1Emulator, Ps1EmulatorBuilder, Ps1EmulatorState, SaveWriter, TickEffect,
    TickError,
};
use ps1_core::input::Ps1Inputs;
use sdl2::audio::AudioDevice;
use sdl2::AudioSubsystem;
use std::collections::VecDeque;
use std::ffi::OsStr;
use std::fs::File;
use std::io::{BufReader, BufWriter};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;
use std::{fs, io, thread};
use winit::dpi::PhysicalSize;

mod audio;
mod renderer;

const MEMORY_CARD_1_PATH: &str = "card1.mcd";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ps1Button {
    Up,
    Down,
    Left,
    Right,
    Cross,
    Circle,
    Square,
    Triangle,
    L1,
    L2,
    R1,
    R2,
    Start,
    Select,
}

#[derive(Debug)]
pub enum EmulatorThreadCommand {
    Stop,
    DigitalInput { button: Ps1Button, pressed: bool },
    UpdateConfig(AppConfig),
    SaveState,
    LoadState,
    TogglePause,
    StepFrame,
    FastForward { enabled: bool },
}

#[derive(Debug)]
struct FixedSizeDeque<const N: usize>(VecDeque<TextureWithAspectRatio>);

impl<const N: usize> FixedSizeDeque<N> {
    fn new() -> Self {
        assert_ne!(N, 0);

        Self(VecDeque::with_capacity(N))
    }

    #[must_use]
    fn push_back(&mut self, value: TextureWithAspectRatio) -> Option<wgpu::Texture> {
        let popped = (self.0.len() == N).then(|| self.0.pop_front().unwrap());
        self.0.push_back(value);
        popped.map(|TextureWithAspectRatio(texture, _)| texture)
    }

    #[must_use]
    fn pop_front(&mut self) -> Option<TextureWithAspectRatio> {
        self.0.pop_front()
    }

    fn clear(&mut self) {
        self.0.clear();
    }
}

pub const SWAP_CHAIN_LEN: usize = 3;

#[derive(Debug)]
struct TextureWithAspectRatio(wgpu::Texture, f64);

type SwapChainTextureBuffer = FixedSizeDeque<{ SWAP_CHAIN_LEN }>;

#[derive(Debug, Clone)]
pub struct EmulatorSwapChain {
    rendered_frames: Arc<Mutex<SwapChainTextureBuffer>>,
    returned_frames: Arc<Mutex<VecDeque<wgpu::Texture>>>,
    async_rendering: Arc<AtomicBool>,
}

impl EmulatorSwapChain {
    fn new(video_config: &VideoConfig) -> Self {
        Self {
            rendered_frames: Arc::new(Mutex::new(SwapChainTextureBuffer::new())),
            returned_frames: Arc::new(Mutex::new(VecDeque::with_capacity(SWAP_CHAIN_LEN))),
            async_rendering: Arc::new(AtomicBool::new(video_config.async_swap_chain_rendering)),
        }
    }

    fn update_config(&self, config: &VideoConfig) {
        self.async_rendering.store(config.async_swap_chain_rendering, Ordering::Relaxed);
    }
}

pub struct EmulationThreadHandle {
    swap_chain: EmulatorSwapChain,
    surface_renderer: SurfaceRenderer,
    audio_subsystem: AudioSubsystem,
    audio_queue: AudioQueue,
    audio_device: AudioDevice<QueueAudioCallback>,
    command_sender: Sender<EmulatorThreadCommand>,
}

impl EmulationThreadHandle {
    #[allow(clippy::missing_errors_doc)]
    pub fn spawn(
        file_path: Option<&Path>,
        config: &AppConfig,
        surface_config: &wgpu::SurfaceConfiguration,
        device: Arc<wgpu::Device>,
        queue: Arc<wgpu::Queue>,
    ) -> anyhow::Result<Self> {
        let Some(bios_path) = &config.paths.bios else {
            return Err(anyhow!("BIOS path is required to run emulator"));
        };

        let bios = fs::read(bios_path)
            .with_context(|| format!("Failed to read BIOS from '{}'", bios_path.display()))?;

        let display_config = DisplayConfig {
            crop_vertical_overscan: config.video.crop_vertical_overscan,
            dump_vram: config.video.vram_display,
            rasterizer_type: config.video.rasterizer.to_rasterizer_type(),
            hardware_resolution_scale: config.video.hardware_resolution_scale,
        };

        let mut builder = Ps1EmulatorBuilder::new(bios, Arc::clone(&device), Arc::clone(&queue))
            .with_display_config(display_config);

        if let Ok(card_data) = fs::read(MEMORY_CARD_1_PATH) {
            builder = builder.with_memory_card_1(card_data);
        }

        let emulator = match file_path {
            Some(file_path) => match file_path.extension().and_then(OsStr::to_str) {
                Some(extension @ ("cue" | "chd")) => {
                    let format = match extension {
                        "cue" => CdRomFileFormat::CueBin,
                        "chd" => CdRomFileFormat::Chd,
                        _ => unreachable!("nested match expressions"),
                    };

                    let disc = CdRom::open(file_path, format)?;
                    builder.with_disc(disc).build()?
                }
                Some("exe") => {
                    let exe = fs::read(file_path).with_context(|| {
                        format!("Failed to read EXE from path {}", file_path.display())
                    })?;

                    let mut emulator = builder.build()?;
                    emulator.run_until_exe_sideloaded(&exe)?;

                    emulator
                }
                Some(extension) => {
                    return Err(anyhow!("Unsupported file extension {extension}"));
                }
                None => {
                    return Err(anyhow!(
                        "Unable to determine file extension of '{}'",
                        file_path.display()
                    ));
                }
            },
            None => builder.build()?,
        };

        let swap_chain = EmulatorSwapChain::new(&config.video);
        let swap_chain_renderer =
            SwapChainRenderer::new(Arc::clone(&device), Arc::clone(&queue), swap_chain.clone());

        let sdl_ctx = sdl2::init().map_err(|err| anyhow!("Failed to initialize SDL2: {err}"))?;
        let audio_subsystem = sdl_ctx
            .audio()
            .map_err(|err| anyhow!("Failed to initialize SDL2 audio subsystem: {err}"))?;
        let audio_queue = Arc::new(Mutex::new(VecDeque::with_capacity(44100 / 30)));
        let audio_callback = QueueAudioCallback::new(Arc::clone(&audio_queue));

        let audio_spec = audio::new_spec(&config.audio);
        let audio_device = audio_subsystem
            .open_playback(None, &audio_spec, move |_| audio_callback)
            .map_err(|err| anyhow!("Failed to initialize SDL2 audio callback: {err}"))?;
        audio_device.resume();

        let audio_output = QueueAudioOutput::new(Arc::clone(&audio_queue));

        let (command_sender, command_receiver) = mpsc::channel();

        let save_state_path =
            file_path.map_or(PathBuf::from("bios.ss0"), |path| path.with_extension("ss0"));

        spawn_emu_thread(
            save_state_path,
            emulator,
            swap_chain_renderer,
            audio_output,
            config.audio.sync_threshold,
            command_receiver,
        );

        let surface_renderer = SurfaceRenderer::new(
            &config.video,
            Arc::clone(&device),
            Arc::clone(&queue),
            swap_chain.clone(),
            surface_config,
        );

        Ok(Self {
            swap_chain,
            surface_renderer,
            audio_subsystem,
            audio_queue,
            audio_device,
            command_sender,
        })
    }

    pub fn handle_resize(&mut self, size: PhysicalSize<u32>) {
        self.surface_renderer.handle_resize(size);
    }

    #[allow(clippy::missing_errors_doc)]
    pub fn handle_config_change(&mut self, config: &AppConfig) -> anyhow::Result<()> {
        self.swap_chain.update_config(&config.video);
        self.surface_renderer.update_config(&config.video);

        if config.audio.device_queue_size != self.audio_device.spec().samples {
            self.audio_device.pause();

            let audio_spec = audio::new_spec(&config.audio);
            let audio_callback = QueueAudioCallback::new(Arc::clone(&self.audio_queue));
            self.audio_device = self
                .audio_subsystem
                .open_playback(None, &audio_spec, move |_| audio_callback)
                .map_err(|err| anyhow!("Error recreating audio device: {err}"))?;
            self.audio_device.resume();
        }

        self.send_command(EmulatorThreadCommand::UpdateConfig(config.clone()));

        Ok(())
    }

    pub fn send_command(&self, command: EmulatorThreadCommand) {
        if matches!(command, EmulatorThreadCommand::Stop) {
            self.audio_device.pause();
        }

        if let Err(err) = self.command_sender.send(command) {
            log::error!("Failed to send command to emulator thread: {err}");
        }
    }

    pub fn swap_chain(&mut self) -> &mut EmulatorSwapChain {
        &mut self.swap_chain
    }

    #[allow(clippy::missing_errors_doc)]
    pub fn render_frame_if_available(&mut self, surface: &wgpu::Surface<'_>) -> anyhow::Result<()> {
        self.surface_renderer.render_frame_if_available(surface)
    }
}

fn spawn_emu_thread(
    save_state_path: PathBuf,
    mut emulator: Ps1Emulator,
    mut renderer: SwapChainRenderer,
    mut audio_output: QueueAudioOutput,
    mut audio_sync_threshold: u32,
    command_receiver: Receiver<EmulatorThreadCommand>,
) {
    thread::spawn(move || {
        let mut inputs = Ps1Inputs::default();
        let mut paused = false;
        let mut step_frame = false;
        let mut fast_forward = false;

        loop {
            // TODO configurable threshold
            if (!paused || step_frame)
                && (fast_forward || (audio_output.samples_len() as u32) < audio_sync_threshold)
            {
                if let Err(err) =
                    process_next_frame(inputs, &mut emulator, &mut renderer, &mut audio_output)
                {
                    log::error!("Video/audio/save write error: {err:?}");
                }

                step_frame = false;
            }

            if fast_forward && (audio_output.samples_len() as u32) >= 2 * audio_sync_threshold {
                audio_output.truncate_front(audio_sync_threshold as usize);
            }

            while let Ok(command) = command_receiver.try_recv() {
                match command {
                    EmulatorThreadCommand::Stop => {
                        log::info!("Stopping emulator thread");
                        return;
                    }
                    EmulatorThreadCommand::DigitalInput { button, pressed } => {
                        update_digital_inputs(&mut inputs, button, pressed);
                    }
                    EmulatorThreadCommand::UpdateConfig(config) => {
                        emulator.update_display_config(DisplayConfig {
                            crop_vertical_overscan: config.video.crop_vertical_overscan,
                            dump_vram: config.video.vram_display,
                            rasterizer_type: config.video.rasterizer.to_rasterizer_type(),
                            hardware_resolution_scale: config.video.hardware_resolution_scale,
                        });
                        audio_sync_threshold = config.audio.sync_threshold;
                    }
                    EmulatorThreadCommand::SaveState => {
                        if let Err(err) = save_state(&mut emulator, &save_state_path) {
                            log::info!("Error saving state: {err}");
                        }
                    }
                    EmulatorThreadCommand::LoadState => {
                        if let Err(err) = load_state(&mut emulator, &save_state_path) {
                            log::info!("Error loading state: {err}");
                        }
                    }
                    EmulatorThreadCommand::TogglePause => {
                        paused = !paused;
                    }
                    EmulatorThreadCommand::StepFrame => {
                        step_frame = true;
                    }
                    EmulatorThreadCommand::FastForward { enabled } => {
                        fast_forward = enabled;

                        // Clear swap chain when fast forward ends to prevent a temporary input
                        // latency increase due to buffered frames
                        if !fast_forward {
                            renderer.clear_swap_chain();
                        }
                    }
                }
            }

            sleep(Duration::from_millis(1));
        }
    });
}

fn process_next_frame(
    inputs: Ps1Inputs,
    emulator: &mut Ps1Emulator,
    renderer: &mut SwapChainRenderer,
    audio_output: &mut QueueAudioOutput,
) -> Result<(), TickError<Never, Never, io::Error>> {
    while emulator.tick(inputs, renderer, audio_output, &mut FsSaveWriter)?
        != TickEffect::FrameRendered
    {}

    Ok(())
}

macro_rules! impl_update_digital_inputs {
    ($inputs:expr, $input_button:expr, $pressed:expr, [$($button:ident => $setter:ident),* $(,)?]) => {
        match $input_button {
            $(
                Ps1Button::$button => $inputs.$setter($pressed),
            )*
        }
    }
}

fn update_digital_inputs(inputs: &mut Ps1Inputs, button: Ps1Button, pressed: bool) {
    impl_update_digital_inputs!(inputs.p1, button, pressed, [
        Up => set_up,
        Down => set_down,
        Left => set_left,
        Right => set_right,
        Cross => set_cross,
        Circle => set_circle,
        Square => set_square,
        Triangle => set_triangle,
        L1 => set_l1,
        L2 => set_l2,
        R1 => set_r1,
        R2 => set_r2,
        Start => set_start,
        Select => set_select,
    ]);
}

macro_rules! bincode_config {
    () => {
        bincode::config::standard()
            .with_little_endian()
            .with_fixed_int_encoding()
            .with_limit::<1_000_000_000>()
    };
}

fn save_state(emulator: &mut Ps1Emulator, path: &Path) -> anyhow::Result<()> {
    let file = File::create(path)?;
    let mut writer = BufWriter::new(file);
    bincode::encode_into_std_write(emulator.save_state(), &mut writer, bincode_config!())?;

    Ok(())
}

fn load_state(emulator: &mut Ps1Emulator, path: &Path) -> anyhow::Result<()> {
    let file = File::open(path)?;
    let mut reader = BufReader::new(file);
    let state: Ps1EmulatorState = bincode::decode_from_std_read(&mut reader, bincode_config!())?;

    *emulator = Ps1Emulator::from_state(state, emulator.take_unserialized_fields());

    Ok(())
}

fn sleep(duration: Duration) {
    cfg_if! {
        if #[cfg(target_os = "windows")] {
            unsafe {
                windows::Win32::Media::timeBeginPeriod(1);
                thread::sleep(duration);
                windows::Win32::Media::timeEndPeriod(1);
            }
        } else {
            thread::sleep(duration);
        }
    }
}

struct FsSaveWriter;

impl SaveWriter for FsSaveWriter {
    type Err = io::Error;

    fn save_memory_card_1(&mut self, card_data: &[u8]) -> Result<(), Self::Err> {
        fs::write(MEMORY_CARD_1_PATH, card_data)
    }
}
