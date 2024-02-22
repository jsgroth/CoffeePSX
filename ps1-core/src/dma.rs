use crate::num::U32Ext;
use std::array;

#[derive(Debug, Clone)]
struct ControlRegister {
    channel_priority: [u8; 7],
    channel_enabled: [bool; 7],
}

impl ControlRegister {
    fn new() -> Self {
        // Control register value at power-on should be $07654321:
        //   Channel 0 has priority 1, channel 1 has priority 2, etc.
        //   All channels are disabled
        Self {
            channel_priority: array::from_fn(|i| (i + 1) as u8),
            channel_enabled: array::from_fn(|_| false),
        }
    }

    fn read(&self) -> u32 {
        let priority_bits = self
            .channel_priority
            .into_iter()
            .enumerate()
            .map(|(channel, priority)| u32::from(priority) << (4 * channel))
            .reduce(|a, b| a | b)
            .unwrap();

        let enabled_bits = self
            .channel_enabled
            .into_iter()
            .enumerate()
            .map(|(channel, enabled)| u32::from(enabled) << (3 + 4 * channel))
            .reduce(|a, b| a | b)
            .unwrap();

        priority_bits | enabled_bits
    }

    fn write(&mut self, value: u32) {
        self.channel_priority = array::from_fn(|i| ((value >> (4 * i)) & 7) as u8);

        self.channel_enabled = array::from_fn(|i| value.bit(3 + 4 * i as u8));
    }
}

#[derive(Debug, Clone)]
pub struct DmaController {
    control: ControlRegister,
}

impl DmaController {
    pub fn new() -> Self {
        Self {
            control: ControlRegister::new(),
        }
    }

    pub fn read_control(&self) -> u32 {
        self.control.read()
    }

    pub fn write_control(&mut self, value: u32) {
        self.control.write(value);

        log::trace!("DMA control register write: {value:08X}");
    }
}
