use crate::config::{AppConfig, Rasterizer, VideoConfig};
use crate::emuthread::{EmulationThreadHandle, EmulatorThreadCommand, Ps1Button};
use crate::{OpenFileType, UserEvent};
use anyhow::anyhow;
use std::cmp;
use std::ffi::OsStr;
use std::path::Path;
use std::sync::Arc;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, Event, KeyEvent, WindowEvent};
use winit::event_loop::EventLoopWindowTarget;
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Fullscreen, Window, WindowBuilder};

#[derive(Debug)]
struct EmulatorWindow {
    surface: wgpu::Surface<'static>,
    surface_config: wgpu::SurfaceConfiguration,
    supported_present_modes: Vec<wgpu::PresentMode>,
    device: Arc<wgpu::Device>,
    queue: Arc<wgpu::Queue>,
    // SAFETY: The window must be dropped after the surface
    window: Window,
}

impl EmulatorWindow {
    fn new(
        file_path: Option<&Path>,
        elwt: &EventLoopWindowTarget<UserEvent>,
        config: &AppConfig,
    ) -> anyhow::Result<Self> {
        let window_title = match file_path {
            Some(file_path) => file_path
                .with_extension("")
                .file_name()
                .and_then(OsStr::to_str)
                .unwrap_or("PS1")
                .to_string(),
            None => "(BIOS)".into(),
        };
        let window_size = LogicalSize::new(config.video.window_width, config.video.window_height);
        let mut window_builder =
            WindowBuilder::new().with_title(window_title).with_inner_size(window_size);
        if config.video.launch_in_fullscreen {
            window_builder = window_builder.with_fullscreen(Some(Fullscreen::Borderless(None)));
        }
        let window = window_builder.build(elwt)?;

        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: config.graphics.wgpu_backend.to_wgpu(),
            ..wgpu::InstanceDescriptor::default()
        });

        // SAFETY: The surface must not outlive the window
        let surface = unsafe {
            instance.create_surface_unsafe(wgpu::SurfaceTargetUnsafe::from_window(&window)?)
        }?;

        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            force_fallback_adapter: false,
            compatible_surface: Some(&surface),
        }))
        .ok_or_else(|| anyhow!("Unable to obtain wgpu adapter for emulator window"))?;

        let (device, queue) = pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label: "emulator_device".into(),
                required_features: ps1_core::required_wgpu_features(),
                required_limits: ps1_core::required_wgpu_limits(),
            },
            None,
        ))?;

        let surface_capabilities = surface.get_capabilities(&adapter);

        let present_mode = config.video.vsync_mode.to_present_mode();
        if !surface_capabilities.present_modes.contains(&present_mode) {
            return Err(anyhow!(
                "wgpu surface does not support requested VSync mode {:?}",
                config.video.vsync_mode
            ));
        }

        let surface_format = surface_capabilities
            .formats
            .iter()
            .copied()
            .find(wgpu::TextureFormat::is_srgb)
            .unwrap_or_else(|| {
                let format = surface_capabilities
                    .formats
                    .first()
                    .copied()
                    .unwrap_or(wgpu::TextureFormat::Bgra8Unorm);
                log::error!("Surface does not support any sRGB texture formats; using {format:?}");
                format
            });

        log::info!("Using texture format {surface_format:?} in emulator window");

        let surface_config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: surface_format,
            width: window.inner_size().width,
            height: window.inner_size().height,
            present_mode: config.video.vsync_mode.to_present_mode(),
            desired_maximum_frame_latency: 2,
            alpha_mode: wgpu::CompositeAlphaMode::default(),
            view_formats: vec![],
        };
        surface.configure(&device, &surface_config);

        Ok(Self {
            surface,
            surface_config,
            supported_present_modes: surface_capabilities.present_modes,
            device: Arc::new(device),
            queue: Arc::new(queue),
            window,
        })
    }

    pub fn update_config(&mut self, video_config: &VideoConfig) {
        let present_mode = video_config.vsync_mode.to_present_mode();
        if !self.supported_present_modes.contains(&present_mode) {
            log::error!(
                "wgpu present mode {present_mode:?} is not supported; not changing VSync mode"
            );
            return;
        }

        self.surface_config.present_mode = present_mode;
        self.surface.configure(&self.device, &self.surface_config);
    }

    fn toggle_fullscreen(&self) {
        let new_fullscreen = match self.window.fullscreen() {
            Some(_) => None,
            None => Some(Fullscreen::Borderless(None)),
        };
        self.window.set_fullscreen(new_fullscreen);
    }
}

struct RunningState {
    window: EmulatorWindow,
    emu_thread: EmulationThreadHandle,
}

pub struct EmulatorState {
    running: Option<RunningState>,
}

impl EmulatorState {
    #[must_use]
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        Self { running: None }
    }

    #[allow(clippy::missing_errors_doc)]
    pub fn handle_event(
        &mut self,
        event: &Event<UserEvent>,
        elwt: &EventLoopWindowTarget<UserEvent>,
        app_config: &mut AppConfig,
    ) -> anyhow::Result<()> {
        match event {
            Event::UserEvent(UserEvent::FileOpened(OpenFileType::Open, Some(file_path))) => {
                return self.start_emulator(Some(file_path), elwt, app_config);
            }
            Event::UserEvent(UserEvent::RunBios) => {
                return self.start_emulator(None, elwt, app_config);
            }
            _ => {}
        }

        let Some(RunningState { window, emu_thread }) = &mut self.running else { return Ok(()) };

        match event {
            Event::UserEvent(UserEvent::AppConfigChanged) => {
                window.update_config(&app_config.video);
                emu_thread.handle_config_change(app_config)?;
            }
            Event::WindowEvent { event: win_event, window_id }
                if *window_id == window.window.id() =>
            {
                match win_event {
                    WindowEvent::CloseRequested => {
                        emu_thread.send_command(EmulatorThreadCommand::Stop);
                        self.running = None;
                    }
                    WindowEvent::Resized(size) => {
                        window.surface_config.width = size.width;
                        window.surface_config.height = size.height;
                        window.surface.configure(&window.device, &window.surface_config);

                        match window.window.fullscreen() {
                            Some(_) => {
                                window.window.set_cursor_visible(false);
                            }
                            None => {
                                let logical_size = size.to_logical(window.window.scale_factor());
                                app_config.video.window_width = logical_size.width;
                                app_config.video.window_height = logical_size.height;

                                window.window.set_cursor_visible(true);
                            }
                        }

                        emu_thread.handle_resize(*size);
                    }
                    &WindowEvent::KeyboardInput {
                        event: KeyEvent { physical_key, state, .. },
                        ..
                    } => {
                        if let Some(command) = key_input_command(physical_key, state) {
                            emu_thread.send_command(command);
                        }

                        let hotkey = check_hotkey(physical_key, state);
                        match hotkey {
                            Some(Hotkey::Quit) => {
                                emu_thread.send_command(EmulatorThreadCommand::Stop);
                                self.running = None;
                            }
                            Some(Hotkey::ToggleFullscreen) => {
                                window.toggle_fullscreen();
                            }
                            Some(Hotkey::ToggleVramDisplay) => {
                                app_config.video.vram_display = !app_config.video.vram_display;
                                emu_thread.send_command(EmulatorThreadCommand::UpdateConfig(
                                    app_config.clone(),
                                ));
                            }
                            Some(Hotkey::EnableHardwareRasterizer) => {
                                app_config.graphics.rasterizer = Rasterizer::Hardware;
                                emu_thread.send_command(EmulatorThreadCommand::UpdateConfig(
                                    app_config.clone(),
                                ));
                                log::info!(
                                    "Using hardware rasterizer with resolution scale {}",
                                    app_config.graphics.hardware_resolution_scale
                                );
                            }
                            Some(Hotkey::EnableSoftwareRasterizer) => {
                                app_config.graphics.rasterizer = Rasterizer::Software;
                                emu_thread.send_command(EmulatorThreadCommand::UpdateConfig(
                                    app_config.clone(),
                                ));
                                log::info!("Using software rasterizer");
                            }
                            Some(Hotkey::DecreaseResolutionScale) => {
                                let scale =
                                    cmp::max(1, app_config.graphics.hardware_resolution_scale - 1);
                                app_config.graphics.hardware_resolution_scale = scale;
                                emu_thread.send_command(EmulatorThreadCommand::UpdateConfig(
                                    app_config.clone(),
                                ));
                                log::info!("Set resolution scale to {scale}");
                            }
                            Some(Hotkey::IncreaseResolutionScale) => {
                                let scale =
                                    cmp::min(16, app_config.graphics.hardware_resolution_scale + 1);
                                app_config.graphics.hardware_resolution_scale = scale;
                                emu_thread.send_command(EmulatorThreadCommand::UpdateConfig(
                                    app_config.clone(),
                                ));
                                log::info!("Set resolution scale to {scale}");
                            }
                            Some(Hotkey::SaveState) => {
                                emu_thread.send_command(EmulatorThreadCommand::SaveState);
                            }
                            Some(Hotkey::LoadState) => {
                                emu_thread.send_command(EmulatorThreadCommand::LoadState);
                            }
                            Some(Hotkey::Pause) => {
                                emu_thread.send_command(EmulatorThreadCommand::TogglePause);
                            }
                            Some(Hotkey::StepFrame) => {
                                emu_thread.send_command(EmulatorThreadCommand::StepFrame);
                            }
                            Some(Hotkey::FastForward) => {
                                emu_thread.send_command(EmulatorThreadCommand::FastForward {
                                    enabled: state == ElementState::Pressed,
                                });
                            }
                            None => {}
                        }
                    }
                    _ => {}
                }
            }
            Event::AboutToWait => {
                emu_thread.render_frame_if_available(&window.surface)?;
            }
            _ => {}
        }

        Ok(())
    }

    fn start_emulator(
        &mut self,
        file_path: Option<&Path>,
        elwt: &EventLoopWindowTarget<UserEvent>,
        app_config: &AppConfig,
    ) -> anyhow::Result<()> {
        if let Some(RunningState { emu_thread, .. }) = &self.running {
            emu_thread.send_command(EmulatorThreadCommand::Stop);
        }

        let window = EmulatorWindow::new(file_path, elwt, app_config)?;

        let emu_thread = EmulationThreadHandle::spawn(
            file_path,
            app_config,
            &window.surface_config,
            Arc::clone(&window.device),
            Arc::clone(&window.queue),
        )?;

        self.running = Some(RunningState { window, emu_thread });

        Ok(())
    }

    pub fn is_emulator_running(&self) -> bool {
        self.running.is_some()
    }
}

fn key_input_command(key: PhysicalKey, state: ElementState) -> Option<EmulatorThreadCommand> {
    let PhysicalKey::Code(keycode) = key else { return None };
    let pressed = state == ElementState::Pressed;

    // TODO configurable
    let button = match keycode {
        KeyCode::ArrowUp => Ps1Button::Up,
        KeyCode::ArrowDown => Ps1Button::Down,
        KeyCode::ArrowLeft => Ps1Button::Left,
        KeyCode::ArrowRight => Ps1Button::Right,
        KeyCode::KeyX => Ps1Button::Cross,
        KeyCode::KeyS => Ps1Button::Circle,
        KeyCode::KeyZ => Ps1Button::Square,
        KeyCode::KeyA => Ps1Button::Triangle,
        KeyCode::KeyW => Ps1Button::L1,
        KeyCode::KeyQ => Ps1Button::L2,
        KeyCode::KeyE => Ps1Button::R1,
        KeyCode::KeyR => Ps1Button::R2,
        KeyCode::Enter => Ps1Button::Start,
        KeyCode::ShiftRight => Ps1Button::Select,
        _ => return None,
    };

    Some(EmulatorThreadCommand::DigitalInput { button, pressed })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Hotkey {
    Quit,
    ToggleFullscreen,
    ToggleVramDisplay,
    EnableHardwareRasterizer,
    EnableSoftwareRasterizer,
    DecreaseResolutionScale,
    IncreaseResolutionScale,
    SaveState,
    LoadState,
    Pause,
    StepFrame,
    FastForward,
}

fn check_hotkey(key: PhysicalKey, state: ElementState) -> Option<Hotkey> {
    let PhysicalKey::Code(keycode) = key else { return None };
    let pressed = state == ElementState::Pressed;

    // TODO configurable
    match keycode {
        KeyCode::Escape if pressed => Some(Hotkey::Quit),
        KeyCode::F9 if pressed => Some(Hotkey::ToggleFullscreen),
        KeyCode::Quote if pressed => Some(Hotkey::ToggleVramDisplay),
        KeyCode::Digit0 if pressed => Some(Hotkey::EnableHardwareRasterizer),
        KeyCode::Minus if pressed => Some(Hotkey::EnableSoftwareRasterizer),
        KeyCode::BracketLeft if pressed => Some(Hotkey::DecreaseResolutionScale),
        KeyCode::BracketRight if pressed => Some(Hotkey::IncreaseResolutionScale),
        KeyCode::F5 if pressed => Some(Hotkey::SaveState),
        KeyCode::F6 if pressed => Some(Hotkey::LoadState),
        KeyCode::KeyP if pressed => Some(Hotkey::Pause),
        KeyCode::KeyN if pressed => Some(Hotkey::StepFrame),
        KeyCode::Tab => Some(Hotkey::FastForward),
        _ => None,
    }
}
