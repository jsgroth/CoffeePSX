use crate::config::{AppConfig, Rasterizer, VideoConfig};
use crate::emuthread::{EmulationThreadHandle, EmulatorThreadCommand, Ps1AnalogInput, Ps1Button};
use crate::{OpenFileType, UserEvent};
use anyhow::anyhow;
use sdl2::controller::Axis as SdlAxis;
use sdl2::controller::Button as SdlButton;
use sdl2::controller::GameController;
use sdl2::event::Event as SdlEvent;
use sdl2::{EventPump, GameControllerSubsystem, Sdl};
use std::cmp;
use std::collections::HashMap;
use std::ffi::OsStr;
use std::path::Path;
use std::sync::Arc;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, Event, KeyEvent, WindowEvent};
use winit::event_loop::{EventLoopProxy, EventLoopWindowTarget};
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
            dx12_shader_compiler: wgpu::Dx12Compiler::Dxc {
                dxil_path: Some("dxil.dll".into()),
                dxc_path: Some("dxcompiler.dll".into()),
            },
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

struct Controllers {
    subsystem: GameControllerSubsystem,
    controllers: HashMap<u32, GameController>,
    instance_id_to_device_id: HashMap<u32, u32>,
}

impl Controllers {
    fn new(subsystem: GameControllerSubsystem) -> Self {
        Self { subsystem, controllers: HashMap::new(), instance_id_to_device_id: HashMap::new() }
    }

    fn handle_device_added(&mut self, which: u32) -> anyhow::Result<()> {
        let controller = self.subsystem.open(which)?;

        log::info!("Controller added: '{}'", controller.name());

        self.instance_id_to_device_id.insert(controller.instance_id(), which);
        self.controllers.insert(which, controller);

        Ok(())
    }

    fn handle_device_removed(&mut self, which: u32) {
        let Some(device_id) = self.instance_id_to_device_id.remove(&which) else { return };
        let Some(controller) = self.controllers.remove(&device_id) else { return };

        log::info!("Controller removed: '{}'", controller.name());
    }

    #[allow(clippy::unused_self)]
    fn handle_button_press(
        &self,
        button: SdlButton,
        pressed: bool,
        proxy: &EventLoopProxy<UserEvent>,
    ) {
        let ps1_button = match button {
            SdlButton::DPadUp => Ps1Button::Up,
            SdlButton::DPadLeft => Ps1Button::Left,
            SdlButton::DPadRight => Ps1Button::Right,
            SdlButton::DPadDown => Ps1Button::Down,
            SdlButton::A => Ps1Button::Cross,
            SdlButton::B => Ps1Button::Circle,
            SdlButton::X => Ps1Button::Square,
            SdlButton::Y => Ps1Button::Triangle,
            SdlButton::LeftShoulder => Ps1Button::L1,
            SdlButton::RightShoulder => Ps1Button::R1,
            SdlButton::Start => Ps1Button::Start,
            SdlButton::Back => Ps1Button::Select,
            SdlButton::Guide => Ps1Button::Analog,
            _ => return,
        };

        proxy.send_event(UserEvent::ControllerButton { button: ps1_button, pressed }).unwrap();
    }

    #[allow(clippy::unused_self)]
    fn handle_axis_motion(&self, axis: SdlAxis, value: i16, proxy: &EventLoopProxy<UserEvent>) {
        let ps1_axis = match axis {
            SdlAxis::LeftX => Some(Ps1AnalogInput::LeftStickX),
            SdlAxis::LeftY => Some(Ps1AnalogInput::LeftStickY),
            SdlAxis::RightX => Some(Ps1AnalogInput::RightStickX),
            SdlAxis::RightY => Some(Ps1AnalogInput::RightStickY),
            _ => None,
        };
        if let Some(ps1_axis) = ps1_axis {
            proxy.send_event(UserEvent::ControllerAnalog { input: ps1_axis, value }).unwrap();
            return;
        }

        let button = match axis {
            SdlAxis::TriggerLeft => Ps1Button::L2,
            SdlAxis::TriggerRight => Ps1Button::R2,
            _ => return,
        };

        let pressed = value >= i16::MAX / 2;
        proxy.send_event(UserEvent::ControllerButton { button, pressed }).unwrap();
    }
}

pub struct EmulatorState {
    running: Option<RunningState>,
    sdl_ctx: Sdl,
    sdl_event_pump: EventPump,
    controllers: Controllers,
}

impl EmulatorState {
    #[allow(clippy::new_without_default)]
    #[allow(clippy::missing_errors_doc)]
    pub fn new() -> anyhow::Result<Self> {
        let sdl_ctx = sdl2::init().map_err(|err| anyhow!("Error initializing SDL2: {err}"))?;
        let sdl_event_pump = sdl_ctx
            .event_pump()
            .map_err(|err| anyhow!("Error initializing SDL2 event pump: {err}"))?;
        let controller_subsystem = sdl_ctx
            .game_controller()
            .map_err(|err| anyhow!("Error initializing SDL2 game controller subsystem: {err}"))?;
        let controllers = Controllers::new(controller_subsystem);

        Ok(Self { running: None, sdl_ctx, sdl_event_pump, controllers })
    }

    #[allow(clippy::missing_errors_doc)]
    pub fn handle_event(
        &mut self,
        event: &Event<UserEvent>,
        elwt: &EventLoopWindowTarget<UserEvent>,
        proxy: &EventLoopProxy<UserEvent>,
        app_config: &mut AppConfig,
    ) -> anyhow::Result<()> {
        match event {
            Event::UserEvent(UserEvent::FileOpened(OpenFileType::Open, Some(file_path))) => {
                return self.start_emulator(Some(file_path), elwt, app_config);
            }
            Event::UserEvent(UserEvent::RunBios) => {
                return self.start_emulator(None, elwt, app_config);
            }
            Event::AboutToWait => {
                self.process_sdl_events(proxy)?;
            }
            _ => {}
        }

        let Some(RunningState { window, emu_thread }) = &mut self.running else { return Ok(()) };

        match event {
            Event::UserEvent(UserEvent::AppConfigChanged) => {
                window.update_config(&app_config.video);
                emu_thread.handle_config_change(app_config)?;
            }
            &Event::UserEvent(UserEvent::ControllerButton { button, pressed }) => {
                emu_thread.send_command(EmulatorThreadCommand::DigitalInput { button, pressed });
            }
            &Event::UserEvent(UserEvent::ControllerAnalog { input, value }) => {
                emu_thread.send_command(EmulatorThreadCommand::AnalogInput { input, value });
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
                                app_config.debug.vram_display = !app_config.debug.vram_display;
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

    fn process_sdl_events(&mut self, proxy: &EventLoopProxy<UserEvent>) -> anyhow::Result<()> {
        for event in self.sdl_event_pump.poll_iter() {
            match event {
                SdlEvent::ControllerDeviceAdded { which, .. } => {
                    self.controllers.handle_device_added(which)?;
                }
                SdlEvent::ControllerDeviceRemoved { which, .. } => {
                    self.controllers.handle_device_removed(which);
                }
                SdlEvent::ControllerButtonDown { button, .. } => {
                    self.controllers.handle_button_press(button, true, proxy);
                }
                SdlEvent::ControllerButtonUp { button, .. } => {
                    self.controllers.handle_button_press(button, false, proxy);
                }
                SdlEvent::ControllerAxisMotion { axis, value, .. } => {
                    self.controllers.handle_axis_motion(axis, value, proxy);
                }
                _ => {}
            }
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
            &self.sdl_ctx,
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
        KeyCode::KeyY => Ps1Button::Analog,
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
