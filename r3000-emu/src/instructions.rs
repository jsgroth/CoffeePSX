use crate::bus::BusInterface;
use crate::R3000;

impl R3000 {
    pub(super) fn execute_opcode<B: BusInterface>(&mut self, opcode: u32, bus: &mut B) {
        // First 6 bits of opcode identify operation
        match opcode >> 26 {
            0x0F => self.lui(opcode),
            _ => todo!("opcode {opcode:08X}"),
        }
    }

    // LUI: Load upper immediate
    fn lui(&mut self, opcode: u32) {
        let register = (opcode >> 16) & 0x1F;
        self.registers.write_gpr(register, opcode << 16);
    }
}
