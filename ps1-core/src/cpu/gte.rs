#[derive(Debug, Clone)]
pub struct GeometryTransformationEngine;

impl GeometryTransformationEngine {
    pub fn new() -> Self {
        Self
    }

    #[allow(clippy::unused_self)]
    pub fn write_control_register(&mut self, register: u32, value: u32) {
        log::warn!("Unimplemented GTE control register write: R{register} <- {value:08X}");
    }
}
