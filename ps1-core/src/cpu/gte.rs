//! Geometry Transformation Engine (GTE), a 3D math coprocessor

mod colors;
mod coordinates;
mod fixedpoint;
mod registers;

use crate::cpu::gte::fixedpoint::{
    FixedPointDecimal, MatrixComponent, TranslationComponent, Vector16Component,
};
use crate::cpu::gte::registers::{Flag, Register};
use crate::num::U32Ext;
use bincode::{Decode, Encode};
use std::array;

const SF_BIT: u8 = 19;
const LM_BIT: u8 = 10;

const I16_MIN: i32 = i16::MIN as i32;
const I16_MAX: i32 = i16::MAX as i32;

const I32_MIN: i64 = i32::MIN as i64;
const I32_MAX: i64 = i32::MAX as i64;

// Min/max values for multiply-add results
// The results wrap instead of saturating, but overflow flags are set when they wrap
const I44_MIN: i64 = -(1 << 43);
const I44_MAX: i64 = (1 << 43) - 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MatrixMultiplyBehavior {
    Rtp,
    Standard,
}

#[derive(Debug, Clone, Encode, Decode)]
pub struct GeometryTransformationEngine {
    r: [u32; 64],
}

impl GeometryTransformationEngine {
    pub fn new() -> Self {
        Self { r: array::from_fn(|_| 0) }
    }

    pub fn load_word(&mut self, register: u32, value: u32) {
        // TODO load delay?
        self.write_register(register, value);
    }

    pub fn read_register(&self, register: u32) -> u32 {
        let value = match register {
            // VZ0, VZ1, VZ2, IR0, IR1, IR2, IR3 are all signed 16-bit
            1 | 3 | 5 | 8 | 9 | 10 | 11 => sign_extend_i16(self.r[register as usize]),
            // SXYP, mirrors SXY2 on reads
            15 => self.r[Register::SXY2],
            // IRGB and ORGB, return converted colors from IR1/IR2/IR3
            28 | 29 => {
                todo!("IRGB/ORGB read")
            }
            // LZCR, reading returns the number of leading bits in LZCS
            31 => {
                let lzcs = self.r[Register::LZCS];
                if lzcs.sign_bit() { lzcs.leading_ones() } else { lzcs.leading_zeros() }
            }
            _ => self.r[register as usize],
        };

        log::trace!("GTE register read: R{register} ({value:08X}) ({})", Register::name(register));

        value
    }

    pub fn write_register(&mut self, register: u32, value: u32) {
        log::trace!("GTE register write: R{register} = {value:08X} ({})", Register::name(register));

        match register {
            // OTZ, ORGB, LZCR are read-only
            7 | 29 | 31 => {}
            // SXYP, writing shifts the screen X/Y FIFO in addition to writing SXY2/SXYP
            15 => {
                self.r[Register::SXY0] = self.r[Register::SXY1];
                self.r[Register::SXY1] = self.r[Register::SXY2];
                self.r[Register::SXY2] = value;
            }
            // IRGB, writing triggers a color conversion operation into IR1/IR2/IR3
            28 => {
                todo!("IRGB write {value:08X}")
            }
            _ => self.r[register as usize] = value,
        }
    }

    pub fn read_control_register(&self, control_register: u32) -> u32 {
        let register = 32 | control_register;

        let value = match register {
            // RT33, LLM33, LCM33, H, DQA, ZSF3, ZSF4 are all signed 16-bit
            // H is technically unsigned 16-bit, but the hardware sign extends it on register reads
            36 | 44 | 52 | 58 | 59 | 61 | 62 => sign_extend_i16(self.r[register as usize]),
            _ => self.r[register as usize],
        };

        log::trace!(
            "GTE control register read: R{register} ({value:08X}) ({})",
            Register::name(register)
        );

        value
    }

    pub fn write_control_register(&mut self, control_register: u32, value: u32) {
        let register = 32 | control_register;
        log::trace!(
            "GTE control register write: R{register} = {value:08X} ({})",
            Register::name(register)
        );

        match register {
            // FLAG, only bits 30-12 are writable and bit 31 is always the OR of bits 30-23 and 18-13
            63 => {
                self.r[Register::FLAG] = value & 0x7FFFF000;
                self.r[Register::FLAG] |= u32::from(value & 0x7F87E000 != 0) << 31;
            }
            _ => self.r[register as usize] = value,
        }
    }

    pub fn execute_opcode(&mut self, opcode: u32) {
        log::trace!("GTE opcode: {opcode:08X}");

        self.r[Register::FLAG] = 0;

        let command = opcode & 0x3F;
        match command {
            0x01 => self.rtps(opcode),
            0x06 => self.nclip(),
            0x13 => self.ncds(opcode),
            0x2D => self.avsz3(),
            0x2E => self.avsz4(),
            0x30 => self.rtpt(opcode),
            _ => log::warn!("Unimplemented GTE command {command:02X} {opcode:08X}"),
        }
    }

    fn matrix_multiply_add(
        &mut self,
        opcode: u32,
        vector: &[Vector16Component; 3],
        matrix: &[[MatrixComponent; 3]; 3],
        translation: &[TranslationComponent; 3],
        behavior: MatrixMultiplyBehavior,
    ) {
        let mac1 = translation[0].shift_to::<12>()
            + matrix[0][0] * vector[0]
            + matrix[0][1] * vector[1]
            + matrix[0][2] * vector[2];
        let mac2 = translation[1].shift_to::<12>()
            + matrix[1][0] * vector[0]
            + matrix[1][1] * vector[1]
            + matrix[1][2] * vector[2];
        let mac3 = translation[2].shift_to::<12>()
            + matrix[2][0] * vector[0]
            + matrix[2][1] * vector[1]
            + matrix[2][2] * vector[2];

        let mac1 = self.check_mac123_overflow(
            mac1,
            Flag::MAC1_OVERFLOW_POSITIVE,
            Flag::MAC1_OVERFLOW_NEGATIVE,
        );
        let mac2 = self.check_mac123_overflow(
            mac2,
            Flag::MAC2_OVERFLOW_POSITIVE,
            Flag::MAC2_OVERFLOW_NEGATIVE,
        );
        let mac3 = self.check_mac123_overflow(
            mac3,
            Flag::MAC3_OVERFLOW_POSITIVE,
            Flag::MAC3_OVERFLOW_NEGATIVE,
        );

        let sf = opcode.bit(SF_BIT);
        if sf {
            self.r[Register::MAC1] = (i64::from(mac1) >> 12) as u32;
            self.r[Register::MAC2] = (i64::from(mac2) >> 12) as u32;
            self.r[Register::MAC3] = (i64::from(mac3) >> 12) as u32;
        } else {
            self.r[Register::MAC1] = i64::from(mac1) as u32;
            self.r[Register::MAC2] = i64::from(mac2) as u32;
            self.r[Register::MAC3] = i64::from(mac3) as u32;
        }

        // RTPS/RTPT always behave as if lm=0
        let lm = behavior == MatrixMultiplyBehavior::Standard && opcode.bit(LM_BIT);

        self.set_ir_component(Register::IR1, self.r[Register::MAC1], Flag::IR1_SATURATED, lm);
        self.set_ir_component(Register::IR2, self.r[Register::MAC2], Flag::IR2_SATURATED, lm);

        match behavior {
            MatrixMultiplyBehavior::Rtp if !sf => {
                // Apparent hardware bug: When sf=0, IR3 saturation flag is set based on
                // (MAC3 >> 12) instead of MAC3
                let value = self.r[Register::MAC3] as i32;
                if !(I16_MIN..=I16_MAX).contains(&(value >> 12)) {
                    self.r[Register::FLAG] |= Flag::IR3_SATURATED;
                }

                let min = if lm { 0 } else { I16_MIN };
                self.r[Register::IR3] = value.clamp(min, I16_MAX) as u32;
            }
            _ => {
                self.set_ir_component(
                    Register::IR3,
                    self.r[Register::MAC3],
                    Flag::IR3_SATURATED,
                    lm,
                );
            }
        }
    }

    fn check_mac0_overflow<const FRACTION_BITS: u8>(
        &mut self,
        value: FixedPointDecimal<FRACTION_BITS>,
    ) {
        let value = i64::from(value);
        if !(I32_MIN..=I32_MAX).contains(&value) {
            self.r[Register::FLAG] |=
                if value < 0 { Flag::MAC0_OVERFLOW_NEGATIVE } else { Flag::MAC0_OVERFLOW_POSITIVE };
            self.r[Register::FLAG] |= Flag::ERROR;
        }
    }

    #[must_use]
    fn check_mac123_overflow<const FRACTION_BITS: u8>(
        &mut self,
        value: FixedPointDecimal<FRACTION_BITS>,
        positive_bit: u32,
        negative_bit: u32,
    ) -> FixedPointDecimal<FRACTION_BITS> {
        let raw_value = i64::from(value);
        if (I44_MIN..=I44_MAX).contains(&raw_value) {
            return value;
        }

        self.r[Register::FLAG] |= if raw_value < 0 { negative_bit } else { positive_bit };
        self.r[Register::FLAG] |= Flag::ERROR;

        FixedPointDecimal::new((raw_value << 20) >> 20)
    }

    fn set_ir_component(&mut self, register: usize, value: u32, saturation_bit: u32, lm: bool) {
        // IR1-3 are clamped to the i16 range if lm=0 and to [$0000, $7FFF] if lm=1
        let value = value as i32;
        let min = if lm { 0 } else { I16_MIN };
        let clamped = value.clamp(min, I16_MAX);

        if value != clamped {
            self.r[Register::FLAG] |= saturation_bit;
            if register != Register::IR3 {
                self.r[Register::FLAG] |= Flag::ERROR;
            }
        }

        self.r[register] = clamped as u32;
    }

    fn set_ir<const FRACTION_BITS: u8>(
        &mut self,
        ir1: FixedPointDecimal<FRACTION_BITS>,
        ir2: FixedPointDecimal<FRACTION_BITS>,
        ir3: FixedPointDecimal<FRACTION_BITS>,
        lm: bool,
    ) {
        self.set_ir_component(Register::IR1, i64::from(ir1) as u32, Flag::IR1_SATURATED, lm);
        self.set_ir_component(Register::IR2, i64::from(ir2) as u32, Flag::IR2_SATURATED, lm);
        self.set_ir_component(Register::IR3, i64::from(ir3) as u32, Flag::IR3_SATURATED, lm);
    }

    fn set_mac<const FRACTION_BITS: u8>(
        &mut self,
        mac1: FixedPointDecimal<FRACTION_BITS>,
        mac2: FixedPointDecimal<FRACTION_BITS>,
        mac3: FixedPointDecimal<FRACTION_BITS>,
    ) {
        let mac1 = self.check_mac123_overflow(
            mac1,
            Flag::MAC1_OVERFLOW_POSITIVE,
            Flag::MAC1_OVERFLOW_NEGATIVE,
        );
        self.r[Register::MAC1] = i64::from(mac1) as u32;

        let mac2 = self.check_mac123_overflow(
            mac2,
            Flag::MAC2_OVERFLOW_POSITIVE,
            Flag::MAC2_OVERFLOW_NEGATIVE,
        );
        self.r[Register::MAC2] = i64::from(mac2) as u32;

        let mac3 = self.check_mac123_overflow(
            mac3,
            Flag::MAC3_OVERFLOW_POSITIVE,
            Flag::MAC3_OVERFLOW_NEGATIVE,
        );
        self.r[Register::MAC3] = i64::from(mac3) as u32;
    }

    fn read_matrix(&self, base_register: usize) -> [[MatrixComponent; 3]; 3] {
        [
            [
                fixedpoint::matrix_component(self.r[base_register]),
                fixedpoint::matrix_component(self.r[base_register] >> 16),
                fixedpoint::matrix_component(self.r[base_register + 1]),
            ],
            [
                fixedpoint::matrix_component(self.r[base_register + 1] >> 16),
                fixedpoint::matrix_component(self.r[base_register + 2]),
                fixedpoint::matrix_component(self.r[base_register + 2] >> 16),
            ],
            [
                fixedpoint::matrix_component(self.r[base_register + 3]),
                fixedpoint::matrix_component(self.r[base_register + 3] >> 16),
                fixedpoint::matrix_component(self.r[base_register + 4]),
            ],
        ]
    }

    fn read_vector16_packed(
        &self,
        xy_register: usize,
        z_register: usize,
    ) -> [Vector16Component; 3] {
        [
            fixedpoint::vector16_component(self.r[xy_register]),
            fixedpoint::vector16_component(self.r[xy_register] >> 16),
            fixedpoint::vector16_component(self.r[z_register]),
        ]
    }

    fn read_vector16_unpacked(
        &self,
        x_register: usize,
        y_register: usize,
        z_register: usize,
    ) -> [Vector16Component; 3] {
        [
            fixedpoint::vector16_component(self.r[x_register]),
            fixedpoint::vector16_component(self.r[y_register]),
            fixedpoint::vector16_component(self.r[z_register]),
        ]
    }
}

fn sign_extend_i16(value: u32) -> u32 {
    value as i16 as u32
}
