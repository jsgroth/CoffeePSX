use proc_bitfield::bitfield;

bitfield! {
    #[derive(Clone, Copy, PartialEq, Eq, Default)]
    pub struct Ps1JoypadState(u16): Debug, IntoRaw {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Ps1Inputs {
    pub p1: Ps1JoypadState,
}
