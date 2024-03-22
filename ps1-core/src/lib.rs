pub mod api;
mod bus;
mod cd;
mod cpu;
mod dma;
mod gpu;
pub mod input;
mod interrupts;
mod mdec;
mod memory;
mod num;
mod scheduler;
mod sio;
mod spu;
mod timers;

pub const VRAM_WIDTH: u16 = 1024;
pub const VRAM_HEIGHT: u16 = 512;
