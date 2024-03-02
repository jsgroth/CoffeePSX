use crate::gpu::gp0::{Gp0CommandState, Gp0State};
use crate::gpu::ClockState;
use std::fmt::{Display, Formatter};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DmaMode {
    #[default]
    Off = 0,
    Fifo = 1,
    CpuToGpu = 2,
    GpuToCpu = 3,
}

impl DmaMode {
    pub fn from_bits(bits: u32) -> Self {
        match bits & 3 {
            0 => Self::Off,
            1 => Self::Fifo,
            2 => Self::CpuToGpu,
            3 => Self::GpuToCpu,
            _ => unreachable!("value & 3 is always <= 3"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HorizontalResolution {
    // 256px
    #[default]
    TwoFiftySix = 0,
    // 320px
    ThreeTwenty = 1,
    // 512px
    FiveTwelve = 2,
    // 640px
    SixForty = 3,
}

impl Display for HorizontalResolution {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TwoFiftySix => write!(f, "256px"),
            Self::ThreeTwenty => write!(f, "320px"),
            Self::FiveTwelve => write!(f, "512px"),
            Self::SixForty => write!(f, "640px"),
        }
    }
}

impl HorizontalResolution {
    pub fn from_bits(bits: u32) -> Self {
        match bits & 3 {
            0 => Self::TwoFiftySix,
            1 => Self::ThreeTwenty,
            2 => Self::FiveTwelve,
            3 => Self::SixForty,
            _ => unreachable!("value & 3 is always <= 3"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VerticalResolution {
    // 240px
    #[default]
    Single = 0,
    // 480px (interlaced)
    Double = 1,
}

impl VerticalResolution {
    pub fn from_bit(bit: bool) -> Self {
        if bit {
            Self::Double
        } else {
            Self::Single
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VideoMode {
    #[default]
    Ntsc = 0,
    Pal = 1,
}

impl Display for VideoMode {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Ntsc => write!(f, "NTSC/60Hz"),
            Self::Pal => write!(f, "PAL/50Hz"),
        }
    }
}

impl VideoMode {
    pub fn from_bit(bit: bool) -> Self {
        if bit {
            Self::Pal
        } else {
            Self::Ntsc
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ColorDepthBits {
    #[default]
    Fifteen = 0,
    TwentyFour = 1,
}

impl Display for ColorDepthBits {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Fifteen => write!(f, "15-bit"),
            Self::TwentyFour => write!(f, "24-bit"),
        }
    }
}

impl ColorDepthBits {
    pub fn from_bit(bit: bool) -> Self {
        if bit {
            Self::TwentyFour
        } else {
            Self::Fifteen
        }
    }
}

pub const DEFAULT_X_DISPLAY_RANGE: (u32, u32) = (0x200, 0x200 + 256 * 10);
pub const DEFAULT_Y_DISPLAY_RANGE: (u32, u32) = (0x010, 0x010 + 240);

#[derive(Debug, Clone)]
pub struct Registers {
    pub irq: bool,
    pub display_enabled: bool,
    pub dma_mode: DmaMode,
    pub display_area_x: u32,
    pub display_area_y: u32,
    pub x_display_range: (u32, u32),
    pub y_display_range: (u32, u32),
    pub h_resolution: HorizontalResolution,
    pub v_resolution: VerticalResolution,
    pub video_mode: VideoMode,
    pub display_area_color_depth: ColorDepthBits,
    pub interlaced: bool,
    pub force_h_368px: bool,
}

impl Registers {
    pub fn new() -> Self {
        Self {
            irq: false,
            display_enabled: false,
            dma_mode: DmaMode::default(),
            display_area_x: 0,
            display_area_y: 0,
            x_display_range: DEFAULT_X_DISPLAY_RANGE,
            y_display_range: DEFAULT_Y_DISPLAY_RANGE,
            h_resolution: HorizontalResolution::default(),
            v_resolution: VerticalResolution::default(),
            video_mode: VideoMode::default(),
            display_area_color_depth: ColorDepthBits::default(),
            interlaced: false,
            force_h_368px: false,
        }
    }

    pub fn read_status(&self, gp0_state: &Gp0State, clock_state: &ClockState) -> u32 {
        let ready_to_receive_command =
            matches!(gp0_state.command_state, Gp0CommandState::WaitingForCommand);
        let ready_to_send_vram =
            matches!(gp0_state.command_state, Gp0CommandState::SendingToCpu(..));
        let ready_to_receive_dma = ready_to_receive_command;

        let dma_request: u32 = match self.dma_mode {
            DmaMode::Off => 0,
            DmaMode::Fifo => 1,
            DmaMode::CpuToGpu => ready_to_receive_dma.into(),
            DmaMode::GpuToCpu => ready_to_send_vram.into(),
        };

        let interlaced_bit = if self.interlaced {
            clock_state.odd_frame && (16..256).contains(&clock_state.line)
        } else {
            (16..256).contains(&clock_state.line) && clock_state.line % 2 == 1
        };

        // TODO bits hardcoded:
        //   Bit 13: interlaced field
        //   Bit 14: "Reverseflag"
        //   Bit 31: Even/odd line
        gp0_state.global_texture_page.x_base
            | ((gp0_state.global_texture_page.y_base / 256) << 4)
            | ((gp0_state.global_texture_page.semi_transparency_mode as u32) << 5)
            | ((gp0_state.global_texture_page.color_depth as u32) << 7)
            | (u32::from(gp0_state.draw_settings.dithering_enabled) << 9)
            | (u32::from(gp0_state.draw_settings.drawing_in_display_allowed) << 10)
            | (u32::from(gp0_state.draw_settings.force_mask_bit) << 11)
            | (u32::from(gp0_state.draw_settings.check_mask_bit) << 12)
            | (1 << 13)
            | (u32::from(self.force_h_368px) << 16)
            | ((self.h_resolution as u32) << 17)
            | ((self.v_resolution as u32) << 19)
            | ((self.video_mode as u32) << 20)
            | ((self.display_area_color_depth as u32) << 21)
            | (u32::from(self.interlaced) << 22)
            | (u32::from(!self.display_enabled) << 23)
            | (u32::from(self.irq) << 24)
            | (u32::from(dma_request) << 25)
            | (u32::from(ready_to_receive_command) << 26)
            | (u32::from(ready_to_send_vram) << 27)
            | (u32::from(ready_to_receive_dma) << 28)
            | ((self.dma_mode as u32) << 29)
            | (u32::from(interlaced_bit) << 31)
    }
}
