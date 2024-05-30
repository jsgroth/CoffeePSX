use crate::config::{AppConfig, FilterMode, Rasterizer, VSyncMode, WgpuBackend};
use crate::{FileType, UserEvent};
use egui::{
    Button, CentralPanel, Color32, Context, Key, KeyboardShortcut, Modifiers, Slider, TextEdit,
    TopBottomPanel, Window,
};
use std::fs;
use std::path::{Path, PathBuf};
use winit::event_loop::EventLoopProxy;

struct AppState {
    video_window_open: bool,
    graphics_window_open: bool,
    audio_window_open: bool,
    paths_window_open: bool,
    audio_sync_threshold_text: String,
    audio_sync_threshold_invalid: bool,
    audio_device_queue_size_text: String,
    audio_device_queue_size_invalid: bool,
    last_serialized_config: AppConfig,
}

impl AppState {
    fn new(config: &AppConfig) -> Self {
        Self {
            video_window_open: false,
            graphics_window_open: false,
            audio_window_open: false,
            paths_window_open: false,
            audio_sync_threshold_text: config.audio.sync_threshold.to_string(),
            audio_sync_threshold_invalid: false,
            audio_device_queue_size_text: config.audio.device_queue_size.to_string(),
            audio_device_queue_size_invalid: false,
            last_serialized_config: config.clone(),
        }
    }
}

pub struct App {
    config_path: PathBuf,
    config: AppConfig,
    state: AppState,
}

impl App {
    #[must_use]
    pub fn new(config_path: PathBuf) -> Self {
        let config = read_config(&config_path).unwrap_or_else(|err| {
            log::warn!(
                "Unable to read config from '{}', using default: {err}",
                config_path.display()
            );
            AppConfig::default()
        });

        let state = AppState::new(&config);

        Self { config_path, config, state }
    }

    #[allow(clippy::single_match)]
    pub fn handle_event(&mut self, event: &UserEvent) {
        match event {
            UserEvent::FileOpened(FileType::BiosPath, Some(path)) => {
                self.config.paths.bios = Some(path.clone());
            }
            _ => {}
        }
    }

    #[allow(clippy::missing_panics_doc)]
    pub fn render(&mut self, ctx: &Context, proxy: &EventLoopProxy<UserEvent>) {
        self.render_menu(ctx, proxy);

        CentralPanel::default().show(ctx, |ui| {
            ui.centered_and_justified(|ui| {
                ui.label("TODO put something here");
            });
        });

        if self.state.video_window_open {
            self.render_video_window(ctx);
        }

        if self.state.graphics_window_open {
            self.render_graphics_window(ctx);
        }

        if self.state.audio_window_open {
            self.render_audio_window(ctx);
        }

        if self.state.paths_window_open {
            self.render_paths_window(ctx, proxy);
        }

        if self.config != self.state.last_serialized_config {
            if let Err(err) = self.serialize_config() {
                log::error!(
                    "Error serializing config file to '{}': {err}",
                    self.config_path.display()
                );
            }
            self.state.last_serialized_config = self.config.clone();

            proxy.send_event(UserEvent::AppConfigChanged).unwrap();
        }
    }

    fn render_menu(&mut self, ctx: &Context, proxy: &EventLoopProxy<UserEvent>) {
        let open_shortcut = KeyboardShortcut::new(Modifiers::CTRL, Key::O);
        if ctx.input_mut(|input| input.consume_shortcut(&open_shortcut)) {
            proxy
                .send_event(UserEvent::OpenFile { file_type: FileType::Open, initial_dir: None })
                .unwrap();
        }

        let quit_shortcut = KeyboardShortcut::new(Modifiers::CTRL, Key::Q);
        if ctx.input_mut(|input| input.consume_shortcut(&quit_shortcut)) {
            proxy.send_event(UserEvent::Close).unwrap();
        }

        TopBottomPanel::top("menu_panel").show(ctx, |ui| {
            egui::menu::bar(ui, |ui| {
                ui.menu_button("File", |ui| {
                    let open_button =
                        Button::new("Open").shortcut_text(ctx.format_shortcut(&open_shortcut));
                    if ui.add(open_button).clicked() {
                        proxy
                            .send_event(UserEvent::OpenFile {
                                file_type: FileType::Open,
                                initial_dir: None,
                            })
                            .unwrap();
                        ui.close_menu();
                    }

                    if ui.button("Run BIOS").clicked() {
                        proxy.send_event(UserEvent::RunBios).unwrap();
                        ui.close_menu();
                    }

                    let quit_button =
                        Button::new("Quit").shortcut_text(ctx.format_shortcut(&quit_shortcut));
                    if ui.add(quit_button).clicked() {
                        proxy.send_event(UserEvent::Close).unwrap();
                    }
                });

                ui.menu_button("Settings", |ui| {
                    if ui.button("Video").clicked() {
                        self.state.video_window_open = true;
                        ui.close_menu();
                    }

                    if ui.button("Graphics").clicked() {
                        self.state.graphics_window_open = true;
                        ui.close_menu();
                    }

                    if ui.button("Audio").clicked() {
                        self.state.audio_window_open = true;
                        ui.close_menu();
                    }

                    if ui.button("Paths").clicked() {
                        self.state.paths_window_open = true;
                        ui.close_menu();
                    }
                });
            });
        });
    }

    fn render_video_window(&mut self, ctx: &Context) {
        Window::new("Video Settings")
            .open(&mut self.state.video_window_open)
            .resizable(false)
            .show(ctx, |ui| {
                ui.group(|ui| {
                    ui.label("VSync mode");

                    ui.horizontal(|ui| {
                        ui.radio_value(
                            &mut self.config.video.vsync_mode,
                            VSyncMode::Enabled,
                            "Enabled",
                        )
                        .on_hover_text("wgpu Fifo present mode");
                        ui.radio_value(
                            &mut self.config.video.vsync_mode,
                            VSyncMode::Disabled,
                            "Disabled",
                        )
                        .on_hover_text("wgpu Immediate present mode");
                        ui.radio_value(&mut self.config.video.vsync_mode, VSyncMode::Fast, "Fast")
                            .on_hover_text("wgpu Mailbox present mode");
                    });
                });

                ui.group(|ui| {
                    ui.label("Image filtering");

                    ui.horizontal(|ui| {
                        ui.radio_value(
                            &mut self.config.video.filter_mode,
                            FilterMode::Linear,
                            "Bilinear interpolation",
                        );
                        ui.radio_value(
                            &mut self.config.video.filter_mode,
                            FilterMode::Nearest,
                            "Nearest neighbor",
                        );
                    });
                });

                ui.checkbox(
                    &mut self.config.video.crop_vertical_overscan,
                    "Crop vertical overscan",
                )
                .on_hover_text("Crop vertical display to 224px NTSC / 268px PAL");

                ui.checkbox(&mut self.config.video.vram_display, "VRAM display").on_hover_text(
                    "Display the entire contents of VRAM instead of only the current frame buffer",
                );
            });
    }

    fn render_graphics_window(&mut self, ctx: &Context) {
        Window::new("Graphics Settings")
            .open(&mut self.state.graphics_window_open)
            .resizable(false)
            .show(ctx, |ui| {
                ui.group(|ui| {
                    ui.label("Rasterizer");

                    ui.horizontal(|ui| {
                        ui.radio_value(
                            &mut self.config.video.rasterizer,
                            Rasterizer::Software,
                            "Software",
                        )
                        .on_hover_text("CPU-based; more accurate but no enhancements");
                        ui.radio_value(
                            &mut self.config.video.rasterizer,
                            Rasterizer::Hardware,
                            "Hardware (wgpu)",
                        )
                        .on_hover_text("GPU-based; supports enhancements but less accurate");
                    });
                });

                let is_hw_rasterizer = self.config.video.rasterizer == Rasterizer::Hardware;
                let disabled_hover_text = "Hardware rasterizer only";

                ui.add_enabled_ui(is_hw_rasterizer, |ui| {
                    ui.group(|ui| {
                        ui.label("wgpu backend (requires game restart)")
                            .on_disabled_hover_text(disabled_hover_text);

                        ui.horizontal(|ui| {
                            ui.radio_value(
                                &mut self.config.video.wgpu_backend,
                                WgpuBackend::Auto,
                                "Auto",
                            )
                            .on_disabled_hover_text(disabled_hover_text);
                            ui.radio_value(
                                &mut self.config.video.wgpu_backend,
                                WgpuBackend::Vulkan,
                                "Vulkan",
                            )
                            .on_disabled_hover_text(disabled_hover_text);
                            ui.radio_value(
                                &mut self.config.video.wgpu_backend,
                                WgpuBackend::DirectX12,
                                "DirectX 12",
                            )
                            .on_disabled_hover_text(disabled_hover_text);
                            ui.radio_value(
                                &mut self.config.video.wgpu_backend,
                                WgpuBackend::Metal,
                                "Metal",
                            )
                            .on_disabled_hover_text(disabled_hover_text);
                        });
                    });

                    ui.horizontal(|ui| {
                        ui.label("Resolution scale:").on_disabled_hover_text(disabled_hover_text);

                        ui.add(Slider::new(
                            &mut self.config.video.hardware_resolution_scale,
                            1..=16,
                        ))
                        .on_disabled_hover_text(disabled_hover_text);
                    });
                });

                ui.checkbox(
                    &mut self.config.video.async_swap_chain_rendering,
                    "Asynchronous rendering",
                )
                .on_hover_text("Can improve performance but can also cause skipped frames and increased input latency")
                .on_disabled_hover_text(disabled_hover_text);
            });
    }

    fn render_audio_window(&mut self, ctx: &Context) {
        Window::new("Audio Settings")
            .open(&mut self.state.audio_window_open)
            .resizable(false)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    let hover_text =
                        "Higher values reduce audio stutters but increase audio latency";

                    if ui
                        .add(
                            TextEdit::singleline(&mut self.state.audio_sync_threshold_text)
                                .desired_width(30.0),
                        )
                        .on_hover_text(hover_text)
                        .changed()
                    {
                        match self.state.audio_sync_threshold_text.parse::<u32>() {
                            Ok(value) if value != 0 => {
                                self.config.audio.sync_threshold = value;
                                self.state.audio_sync_threshold_invalid = false;
                            }
                            _ => {
                                self.state.audio_sync_threshold_invalid = true;
                            }
                        }
                    }

                    ui.label("Audio sync threshold (samples)").on_hover_text(hover_text);
                });

                if self.state.audio_sync_threshold_invalid {
                    ui.colored_label(
                        Color32::RED,
                        "Audio sync threshold must be a non-negative integer",
                    );
                }

                ui.horizontal(|ui| {
                    if ui
                        .add(
                            TextEdit::singleline(&mut self.state.audio_device_queue_size_text)
                                .desired_width(30.0),
                        )
                        .changed()
                    {
                        match self.state.audio_device_queue_size_text.parse::<u16>() {
                            Ok(value) if value >= 8 && value.count_ones() == 1 => {
                                self.config.audio.device_queue_size = value;
                                self.state.audio_device_queue_size_invalid = false;
                            }
                            _ => {
                                self.state.audio_device_queue_size_invalid = true;
                            }
                        }
                    }

                    ui.label("Audio device queue size (samples)");
                });

                if self.state.audio_device_queue_size_invalid {
                    ui.colored_label(
                        Color32::RED,
                        "Audio device queue size must be a power of two",
                    );
                }
            });
    }

    fn render_paths_window(&mut self, ctx: &Context, proxy: &EventLoopProxy<UserEvent>) {
        Window::new("Paths Settings")
            .open(&mut self.state.paths_window_open)
            .resizable(false)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    let button_text = self
                        .config
                        .paths
                        .bios
                        .as_ref()
                        .and_then(|path| path.to_str())
                        .unwrap_or("<None>");
                    if ui.button(button_text).clicked() {
                        let initial_dir = self
                            .config
                            .paths
                            .bios
                            .as_ref()
                            .and_then(|path| path.parent())
                            .map(PathBuf::from);

                        proxy
                            .send_event(UserEvent::OpenFile {
                                file_type: FileType::BiosPath,
                                initial_dir,
                            })
                            .unwrap();
                    }

                    ui.label("BIOS path");
                });
            });
    }

    fn serialize_config(&mut self) -> anyhow::Result<()> {
        let config_str = toml::to_string_pretty(&self.config)?;
        fs::write(&self.config_path, config_str)?;

        log::debug!("Serialized config file to '{}'", self.config_path.display());

        Ok(())
    }

    pub fn config_mut(&mut self) -> &mut AppConfig {
        &mut self.config
    }
}

fn read_config<P: AsRef<Path>>(path: P) -> anyhow::Result<AppConfig> {
    let path = path.as_ref();

    let config_str = fs::read_to_string(path)?;
    let config: AppConfig = toml::from_str(&config_str)?;

    Ok(config)
}
