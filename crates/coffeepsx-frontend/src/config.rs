pub mod input;

use crate::config::input::ControllerConfig;
use anyhow::anyhow;
use cfg_if::cfg_if;
use ps1_core::RasterizerType;
use ps1_core::api::{
    AdpcmInterpolation, DisplayConfig, MemoryCardSlot, MemoryCardsEnabled, PgxpConfig,
    Ps1EmulatorConfig,
};
use ps1_core::input::ControllerType;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::ffi::OsStr;
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum VSyncMode {
    #[default]
    Enabled,
    Disabled,
    Fast,
}

impl VSyncMode {
    #[must_use]
    pub const fn to_present_mode(self) -> wgpu::PresentMode {
        match self {
            Self::Enabled => wgpu::PresentMode::Fifo,
            Self::Disabled => wgpu::PresentMode::Immediate,
            Self::Fast => wgpu::PresentMode::Mailbox,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum AspectRatio {
    #[default]
    Native,
    Stretched,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum FilterMode {
    #[default]
    Linear,
    Nearest,
}

impl FilterMode {
    #[must_use]
    pub fn to_wgpu(self) -> wgpu::FilterMode {
        match self {
            Self::Linear => wgpu::FilterMode::Linear,
            Self::Nearest => wgpu::FilterMode::Nearest,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Rasterizer {
    #[default]
    Software,
    Hardware,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum WgpuBackend {
    #[default]
    Auto,
    Vulkan,
    DirectX12,
    Metal,
}

impl WgpuBackend {
    #[must_use]
    pub fn to_wgpu(self) -> wgpu::Backends {
        match self {
            Self::Auto => wgpu::Backends::VULKAN | wgpu::Backends::DX12 | wgpu::Backends::METAL,
            Self::Vulkan => wgpu::Backends::VULKAN,
            Self::DirectX12 => wgpu::Backends::DX12,
            Self::Metal => wgpu::Backends::METAL,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VideoConfig {
    #[serde(default)]
    pub launch_in_fullscreen: bool,
    #[serde(default)]
    pub vsync_mode: VSyncMode,
    #[serde(default)]
    pub aspect_ratio: AspectRatio,
    #[serde(default)]
    pub filter_mode: FilterMode,
    #[serde(default = "true_fn")]
    pub crop_vertical_overscan: bool,
    #[serde(default = "default_window_width")]
    pub window_width: u32,
    #[serde(default = "default_window_height")]
    pub window_height: u32,
}

fn true_fn() -> bool {
    true
}

fn default_window_width() -> u32 {
    586
}

fn default_window_height() -> u32 {
    448
}

impl Default for VideoConfig {
    fn default() -> Self {
        toml::from_str("").unwrap()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphicsConfig {
    #[serde(default)]
    pub rasterizer: Rasterizer,
    #[serde(default = "true_fn")]
    pub avx2_software_rasterizer: bool,
    #[serde(default)]
    pub wgpu_backend: WgpuBackend,
    #[serde(default = "default_resolution_scale")]
    pub hardware_resolution_scale: u32,
    #[serde(default)]
    pub hardware_high_color: bool,
    #[serde(default = "true_fn")]
    pub hardware_15bpp_dithering: bool,
    #[serde(default = "true_fn")]
    pub high_res_dithering: bool,
    #[serde(default)]
    pub async_swap_chain_rendering: bool,
    #[serde(default)]
    pub pgxp_enabled: bool,
    #[serde(default = "true_fn")]
    pub pgxp_precise_culling: bool,
    #[serde(default = "true_fn")]
    pub pgxp_perspective_texture_mapping: bool,
}

fn default_resolution_scale() -> u32 {
    1
}

impl Default for GraphicsConfig {
    fn default() -> Self {
        toml::from_str("").unwrap()
    }
}

impl GraphicsConfig {
    #[must_use]
    pub fn rasterizer_type(&self) -> RasterizerType {
        let use_avx2_software = self.avx2_software_rasterizer && supports_avx2();
        match (self.rasterizer, use_avx2_software) {
            (Rasterizer::Software, false) => RasterizerType::NaiveSoftware,
            (Rasterizer::Software, true) => RasterizerType::SimdSoftware,
            (Rasterizer::Hardware, _) => RasterizerType::WgpuHardware,
        }
    }
}

#[must_use]
pub fn supports_avx2() -> bool {
    cfg_if! {
        if #[cfg(target_arch = "x86_64")] {
            is_x86_feature_detected!("avx2")
        } else {
            false
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioConfig {
    #[serde(default)]
    pub adpcm_interpolation: AdpcmInterpolation,
    #[serde(default = "default_audio_sync_threshold")]
    pub sync_threshold: u32,
    #[serde(default = "default_device_queue_size")]
    pub device_queue_size: u16,
    #[serde(default = "default_internal_audio_buffer_size")]
    pub internal_buffer_size: NonZeroU32,
}

fn default_audio_sync_threshold() -> u32 {
    1024 + 512
}

fn default_device_queue_size() -> u16 {
    512
}

fn default_internal_audio_buffer_size() -> NonZeroU32 {
    NonZeroU32::new(ps1_core::api::DEFAULT_AUDIO_BUFFER_SIZE).unwrap()
}

impl Default for AudioConfig {
    fn default() -> Self {
        toml::from_str("").unwrap()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PathsConfig {
    pub bios: Option<PathBuf>,
    #[serde(default)]
    pub search: Vec<PathBuf>,
    #[serde(default = "true_fn")]
    pub search_recursively: bool,
}

impl Default for PathsConfig {
    fn default() -> Self {
        toml::from_str("").unwrap()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum MemoryCardMode {
    #[default]
    PerGame,
    Shared,
}

pub const MEMORY_CARDS_DIRECTORY: &str = "memcards";
pub const SHARED_CARD_1_FILE_NAME: &str = "shared_1.mcd";
pub const SHARED_CARD_2_FILE_NAME: &str = "shared_2.mcd";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryCardConfig {
    #[serde(default = "true_fn")]
    pub slot_1_enabled: bool,
    #[serde(default)]
    pub slot_2_enabled: bool,
    #[serde(default)]
    pub slot_1_mode: MemoryCardMode,
    #[serde(default)]
    pub slot_2_mode: MemoryCardMode,
}

impl Default for MemoryCardConfig {
    fn default() -> Self {
        toml::from_str("").unwrap()
    }
}

impl MemoryCardConfig {
    pub(crate) fn cards_enabled(&self) -> MemoryCardsEnabled {
        MemoryCardsEnabled { slot_1: self.slot_1_enabled, slot_2: self.slot_2_enabled }
    }

    pub(crate) fn slot_1_path<P: AsRef<Path>>(&self, disc_path: Option<P>) -> PathBuf {
        slot_path(self.slot_1_mode, disc_path, MemoryCardSlot::One)
    }

    pub(crate) fn slot_2_path<P: AsRef<Path>>(&self, disc_path: Option<P>) -> PathBuf {
        slot_path(self.slot_2_mode, disc_path, MemoryCardSlot::Two)
    }
}

fn slot_path<P: AsRef<Path>>(
    mode: MemoryCardMode,
    disc_path: Option<P>,
    slot: MemoryCardSlot,
) -> PathBuf {
    let shared_file_name = match slot {
        MemoryCardSlot::One => SHARED_CARD_1_FILE_NAME,
        MemoryCardSlot::Two => SHARED_CARD_2_FILE_NAME,
    };

    match (mode, disc_path) {
        (MemoryCardMode::PerGame, Some(disc_path)) => {
            let disc_path = disc_path.as_ref();
            let file_name_no_ext = match file_name_without_disc_or_revision(disc_path) {
                Ok(value) => value,
                Err(err) => {
                    log::error!(
                        "Unable to remove extension/Disc/Rev from file path '{}', using shared memory card for slot {}: {err}",
                        disc_path.display(),
                        slot as u8
                    );
                    return shared_path(shared_file_name);
                }
            };

            let memory_card_file_name_no_ext = format!("{file_name_no_ext}_{}", slot as u8);
            let memory_card_file_name =
                Path::new(&memory_card_file_name_no_ext).with_extension("mcd");
            Path::new(MEMORY_CARDS_DIRECTORY).join(memory_card_file_name)
        }
        (MemoryCardMode::PerGame, None) | (MemoryCardMode::Shared, _) => {
            shared_path(shared_file_name)
        }
    }
}

fn file_name_without_disc_or_revision(path: &Path) -> anyhow::Result<String> {
    static DISC_REV_REGEX: OnceLock<Regex> = OnceLock::new();

    let path_no_ext = path.with_extension("");
    let file_name_no_ext = path_no_ext.file_name().and_then(OsStr::to_str).ok_or_else(|| {
        anyhow!("Unable to determine file extension for path: {}", path.display())
    })?;

    let disc_rev_regex =
        DISC_REV_REGEX.get_or_init(|| Regex::new(r"( \(Disc [1-9]\))?( \(Rev [1-9]\))?$").unwrap());

    Ok(disc_rev_regex.replace(file_name_no_ext, "").into())
}

fn shared_path(shared_file_name: &str) -> PathBuf {
    Path::new(MEMORY_CARDS_DIRECTORY).join(shared_file_name)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FiltersConfig {
    #[serde(default = "true_fn")]
    pub exe: bool,
    #[serde(default = "true_fn")]
    pub cue: bool,
    #[serde(default = "true_fn")]
    pub chd: bool,
}

impl Default for FiltersConfig {
    fn default() -> Self {
        toml::from_str("").unwrap()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DebugConfig {
    #[serde(default)]
    pub tty_enabled: bool,
    #[serde(default)]
    pub vram_display: bool,
}

impl Default for DebugConfig {
    fn default() -> Self {
        toml::from_str("").unwrap()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InputConfig {
    #[serde(default = "default_p1_input_device")]
    pub p1_device: ControllerType,
    #[serde(default = "default_p2_input_device")]
    pub p2_device: ControllerType,
    #[serde(default = "default_p1_set_1")]
    pub p1_set_1: ControllerConfig,
    #[serde(default = "default_p1_set_2")]
    pub p1_set_2: ControllerConfig,
    #[serde(default = "default_p2_set")]
    pub p2_set_1: ControllerConfig,
    #[serde(default = "default_p2_set")]
    pub p2_set_2: ControllerConfig,
}

fn default_p1_input_device() -> ControllerType {
    ControllerType::Digital
}

fn default_p2_input_device() -> ControllerType {
    ControllerType::None
}

fn default_p1_set_1() -> ControllerConfig {
    ControllerConfig::default_p1_keyboard()
}

fn default_p1_set_2() -> ControllerConfig {
    ControllerConfig::default_p1_gamepad()
}

fn default_p2_set() -> ControllerConfig {
    ControllerConfig::none()
}

impl Default for InputConfig {
    fn default() -> Self {
        toml::from_str("").unwrap()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppConfig {
    #[serde(default)]
    pub video: VideoConfig,
    #[serde(default)]
    pub graphics: GraphicsConfig,
    #[serde(default)]
    pub audio: AudioConfig,
    #[serde(default)]
    pub paths: PathsConfig,
    #[serde(default)]
    pub memory_cards: MemoryCardConfig,
    #[serde(default)]
    pub filters: FiltersConfig,
    #[serde(default)]
    pub debug: DebugConfig,
    #[serde(default)]
    pub input: InputConfig,
}

impl Default for AppConfig {
    fn default() -> Self {
        toml::from_str("").unwrap()
    }
}

impl AppConfig {
    #[must_use]
    pub fn to_emulator_config(&self) -> Ps1EmulatorConfig {
        let rasterizer_type = self.graphics.rasterizer_type();

        Ps1EmulatorConfig {
            display: DisplayConfig {
                crop_vertical_overscan: self.video.crop_vertical_overscan,
                dump_vram: self.debug.vram_display,
                rasterizer_type,
                hardware_resolution_scale: self.graphics.hardware_resolution_scale,
                high_color: self.graphics.hardware_high_color,
                dithering_allowed: self.graphics.hardware_15bpp_dithering,
                high_res_dithering: self.graphics.high_res_dithering,
            },
            pgxp: match rasterizer_type {
                RasterizerType::WgpuHardware => PgxpConfig {
                    enabled: self.graphics.pgxp_enabled,
                    precise_nclip: self.graphics.pgxp_precise_culling,
                    perspective_texture_mapping: self.graphics.pgxp_perspective_texture_mapping,
                },
                // Disable PGXP when using software rasterizer
                RasterizerType::NaiveSoftware | RasterizerType::SimdSoftware => {
                    PgxpConfig::default()
                }
            },
            adpcm_interpolation: self.audio.adpcm_interpolation,
            internal_audio_buffer_size: self.audio.internal_buffer_size,
            tty_enabled: self.debug.tty_enabled,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gui_config_default_does_not_panic() {
        let _ = AppConfig::default();
    }
}
