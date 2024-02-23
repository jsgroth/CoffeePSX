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
    TwoFiftySix = 0,
    // 320px
    #[default]
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

    pub fn read_status(&self) -> u32 {
        // TODO bits hardcoded:
        //   Bits 0-12 and 15: various GP0 fields
        //   Bit 13: interlaced field
        //   Bit 14: "Reverseflag"
        //   Bits 25-28: DMA request bits
        (1 << 13)
            | (u32::from(self.force_h_368px) << 16)
            | ((self.h_resolution as u32) << 17)
            | ((self.v_resolution as u32) << 19)
            | ((self.video_mode as u32) << 20)
            | ((self.display_area_color_depth as u32) << 21)
            | (u32::from(self.irq) << 24)
            | (1 << 26)
            | (1 << 27)
            | (1 << 28)
            | ((self.dma_mode as u32) << 29)
    }
}
