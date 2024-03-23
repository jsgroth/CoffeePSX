//! GTE color calculation instructions

use crate::cpu::gte;
use crate::cpu::gte::fixedpoint::{FarColor, FixedPointDecimal, Vector16Component};
use crate::cpu::gte::registers::{Flag, Register};
use crate::cpu::gte::{fixedpoint, GeometryTransformationEngine, MatrixMultiplyBehavior};
use crate::num::U32Ext;

const ZERO_VECTOR: [FixedPointDecimal<0>; 3] =
    [FixedPointDecimal::ZERO, FixedPointDecimal::ZERO, FixedPointDecimal::ZERO];

impl GeometryTransformationEngine {
    // NCCS: Normal color, single vector
    // Performs color calculation on the vector V0 with no depth cueing
    pub(super) fn nccs(&mut self, opcode: u32) {
        log::trace!("GTE NCCS {opcode:08X}");

        self.apply_light_matrix(opcode, Register::VXY0, Register::VZ0);
        self.apply_light_color_matrix(opcode);
        self.apply_color_vector();
        self.apply_mac_shift(opcode);
        self.push_to_color_fifo(opcode);
    }

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

    // DPCS: Depth cue, single vector
    pub(super) fn dpcs(&mut self, opcode: u32) {
        // MAC = RGB << 16
        let [mac1, mac2, mac3, _] = self.r[Register::RGBC]
            .to_le_bytes()
            .map(|n| FixedPointDecimal::<0>::new(n.into()).shift_to::<16>());
        self.set_mac(mac1, mac2, mac3);

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

        // Overflows in the (FC - MAC) calculation will set MAC overflow flags in the FLAG register
        let _ = self.check_mac123_overflow(
            ir1,
            Flag::MAC1_OVERFLOW_POSITIVE,
            Flag::MAC1_OVERFLOW_NEGATIVE,
        );
        let _ = self.check_mac123_overflow(
            ir2,
            Flag::MAC2_OVERFLOW_POSITIVE,
            Flag::MAC2_OVERFLOW_NEGATIVE,
        );
        let _ = self.check_mac123_overflow(
            ir3,
            Flag::MAC3_OVERFLOW_POSITIVE,
            Flag::MAC3_OVERFLOW_NEGATIVE,
        );

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

        let [r, g, b] = [mac1, mac2, mac3].map(|mac| i64::from(mac.shift_to::<0>()));

        let clamped_r = r.clamp(0, 255);
        if r != clamped_r {
            self.r[Register::FLAG] |= Flag::COLOR_R_SATURATED;
        }

        let clamped_g = g.clamp(0, 255);
        if g != clamped_g {
            self.r[Register::FLAG] |= Flag::COLOR_G_SATURATED;
        }

        let clamped_b = b.clamp(0, 255);
        if b != clamped_b {
            self.r[Register::FLAG] |= Flag::COLOR_B_SATURATED;
        }

        self.r[Register::RGB0] = self.r[Register::RGB1];
        self.r[Register::RGB1] = self.r[Register::RGB2];
        self.r[Register::RGB2] =
            u32::from_le_bytes([clamped_r as u8, clamped_g as u8, clamped_b as u8, code]);

        self.set_ir(mac1, mac2, mac3, opcode.bit(gte::LM_BIT));
    }

    // GPF: General-purpose interpolation
    // Interpolates the current contents of the IR vector and pushes to the color FIFO
    pub(super) fn gpf(&mut self, opcode: u32) {
        self.set_mac::<0>(
            FixedPointDecimal::ZERO,
            FixedPointDecimal::ZERO,
            FixedPointDecimal::ZERO,
        );
        self.general_purpose_interpolation(opcode);
    }

    // GPL General-purpose interpolation with base
    // Interpolates the current contents of the IR vector, accumulates into the MAC vector, and
    // pushes to the color FIFO
    #[allow(clippy::redundant_closure_for_method_calls)]
    pub(super) fn gpl(&mut self, opcode: u32) {
        if opcode.bit(gte::SF_BIT) {
            let [mac1, mac2, mac3] = self.read_mac_vector::<0>().map(|mac| mac.shift_to::<12>());
            self.set_mac(mac1, mac2, mac3);
        }

        self.general_purpose_interpolation(opcode);
    }

    fn general_purpose_interpolation(&mut self, opcode: u32) {
        let ir0 = fixedpoint::ir0(self.r[Register::IR0]);
        let [ir1, ir2, ir3] = self.read_ir_vector();
        let [mac1, mac2, mac3] = self.read_mac_vector::<12>();

        let mac1 = ir1 * ir0 + mac1;
        let mac2 = ir2 * ir0 + mac2;
        let mac3 = ir3 * ir0 + mac3;

        if opcode.bit(gte::SF_BIT) {
            self.set_mac(mac1.shift_to::<0>(), mac2.shift_to::<0>(), mac3.shift_to::<0>());
        } else {
            self.set_mac(mac1, mac2, mac3);
        }

        self.push_to_color_fifo(opcode);
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
