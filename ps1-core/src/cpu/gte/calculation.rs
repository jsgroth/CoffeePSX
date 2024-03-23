//! GTE general-purpose calculation instructions

use crate::cpu::gte;
use crate::cpu::gte::fixedpoint::FixedPointDecimal;
use crate::cpu::gte::registers::Register;
use crate::cpu::gte::{fixedpoint, GeometryTransformationEngine, MatrixMultiplyBehavior};
use crate::num::U32Ext;

impl GeometryTransformationEngine {
    // MVMVA: Multiply vector by matrix and vector addition
    // Matrix and vector parameters are specified using opcode bits, and the result is written to
    // IR1-3 and MAC1-3
    pub(super) fn mvmva(&mut self, opcode: u32) {
        log::trace!("GTE MVMVA {opcode:08X}");

        let matrix = match (opcode >> 17) & 3 {
            0 => self.read_matrix(Register::RT1112),
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

    // SQR: Square vector
    // Squares the components of the IR vector
    pub(super) fn sqr(&mut self, opcode: u32) {
        log::trace!("GTE SQR {opcode:08X}");

        let ir_squared = self.read_ir_vector().map(|ir| ir * ir);
        if opcode.bit(gte::SF_BIT) {
            let [mac1, mac2, mac3] = ir_squared.map(|ir| ir.reinterpret::<12>().shift_to::<0>());
            self.set_mac(mac1, mac2, mac3);
        } else {
            self.set_mac(ir_squared[0], ir_squared[1], ir_squared[2]);
        }

        let [mac1, mac2, mac3] = self.read_mac_vector::<0>();
        self.set_ir(mac1, mac2, mac3, false);
    }

    // OP: Cross product
    // Computes the cross product of the IR vector and [RT11, RT22, RT33], and stores the result in IR
    pub(super) fn op(&mut self, opcode: u32) {
        log::trace!("GTE OP {opcode:08X}");

        let [ir1, ir2, ir3] = self.read_ir_vector();
        let d1 = fixedpoint::vector16_component(self.r[Register::RT1112]);
        let d2 = fixedpoint::vector16_component(self.r[Register::RT2223]);
        let d3 = fixedpoint::vector16_component(self.r[Register::RT33]);

        let mac1 = ir3 * d2 - ir2 * d3;
        let mac2 = ir1 * d3 - ir3 * d1;
        let mac3 = ir2 * d1 - ir1 * d2;
        if opcode.bit(gte::SF_BIT) {
            let [mac1, mac2, mac3] =
                [mac1, mac2, mac3].map(|mac| mac.reinterpret::<12>().shift_to::<0>());
            self.set_mac(mac1, mac2, mac3);
        } else {
            self.set_mac(mac1, mac2, mac3);
        }

        let [mac1, mac2, mac3] = self.read_mac_vector::<0>();
        self.set_ir(mac1, mac2, mac3, opcode.bit(gte::LM_BIT));
    }
}
