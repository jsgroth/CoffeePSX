use crate::gpu::gp0::Gp0CommandState;
use crate::gpu::registers::{
    ColorDepthBits, DmaMode, HorizontalResolution, VerticalResolution, VideoMode,
    DEFAULT_X_DISPLAY_RANGE, DEFAULT_Y_DISPLAY_RANGE,
};
use crate::gpu::Gpu;
use crate::num::U32Ext;

const RESET_06_VALUE: u32 = DEFAULT_X_DISPLAY_RANGE.0 | (DEFAULT_X_DISPLAY_RANGE.1 << 12);
const RESET_07_VALUE: u32 = DEFAULT_Y_DISPLAY_RANGE.0 | (DEFAULT_Y_DISPLAY_RANGE.1 << 10);

impl Gpu {
    pub fn write_gp1_command(&mut self, value: u32) {
        log::trace!("GP1 command write: {value:08X}");

        // Highest 8 bits of word determine command
        match value >> 24 {
            0x00 => self.reset(),
            0x01 => self.reset_command_buffer(),
            0x02 => self.acknowledge_interrupt(),
            0x03 => self.set_display_enabled(value),
            0x04 => self.set_dma_mode(value),
            0x05 => self.set_display_area_start(value),
            0x06 => self.set_horizontal_display_range(value),
            0x07 => self.set_vertical_display_range(value),
            0x08 => self.set_display_mode(value),
            0x10..=0x1F => self.get_gpu_info(value),
            _ => log::error!("unimplemented GP1 command {value:08X}"),
        }
    }

    // GP1($00)
    fn reset(&mut self) {
        log::trace!("GP1($00): Reset");

        self.reset_command_buffer();
        self.acknowledge_interrupt();
        self.set_display_enabled(1);
        self.set_dma_mode(0);
        self.set_display_area_start(0);
        self.set_horizontal_display_range(RESET_06_VALUE);
        self.set_vertical_display_range(RESET_07_VALUE);
        self.set_display_mode(0);

        for gp0_command in 0xE1..=0xE6 {
            self.write_gp0_command(gp0_command << 24);
        }
    }

    // GP1($01)
    fn reset_command_buffer(&mut self) {
        // TODO is this right?
        self.gp0_state.command_state = Gp0CommandState::WaitingForCommand;

        log::trace!("GP1($01): Reset command buffer");
    }

    // GP1($02)
    fn acknowledge_interrupt(&mut self) {
        self.registers.irq = false;

        log::trace!("GP1($02): Acknowledge IRQ");
    }

    // GP1($03)
    fn set_display_enabled(&mut self, value: u32) {
        // 0=on, 1=off
        self.registers.display_enabled = !value.bit(0);

        log::trace!(
            "GP1($03): Display enabled - {}",
            self.registers.display_enabled
        );
    }

    // GP1($04)
    fn set_dma_mode(&mut self, value: u32) {
        self.registers.dma_mode = DmaMode::from_bits(value);

        log::trace!("GP1($04): DMA mode - {:?}", self.registers.dma_mode);
    }

    // GP1($05)
    fn set_display_area_start(&mut self, value: u32) {
        self.registers.display_area_x = value & 0x3FF;
        self.registers.display_area_y = (value >> 10) & 0x1FF;

        log::trace!("GP1($05): Display area start");
        log::trace!(
            "  X={}, Y={}",
            self.registers.display_area_x,
            self.registers.display_area_y
        );
    }

    // GP1($06)
    fn set_horizontal_display_range(&mut self, value: u32) {
        let x1 = value & 0xFFF;
        let x2 = (value >> 12) & 0xFFF;
        self.registers.x_display_range = (x1, x2);

        log::trace!("GP1($06): Horizontal display range");
        log::trace!(
            "  (X1, X2)=({:X}, {:X})",
            self.registers.x_display_range.0,
            self.registers.x_display_range.1
        );
    }

    // GP1($07)
    fn set_vertical_display_range(&mut self, value: u32) {
        let y1 = value & 0x3FF;
        let y2 = (value >> 10) & 0x3FF;
        self.registers.y_display_range = (y1, y2);

        log::trace!("GP1($07): Vertical display range");
        log::trace!(
            "  (Y1, Y2)=({:X}, {:X})",
            self.registers.y_display_range.0,
            self.registers.y_display_range.1
        );
    }

    // GP1($08)
    fn set_display_mode(&mut self, value: u32) {
        self.registers.h_resolution = HorizontalResolution::from_bits(value);
        self.registers.v_resolution = VerticalResolution::from_bit(value.bit(2));
        self.registers.video_mode = VideoMode::from_bit(value.bit(3));
        self.registers.display_area_color_depth = ColorDepthBits::from_bit(value.bit(4));
        self.registers.interlaced = value.bit(5);
        self.registers.force_h_368px = value.bit(6);
        // TODO "reverseflag"

        log::trace!("GP1($08): Display mode");
        log::trace!("  Horizontal resolution: {}", self.registers.h_resolution);
        log::trace!("  Vertical resolution: {:?}", self.registers.v_resolution);
        log::trace!("  Video mode: {}", self.registers.video_mode);
        log::trace!(
            "  Display area color depth: {}",
            self.registers.display_area_color_depth
        );
        log::trace!("  Interlacing on: {}", self.registers.interlaced);
        log::trace!(
            "  Force horizontal resolution to 368px: {}",
            self.registers.force_h_368px
        );
    }

    // GP1($10)
    fn get_gpu_info(&mut self, value: u32) {
        match value & 0xF {
            // GPU version (hardcoded to 2)
            0x7 => self.gpu_read_buffer = 2,
            _ => todo!("GP1 GPU info command {value:08X}"),
        }
    }
}
