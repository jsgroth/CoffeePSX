//! GTE general-purpose calculation instructions

use crate::cpu::gte::fixedpoint::FixedPointDecimal;
use crate::cpu::gte::registers::Register;
use crate::cpu::gte::{GeometryTransformationEngine, MatrixMultiplyBehavior};

impl GeometryTransformationEngine {
    // MVMVA: Multiply vector by matrix and vector addition
    // Matrix and vector parameters are specified using opcode bits, and the result is written to
    // IR1-3 and MAC1-3
    pub(super) fn mvmva(&mut self, opcode: u32) {
        log::trace!("GTE MVMVA {opcode:08X}");

        let matrix = match (opcode >> 17) & 3 {
            0 => self.read_matrix(Register::RT_START),
            1 => self.read_matrix(Register::LLM_START),
            2 => self.read_matrix(Register::LCM_START),
            3 => todo!("MVMVA executed with bugged matrix 3"),
            _ => unreachable!("value & 3 is always <= 3"),
        };

        let multiply_vector = match (opcode >> 15) & 3 {
            0 => self.read_vector16_packed(Register::VXY0, Register::VZ0),
            1 => self.read_vector16_packed(Register::VXY1, Register::VZ1),
            2 => self.read_vector16_packed(Register::VXY2, Register::VZ2),
            3 => self.read_ir_vector(),
            _ => unreachable!("value & 3 is always <= 3"),
        };

        let translation_vector = match (opcode >> 13) & 3 {
            0 => self.read_translation_vector(),
            1 => self.read_background_color(),
            2 => todo!("MVMVA executed with bugged FC translation"),
            3 => [FixedPointDecimal::ZERO, FixedPointDecimal::ZERO, FixedPointDecimal::ZERO],
            _ => unreachable!("value & 3 is always <= 3"),
        };

        self.matrix_multiply_add(
            opcode,
            &multiply_vector,
            &matrix,
            &translation_vector,
            MatrixMultiplyBehavior::Standard,
        );
    }
}
