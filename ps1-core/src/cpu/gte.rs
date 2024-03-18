use bincode::{Decode, Encode};

#[derive(Debug, Clone, Encode, Decode)]
pub struct GeometryTransformationEngine;

impl GeometryTransformationEngine {
    pub fn new() -> Self {
        Self
    }

    #[allow(clippy::unused_self)]
    pub fn load_word(&mut self, register: u32, value: u32) {
        log::warn!("Unimplemented GTE register load: R{register} <- {value:08X}");
    }

    #[allow(clippy::unused_self)]
    pub fn read_register(&self, register: u32) -> u32 {
        log::warn!("Unimplemented GTE register read: R{register}");
        0
    }

    #[allow(clippy::unused_self)]
    pub fn write_register(&mut self, register: u32, value: u32) {
        log::warn!("Unimplemented GTE register write: R{register} <- {value:08X}");
    }

    #[allow(clippy::unused_self)]
    pub fn read_control_register(&self, register: u32) -> u32 {
        log::warn!("Unimplemented GTE control register read: R{register}");
        0
    }

    #[allow(clippy::unused_self)]
    pub fn write_control_register(&mut self, register: u32, value: u32) {
        log::warn!("Unimplemented GTE control register write: R{register} <- {value:08X}");
    }
}
