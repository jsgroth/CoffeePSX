//! GTE general-purpose calculation instructions

use crate::cpu::gte;
use crate::cpu::gte::fixedpoint::{FixedPointDecimal, MatrixComponent, Vector16Component};
use crate::cpu::gte::registers::{Flag, Register};
use crate::cpu::gte::{fixedpoint, GeometryTransformationEngine, Mac, MatrixMultiplyBehavior};
use crate::num::U32Ext;

impl GeometryTransformationEngine {
    // MVMVA: Multiply vector by matrix and vector addition
    // Matrix and vector parameters are specified using opcode bits, and the result is written to
    // IR1-3 and MAC1-3
    pub(super) fn mvmva(&mut self, opcode: u32) -> u32 {
        log::trace!("GTE MVMVA {opcode:08X}");

        let matrix = match (opcode >> 17) & 3 {
            0 => self.read_matrix(Register::RT1112),
            1 => self.read_matrix(Register::LLM_START),
            2 => self.read_matrix(Register::LCM_START),
            3 => self.read_bugged_matrix(),
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
            1 => self.read_background_color().map(FixedPointDecimal::reinterpret),
            2 => {
                self.bugged_fc_mvmva(opcode, &multiply_vector, &matrix);
                return 7;
            }
            3 => [FixedPointDecimal::ZERO, FixedPointDecimal::ZERO, FixedPointDecimal::ZERO],
            _ => unreachable!("value & 3 is always <= 3"),
        };

        self.matrix_multiply_add(
            opcode,
            &multiply_vector,
            &matrix,
            &translation_vector,
            MatrixMultiplyBehavior::Mvmva,
        );

        7
    }

    // MVMVA with Tx=2 is supposed to translate by the FC vector but the implementation is bugged.
    // Immediately after the first step of the MAC operation (MACx = (FC[x] << 12) + M[x][0] * V[0]),
    // the hardware erroneously performs the IR1-3 saturation check and then sets MACx to 0 before
    // the remaining two steps.
    // This makes the final result ignore the FC vector and the X components entirely:
    //   MACx = M[x][1] * V[1] + M[x][2] * V[2]
    //   IRx = clamp(MACx, lm)
    // However, the first step must still be performed because it can set the MAC overflow and IR
    // saturation flags.
    #[allow(clippy::redundant_closure_for_method_calls)]
    fn bugged_fc_mvmva(
        &mut self,
        opcode: u32,
        vector: &[Vector16Component; 3],
        matrix: &[[MatrixComponent; 3]; 3],
    ) {
        let shift = if opcode.bit(gte::SF_BIT) { 12 } else { 0 };

        let far_color = self.read_far_color().map(|fc| fc.reinterpret::<0>());

        for (i, mac) in [(0, Mac::One), (1, Mac::Two), (2, Mac::Three)] {
            self.mac[mac as usize] = 0;
            self.accumulate_into_mac(far_color[i].shift_to::<12>() + matrix[i][0] * vector[0], mac);

            let bugged_value = (self.mac[mac as usize] >> shift) as i32;
            if bugged_value < i16::MIN.into() || bugged_value > i16::MAX.into() {
                let ir_saturation_flag = mac.corresponding_ir_saturation_flag();
                self.r[Register::FLAG] |= ir_saturation_flag;
                if ir_saturation_flag != Flag::IR3_SATURATED {
                    self.r[Register::FLAG] |= Flag::ERROR;
                }
            }

            self.mac[mac as usize] = 0;
            self.accumulate_into_mac(matrix[i][1] * vector[1], mac);
            self.accumulate_into_mac(matrix[i][2] * vector[2], mac);
        }

        self.apply_mac_shift(opcode);

        let [mac1, mac2, mac3] = self.read_mac_vector::<0>();
        self.set_ir(mac1, mac2, mac3, opcode.bit(gte::LM_BIT));
    }

    // MVMVA with Mx=3 is supposed to be an invalid command, but the hardware will still execute it.
    // It performs MVMVA as normal but with the following bugged matrix:
    //   [ -(R << 4), (R << 4), IR0   ]
    //   [    RT13,     RT13,   RT13  ]
    //   [    RT22,     RT22,   RT22  ]
    // R is read from the RGBC register.
    fn read_bugged_matrix(&self) -> [[MatrixComponent; 3]; 3] {
        let r: i64 = ((self.r[Register::RGBC] & 0xFF) << 4).into();
        let ir0 = fixedpoint::ir0(self.r[Register::IR0]);
        let rt13 = fixedpoint::matrix_component(self.r[Register::RT1321]);
        let rt22 = fixedpoint::matrix_component(self.r[Register::RT2223]);

        [
            [FixedPointDecimal::new(-r), FixedPointDecimal::new(r), ir0.reinterpret()],
            [rt13, rt13, rt13],
            [rt22, rt22, rt22],
        ]
    }

    // SQR: Square vector
    // Squares the components of the IR vector
    pub(super) fn sqr(&mut self, opcode: u32) -> u32 {
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

        4
    }

    // OP: Cross product
    // Computes the cross product of the IR vector and [RT11, RT22, RT33], and stores the result in IR
    pub(super) fn op(&mut self, opcode: u32) -> u32 {
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

        5
    }
}
