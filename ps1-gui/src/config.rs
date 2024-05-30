use cfg_if::cfg_if;
use ps1_core::RasterizerType;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum VSyncMode {
    #[default]
    Enabled,
    Disabled,
    Fast,
}

impl VSyncMode {
    #[must_use]
    pub fn to_present_mode(self) -> wgpu::PresentMode {
        match self {
            Self::Enabled => wgpu::PresentMode::Fifo,
            Self::Disabled => wgpu::PresentMode::Immediate,
            Self::Fast => wgpu::PresentMode::Mailbox,
        }
    }
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
    pub vsync_mode: VSyncMode,
    #[serde(default)]
    pub filter_mode: FilterMode,
    #[serde(default = "true_fn")]
    pub crop_vertical_overscan: bool,
    #[serde(default)]
    pub vram_display: bool,
    #[serde(default = "default_window_width")]
    pub window_width: u32,
    #[serde(default = "default_window_height")]
    pub window_height: u32,
    #[serde(default)]
    pub rasterizer: Rasterizer,
    #[serde(default = "true_fn")]
    pub avx2_software_rasterizer: bool,
    #[serde(default)]
    pub wgpu_backend: WgpuBackend,
    #[serde(default = "default_resolution_scale")]
    pub hardware_resolution_scale: u32,
    #[serde(default)]
    pub async_swap_chain_rendering: bool,
}

fn true_fn() -> bool {
    true
}

fn default_resolution_scale() -> u32 {
    1
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

impl VideoConfig {
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
    #[serde(default = "default_audio_sync_threshold")]
    pub sync_threshold: u32,
    #[serde(default = "default_device_queue_size")]
    pub device_queue_size: u16,
}

fn default_audio_sync_threshold() -> u32 {
    1024 + 512
}

fn default_device_queue_size() -> u16 {
    1024
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppConfig {
    #[serde(default)]
    pub video: VideoConfig,
    #[serde(default)]
    pub audio: AudioConfig,
    #[serde(default)]
    pub paths: PathsConfig,
}

impl Default for AppConfig {
    fn default() -> Self {
        toml::from_str("").unwrap()
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
