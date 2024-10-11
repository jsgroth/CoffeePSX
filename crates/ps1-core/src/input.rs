use bincode::{Decode, Encode};
use proc_bitfield::bitfield;

bitfield! {
    #[derive(Clone, Copy, PartialEq, Eq, Default, bincode::Encode, bincode::Decode)]
    pub struct DigitalJoypadState(u16): Debug, IntoStorage {
        pub select: bool @ 0,
        pub start: bool @ 3,
        pub up: bool @ 4,
        pub right: bool @ 5,
        pub down: bool @ 6,
        pub left: bool @ 7,
        pub l2: bool @ 8,
        pub r2: bool @ 9,
        pub l1: bool @ 10,
        pub r1: bool @ 11,
        pub triangle: bool @ 12,
        pub circle: bool @ 13,
        pub cross: bool @ 14,
        pub square: bool @ 15,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Encode, Decode)]
pub enum AnalogMode {
    #[default]
    Digital,
    Analog,
}

impl AnalogMode {
    #[must_use]
    pub fn toggle(self) -> Self {
        match self {
            Self::Digital => Self::Analog,
            Self::Analog => Self::Digital,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
pub struct AnalogJoypadState {
    pub analog_button: bool,
    pub left_x: u8,
    pub left_y: u8,
    pub right_x: u8,
    pub right_y: u8,
    pub l3: bool,
    pub r3: bool,
}

impl Default for AnalogJoypadState {
    fn default() -> Self {
        Self {
            analog_button: false,
            // 0x80 represents the center position for both axes
            left_x: 0x80,
            left_y: 0x80,
            right_x: 0x80,
            right_y: 0x80,
            l3: false,
            r3: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum ControllerType {
    None,
    Digital,
    DualShock,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
pub struct ControllerState {
    pub controller_type: ControllerType,
    pub digital: DigitalJoypadState,
    pub analog: AnalogJoypadState,
}

impl ControllerState {
    pub(crate) fn default_p1() -> Self {
        ControllerState {
            controller_type: ControllerType::Digital,
            digital: DigitalJoypadState::default(),
            analog: AnalogJoypadState::default(),
        }
    }

    pub(crate) fn default_p2() -> Self {
        ControllerState {
            controller_type: ControllerType::None,
            digital: DigitalJoypadState::default(),
            analog: AnalogJoypadState::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ps1Inputs {
    pub p1: ControllerState,
    pub p2: ControllerState,
}

impl Default for Ps1Inputs {
    fn default() -> Self {
        Self { p1: ControllerState::default_p1(), p2: ControllerState::default_p2() }
    }
}
