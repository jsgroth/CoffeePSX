//! GTE color calculation instructions

use crate::cpu::gte;
use crate::cpu::gte::fixedpoint::{FarColor, FixedPointDecimal, Vector16Component};
use crate::cpu::gte::registers::Register;
use crate::cpu::gte::{fixedpoint, GeometryTransformationEngine, MatrixMultiplyBehavior};
use crate::num::U32Ext;

const ZERO_VECTOR: [FixedPointDecimal<0>; 3] =
    [FixedPointDecimal::ZERO, FixedPointDecimal::ZERO, FixedPointDecimal::ZERO];

impl GeometryTransformationEngine {
    // NCDS: Normal color depth cue, single vector
    // Performs color calculation on the vector V0 with depth cueing and writes the results to the
    // color FIFO
    pub(super) fn ncds(&mut self, opcode: u32) {
        log::trace!("GTE NCDS {opcode:08X}");

        self.apply_light_matrix(opcode, Register::VXY0, Register::VZ0);
        self.apply_light_color_matrix(opcode);
        self.apply_color_vector();
        self.apply_depth_cue(opcode);
        self.apply_mac_shift(opcode);
        self.push_to_color_fifo(opcode);
    }

    // MAC = (LLM * V) >> (sf * 12)
    // IR = MAC
    fn apply_light_matrix(&mut self, opcode: u32, vxy: usize, vz: usize) {
        let vector = self.read_vector16_packed(vxy, vz);
        let light_matrix = self.read_matrix(Register::LLM_START);
        self.matrix_multiply_add(
            opcode,
            &vector,
            &light_matrix,
            &ZERO_VECTOR,
            MatrixMultiplyBehavior::Standard,
        );
    }

    // MAC = ((BK << 12) + LCM * IR) >> (sf * 12)
    // IR = MAC
    fn apply_light_color_matrix(&mut self, opcode: u32) {
        let ir_vector = self.read_ir_vector();
        let background_color = self.read_background_color();
        let light_color_matrix = self.read_matrix(Register::LCM_START);
        self.matrix_multiply_add(
            opcode,
            &ir_vector,
            &light_color_matrix,
            &background_color,
            MatrixMultiplyBehavior::Standard,
        );
    }

    // MAC = [R * IR1, G * IR2, B * IR3] << 4
    fn apply_color_vector(&mut self) {
        let [r, g, b] = fixedpoint::rgb(self.r[Register::RGBC]);
        let [ir1, ir2, ir3] = self.read_ir_vector();

        let mac1 = (r * ir1).shift_to::<4>();
        let mac2 = (g * ir2).shift_to::<4>();
        let mac3 = (b * ir3).shift_to::<4>();
        self.set_mac(mac1, mac2, mac3);
    }

    // IR = ((FC << 12) - MAC) >> (sf * 12)
    // MAC += IR * IR0
    fn apply_depth_cue(&mut self, opcode: u32) {
        let mac = self.read_mac_vector::<16>();
        let far_color = self.read_far_color().map(FixedPointDecimal::shift_to);

        let ir1 = far_color[0] - mac[0];
        let ir2 = far_color[1] - mac[1];
        let ir3 = far_color[2] - mac[2];

        if opcode.bit(gte::SF_BIT) {
            self.set_ir(ir1.shift_to::<4>(), ir2.shift_to::<4>(), ir3.shift_to::<4>(), false);
        } else {
            self.set_ir(ir1, ir2, ir3, false);
        }

        let ir0 = fixedpoint::ir0(self.r[Register::IR0]);
        let [ir1, ir2, ir3] = self.read_ir_vector();
        let [mac1, mac2, mac3] = mac.map(FixedPointDecimal::reinterpret);

        let mac1 = ir1 * ir0 + mac1;
        let mac2 = ir2 * ir0 + mac2;
        let mac3 = ir3 * ir0 + mac3;
        self.set_mac(mac1, mac2, mac3);
    }

    // MAC >>= (sf * 12)
    #[allow(clippy::redundant_closure_for_method_calls)]
    fn apply_mac_shift(&mut self, opcode: u32) {
        if !opcode.bit(gte::SF_BIT) {
            return;
        }

        let [mac1, mac2, mac3] = self.read_mac_vector::<12>().map(|mac| mac.shift_to::<0>());
        self.set_mac(mac1, mac2, mac3);
    }

    // ColorFifoPush([MAC1 >> 4, MAC2 >> 4, MAC3 >> 4, Code])
    // IR = MAC
    fn push_to_color_fifo(&mut self, opcode: u32) {
        let [mac1, mac2, mac3] = self.read_mac_vector::<4>();
        let [_, _, _, code] = self.r[Register::RGBC].to_le_bytes();

        let r = i64::from(mac1.shift_to::<0>()).clamp(0, 255) as u8;
        let g = i64::from(mac2.shift_to::<0>()).clamp(0, 255) as u8;
        let b = i64::from(mac3.shift_to::<0>()).clamp(0, 255) as u8;

        self.r[Register::RGB0] = self.r[Register::RGB1];
        self.r[Register::RGB1] = self.r[Register::RGB2];
        self.r[Register::RGB2] = u32::from_le_bytes([r, g, b, code]);

        self.set_ir(mac1, mac2, mac3, opcode.bit(gte::LM_BIT));
    }

    pub(super) fn read_background_color(&self) -> [Vector16Component; 3] {
        [
            fixedpoint::vector16_component(self.r[Register::RBK]),
            fixedpoint::vector16_component(self.r[Register::GBK]),
            fixedpoint::vector16_component(self.r[Register::BBK]),
        ]
    }

    fn read_far_color(&self) -> [FarColor; 3] {
        [
            fixedpoint::far_color(self.r[Register::RFC]),
            fixedpoint::far_color(self.r[Register::GFC]),
            fixedpoint::far_color(self.r[Register::BFC]),
        ]
    }
}
