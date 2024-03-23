//! GTE coordinate calculation instructions

use crate::cpu::gte;
use std::cmp;

use crate::cpu::gte::fixedpoint::{DivisionResult, FixedPointDecimal, ScreenCoordinate};
use crate::cpu::gte::registers::{Flag, Register};
use crate::cpu::gte::{fixedpoint, GeometryTransformationEngine, MatrixMultiplyBehavior};
use crate::num::U32Ext;

const U16_MIN: i32 = u16::MIN as i32;
const U16_MAX: i32 = u16::MAX as i32;

// Screen X/Y coordinates are saturated to signed 11-bit
const SCREEN_XY_MIN: i64 = -(1 << 10);
const SCREEN_XY_MAX: i64 = (1 << 10) - 1;

// IR0 is saturated to [$0000, $1000]
const IR0_MIN: i64 = 0;
const IR0_MAX: i64 = 0x1000;

impl GeometryTransformationEngine {
    // RTPS: Perspective transformation, single
    // Applies perspective transformation to V0
    // Coordinates are written to the screen X/Y/Z FIFOs, and the depth cueing interpolation factor
    // is written to IR0
    pub(super) fn rtps(&mut self, opcode: u32) {
        log::trace!("GTE RTPS: {opcode:08X}");

        let translation = self.read_translation_vector();
        let rotation = self.read_matrix(Register::RT1112);
        let v0 = self.read_vector16_packed(Register::VXY0, Register::VZ0);

        self.matrix_multiply_add(opcode, &v0, &rotation, &translation, MatrixMultiplyBehavior::Rtp);
        self.perform_perspective_transformation(opcode);
    }

    // RTPT: Perspective transformation, triple
    // Equivalent to RTPS but processes V1 and V2 in addition to V0
    pub(super) fn rtpt(&mut self, opcode: u32) {
        log::trace!("GTE RTPT: {opcode:08X}");

        let translation = self.read_translation_vector();
        let rotation = self.read_matrix(Register::RT1112);

        let v0 = self.read_vector16_packed(Register::VXY0, Register::VZ0);
        self.matrix_multiply_add(opcode, &v0, &rotation, &translation, MatrixMultiplyBehavior::Rtp);
        self.perform_perspective_transformation(opcode);

        let v1 = self.read_vector16_packed(Register::VXY1, Register::VZ1);
        self.matrix_multiply_add(opcode, &v1, &rotation, &translation, MatrixMultiplyBehavior::Rtp);
        self.perform_perspective_transformation(opcode);

        let v2 = self.read_vector16_packed(Register::VXY2, Register::VZ2);
        self.matrix_multiply_add(opcode, &v2, &rotation, &translation, MatrixMultiplyBehavior::Rtp);
        self.perform_perspective_transformation(opcode);
    }

    // NCLIP: Normal clipping
    // Computes the Z component of the cross product of the 3 screen X/Y coordinates in the FIFO,
    // and writes the result to MAC0
    pub(super) fn nclip(&mut self) {
        log::trace!("GTE NCLIP");

        let (sx0, sy0) = self.read_screen_xy(Register::SXY0);
        let (sx1, sy1) = self.read_screen_xy(Register::SXY1);
        let (sx2, sy2) = self.read_screen_xy(Register::SXY2);

        let mac0 = sx0 * sy1 + sx1 * sy2 + sx2 * sy0 - sx0 * sy2 - sx1 * sy0 - sx2 * sy1;
        self.check_mac0_overflow(mac0);
        self.r[Register::MAC0] = i64::from(mac0) as u32;
    }

    // AVSZ3: Average of three Z values
    // Averages the last three screen Z coordinates in the FIFO using the specified scale factor (ZSF3),
    // and writes the result to MAC0 and OTZ
    pub(super) fn avsz3(&mut self) {
        log::trace!("GTE AVSZ3");

        let sz1 = fixedpoint::screen_z(self.r[Register::SZ1]);
        let sz2 = fixedpoint::screen_z(self.r[Register::SZ2]);
        let sz3 = fixedpoint::screen_z(self.r[Register::SZ3]);
        let zsf3 = fixedpoint::z_scale_factor(self.r[Register::ZSF3]);

        let mac0 = zsf3 * (sz1 + sz2 + sz3);
        self.check_mac0_overflow(mac0);
        self.r[Register::MAC0] = i64::from(mac0) as u32;

        self.set_otz();
    }

    // AVSZ4: Average of four Z values
    // Averages all four screen Z coordinates in he FIFO using the specified scale factor (ZSF4), and
    // writes the result to MAC0 and OTZ
    pub(super) fn avsz4(&mut self) {
        log::trace!("GTE AVSZ4");

        let sz0 = fixedpoint::screen_z(self.r[Register::SZ0]);
        let sz1 = fixedpoint::screen_z(self.r[Register::SZ1]);
        let sz2 = fixedpoint::screen_z(self.r[Register::SZ2]);
        let sz3 = fixedpoint::screen_z(self.r[Register::SZ3]);
        let zsf4 = fixedpoint::z_scale_factor(self.r[Register::ZSF4]);

        let mac0 = zsf4 * (sz0 + sz1 + sz2 + sz3);
        self.check_mac0_overflow(mac0);
        self.r[Register::MAC0] = i64::from(mac0) as u32;

        self.set_otz();
    }

    fn set_otz(&mut self) {
        let otz = (self.r[Register::MAC0] as i32) >> 12;
        let clamped_otz = otz.clamp(U16_MIN, U16_MAX);
        if otz != clamped_otz {
            self.r[Register::FLAG] |= Flag::SZ3_OTZ_SATURATED | Flag::ERROR;
        }
        self.r[Register::OTZ] = clamped_otz as u32;
    }

    fn perform_perspective_transformation(&mut self, opcode: u32) {
        let sf = opcode.bit(gte::SF_BIT);
        let sz3 = self.mac[3] >> (12 * (1 - u8::from(sf)));

        let clamped_sz3 = sz3.clamp(U16_MIN.into(), U16_MAX.into());
        if sz3 != clamped_sz3 {
            self.r[Register::FLAG] |= Flag::SZ3_OTZ_SATURATED | Flag::ERROR;
        }

        self.push_screen_z(clamped_sz3 as u16);

        let ir1 = fixedpoint::vector16_component(self.r[Register::IR1]);
        let ir2 = fixedpoint::vector16_component(self.r[Register::IR2]);

        let ofx = fixedpoint::screen_offset(self.r[Register::OFX]);
        let ofy = fixedpoint::screen_offset(self.r[Register::OFY]);

        let mac0 = gte_divide(&mut self.r) * ir1 + ofx;
        self.check_mac0_overflow(mac0);
        let sx = mac0.shift_to::<0>();

        let mac0 = gte_divide(&mut self.r) * ir2 + ofy;
        self.check_mac0_overflow(mac0);
        let sy = mac0.shift_to::<0>();

        self.push_screen_xy(sx, sy);

        let dqa = fixedpoint::dqa(self.r[Register::DQA]);
        let dqb = fixedpoint::dqb(self.r[Register::DQB]);

        let mac0 = gte_divide(&mut self.r) * dqa + dqb;
        self.check_mac0_overflow(mac0);
        self.r[Register::MAC0] = i64::from(mac0) as u32;

        let ir0 = i64::from(mac0.shift_to::<12>());
        let clamped_ir0 = ir0.clamp(IR0_MIN, IR0_MAX);
        if ir0 != clamped_ir0 {
            self.r[Register::FLAG] |= Flag::IR0_SATURATED;
        }
        self.r[Register::IR0] = clamped_ir0 as u32;
    }

    fn push_screen_xy(&mut self, sx: FixedPointDecimal<0>, sy: FixedPointDecimal<0>) {
        let sx = i64::from(sx);
        let sy = i64::from(sy);

        let clamped_sx = sx.clamp(SCREEN_XY_MIN, SCREEN_XY_MAX);
        if sx != clamped_sx {
            self.r[Register::FLAG] |= Flag::SX2_SATURATED | Flag::ERROR;
        }

        let clamped_sy = sy.clamp(SCREEN_XY_MIN, SCREEN_XY_MAX);
        if sy != clamped_sy {
            self.r[Register::FLAG] |= Flag::SY2_SATURATED | Flag::ERROR;
        }

        let sxy = u32::from(clamped_sx as u16) | (u32::from(clamped_sy as u16) << 16);

        self.r[Register::SXY0] = self.r[Register::SXY1];
        self.r[Register::SXY1] = self.r[Register::SXY2];
        self.r[Register::SXY2] = sxy;
    }

    fn push_screen_z(&mut self, sz3: u16) {
        self.r[Register::SZ0] = self.r[Register::SZ1];
        self.r[Register::SZ1] = self.r[Register::SZ2];
        self.r[Register::SZ2] = self.r[Register::SZ3];
        self.r[Register::SZ3] = sz3.into();
    }

    fn read_screen_xy(&self, xy_register: usize) -> (ScreenCoordinate, ScreenCoordinate) {
        let value = self.r[xy_register];
        let sx = fixedpoint::screen_xy(value);
        let sy = fixedpoint::screen_xy(value >> 16);

        (sx, sy)
    }
}

const GTE_UNR_TABLE: &[u8; 257] = &[
    0xFF, 0xFD, 0xFB, 0xF9, 0xF7, 0xF5, 0xF3, 0xF1, 0xEF, 0xEE, 0xEC, 0xEA, 0xE8, 0xE6, 0xE4, 0xE3,
    0xE1, 0xDF, 0xDD, 0xDC, 0xDA, 0xD8, 0xD6, 0xD5, 0xD3, 0xD1, 0xD0, 0xCE, 0xCD, 0xCB, 0xC9, 0xC8,
    0xC6, 0xC5, 0xC3, 0xC1, 0xC0, 0xBE, 0xBD, 0xBB, 0xBA, 0xB8, 0xB7, 0xB5, 0xB4, 0xB2, 0xB1, 0xB0,
    0xAE, 0xAD, 0xAB, 0xAA, 0xA9, 0xA7, 0xA6, 0xA4, 0xA3, 0xA2, 0xA0, 0x9F, 0x9E, 0x9C, 0x9B, 0x9A,
    0x99, 0x97, 0x96, 0x95, 0x94, 0x92, 0x91, 0x90, 0x8F, 0x8D, 0x8C, 0x8B, 0x8A, 0x89, 0x87, 0x86,
    0x85, 0x84, 0x83, 0x82, 0x81, 0x7F, 0x7E, 0x7D, 0x7C, 0x7B, 0x7A, 0x79, 0x78, 0x77, 0x75, 0x74,
    0x73, 0x72, 0x71, 0x70, 0x6F, 0x6E, 0x6D, 0x6C, 0x6B, 0x6A, 0x69, 0x68, 0x67, 0x66, 0x65, 0x64,
    0x63, 0x62, 0x61, 0x60, 0x5F, 0x5E, 0x5D, 0x5D, 0x5C, 0x5B, 0x5A, 0x59, 0x58, 0x57, 0x56, 0x55,
    0x54, 0x53, 0x53, 0x52, 0x51, 0x50, 0x4F, 0x4E, 0x4D, 0x4D, 0x4C, 0x4B, 0x4A, 0x49, 0x48, 0x48,
    0x47, 0x46, 0x45, 0x44, 0x43, 0x43, 0x42, 0x41, 0x40, 0x3F, 0x3F, 0x3E, 0x3D, 0x3C, 0x3C, 0x3B,
    0x3A, 0x39, 0x39, 0x38, 0x37, 0x36, 0x36, 0x35, 0x34, 0x33, 0x33, 0x32, 0x31, 0x31, 0x30, 0x2F,
    0x2E, 0x2E, 0x2D, 0x2C, 0x2C, 0x2B, 0x2A, 0x2A, 0x29, 0x28, 0x28, 0x27, 0x26, 0x26, 0x25, 0x24,
    0x24, 0x23, 0x22, 0x22, 0x21, 0x20, 0x20, 0x1F, 0x1E, 0x1E, 0x1D, 0x1D, 0x1C, 0x1B, 0x1B, 0x1A,
    0x19, 0x19, 0x18, 0x18, 0x17, 0x16, 0x16, 0x15, 0x15, 0x14, 0x14, 0x13, 0x12, 0x12, 0x11, 0x11,
    0x10, 0x0F, 0x0F, 0x0E, 0x0E, 0x0D, 0x0D, 0x0C, 0x0C, 0x0B, 0x0A, 0x0A, 0x09, 0x09, 0x08, 0x08,
    0x07, 0x07, 0x06, 0x06, 0x05, 0x05, 0x04, 0x04, 0x03, 0x03, 0x02, 0x02, 0x01, 0x01, 0x00, 0x00,
    0x00,
];

// Perform (((H << 17) / SZ3) + 1) / 2
// Used by RTPS and RTPT instructions
#[must_use]
#[allow(clippy::many_single_char_names)]
fn gte_divide(r: &mut [u32; 64]) -> DivisionResult {
    let h = r[Register::H] & 0xFFFF;
    let sz3 = r[Register::SZ3] & 0xFFFF;

    if h >= 2 * sz3 {
        // Result will overflow, saturate to $1FFFF and set divide overflow flag
        r[Register::FLAG] |= Flag::DIVIDE_OVERFLOW | Flag::ERROR;
        return fixedpoint::division_result(0x1FFFF);
    }

    let z = (sz3 as u16).leading_zeros();
    let n: u64 = (h << z).into();
    let d = sz3 << z;
    let u = u32::from(GTE_UNR_TABLE[((d - 0x7FC0) >> 7) as usize]) + 0x101;
    let d = (0x2000080 - (d * u)) >> 8;
    let d: u64 = ((0x0000080 + (d * u)) >> 8).into();

    let result = cmp::min(0x1FFFF, ((n * d) + 0x8000) >> 16) as u32;
    fixedpoint::division_result(result)
}
