pub struct Register;

#[allow(dead_code)]
impl Register {
    // V0-2: 16-bit vectors
    pub const VXY0: usize = 0;
    pub const VZ0: usize = 1;
    pub const VXY1: usize = 2;
    pub const VZ1: usize = 3;
    pub const VXY2: usize = 4;
    pub const VZ2: usize = 5;
    // RGBC: Color/code
    pub const RGBC: usize = 6;
    // OTZ: Average Z value for ordering table
    pub const OTZ: usize = 7;
    // IR0: Interpolation factor
    pub const IR0: usize = 8;
    // IR1-3: 16-bit vector with each element occupying its own register
    pub const IR1: usize = 9;
    pub const IR2: usize = 10;
    pub const IR3: usize = 11;
    // SXY0-2 + SXYP: Screen XY coordinate FIFO
    pub const SXY0: usize = 12;
    pub const SXY1: usize = 13;
    pub const SXY2: usize = 14;
    pub const SXYP: usize = 15;
    // SZ0-3: Screen Z coordinate FIFO
    pub const SZ0: usize = 16;
    pub const SZ1: usize = 17;
    pub const SZ2: usize = 18;
    pub const SZ3: usize = 19;
    // RGB0-2: Color FIFO
    pub const RGB0: usize = 20;
    pub const RGB1: usize = 21;
    pub const RGB2: usize = 22;
    // (R23 is unused)
    // MAC0-3: Multiply-accumulate results
    pub const MAC0: usize = 24;
    pub const MAC1: usize = 25;
    pub const MAC2: usize = 26;
    pub const MAC3: usize = 27;
    // IRGB: Color conversion input
    pub const IRGB: usize = 28;
    // ORGB: Color conversion output
    pub const ORGB: usize = 29;
    // LZCS: Count leading bits source data
    pub const LZCS: usize = 30;
    // LZCR: Count leading bits result
    pub const LZCR: usize = 31;
    // RT: Rotation matrix (R32-36)
    pub const RT_START: usize = 32;
    // TR: Translation vector
    pub const TRX: usize = 37;
    pub const TRY: usize = 38;
    pub const TRZ: usize = 39;
    // LLM: Light matrix (R40-44)
    pub const LLM_START: usize = 40;
    // BK: Background color
    pub const RBK: usize = 45;
    pub const GBK: usize = 46;
    pub const BBK: usize = 47;
    // LCM: Light color matrix (R48-52)
    pub const LCM_START: usize = 48;
    // FC: Far color
    pub const RFC: usize = 53;
    pub const GFC: usize = 54;
    pub const BFC: usize = 55;
    // OF: Screen offset
    pub const OFX: usize = 56;
    pub const OFY: usize = 57;
    // H: Projection plane distance
    pub const H: usize = 58;
    // DQA: Depth cueing coefficient
    pub const DQA: usize = 59;
    // DQB: Depth cueing offset
    pub const DQB: usize = 60;
    // ZSF3: Z3 average scale factor (for AVSZ3)
    pub const ZSF3: usize = 61;
    // ZSF4: Z4 average scale factor (for AVSZ4)
    pub const ZSF4: usize = 62;
    // FLAG: Calculation error flags
    pub const FLAG: usize = 63;

    pub fn name(register: u32) -> &'static str {
        match register {
            0 => "VXY0",
            1 => "VZ0",
            2 => "VXY1",
            3 => "VZ1",
            4 => "VXY2",
            5 => "VZ2",
            6 => "RGBC",
            7 => "OTZ",
            8 => "IR0",
            9 => "IR1",
            10 => "IR2",
            11 => "IR3",
            12 => "SXY0",
            13 => "SXY1",
            14 => "SXY2",
            15 => "SXYP",
            16 => "SZ0",
            17 => "SZ1",
            18 => "SZ2",
            19 => "SZ3",
            20 => "RGB0",
            21 => "RGB1",
            22 => "RGB2",
            23 => "(unused)",
            24 => "MAC0",
            25 => "MAC1",
            26 => "MAC2",
            27 => "MAC3",
            28 => "IRGB",
            29 => "ORGB",
            30 => "LZCS",
            31 => "LZCR",
            32 => "RT11/RT12",
            33 => "RT13/RT21",
            34 => "RT22/RT23",
            35 => "RT31/RT32",
            36 => "RT33",
            37 => "TRX",
            38 => "TRY",
            39 => "TRZ",
            40 => "LLM11/LLM12",
            41 => "LLM13/LLM21",
            42 => "LLM22/LLM23",
            43 => "LLM31/LLM32",
            44 => "LLM33",
            45 => "RBK",
            46 => "GBK",
            47 => "BBK",
            48 => "LCM11/LCM12",
            49 => "LCM13/LCM21",
            50 => "LCM22/LCM23",
            51 => "LCM31/LCM32",
            52 => "LCM33",
            53 => "RFC",
            54 => "GFC",
            55 => "BFC",
            56 => "OFX",
            57 => "OFY",
            58 => "H",
            59 => "DQA",
            60 => "DQB",
            61 => "ZSF3",
            62 => "ZSF4",
            63 => "FLAG",
            _ => "(invalid register value)",
        }
    }
}

pub struct Flag;

impl Flag {
    pub const ERROR: u32 = 1 << 31;
    pub const MAC1_OVERFLOW_POSITIVE: u32 = 1 << 30;
    pub const MAC2_OVERFLOW_POSITIVE: u32 = 1 << 29;
    pub const MAC3_OVERFLOW_POSITIVE: u32 = 1 << 28;
    pub const MAC1_OVERFLOW_NEGATIVE: u32 = 1 << 27;
    pub const MAC2_OVERFLOW_NEGATIVE: u32 = 1 << 26;
    pub const MAC3_OVERFLOW_NEGATIVE: u32 = 1 << 25;
    pub const IR1_SATURATED: u32 = 1 << 24;
    pub const IR2_SATURATED: u32 = 1 << 23;
    pub const IR3_SATURATED: u32 = 1 << 22;
    pub const SZ3_OTZ_SATURATED: u32 = 1 << 18;
    pub const DIVIDE_OVERFLOW: u32 = 1 << 17;
    pub const MAC0_OVERFLOW_POSITIVE: u32 = 1 << 16;
    pub const MAC0_OVERFLOW_NEGATIVE: u32 = 1 << 15;
    pub const SX2_SATURATED: u32 = 1 << 14;
    pub const SY2_SATURATED: u32 = 1 << 13;
    pub const IR0_SATURATED: u32 = 1 << 12;
}
