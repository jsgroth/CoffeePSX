//! Geometry Transformation Engine (GTE), a 3D math coprocessor

mod coordcalc;
mod fixedpoint;
mod registers;

use crate::cpu::gte::registers::Register;
use crate::num::U32Ext;
use bincode::{Decode, Encode};
use std::array;

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
            0x30 => self.rtpt(opcode),
            _ => log::warn!("Unimplemented GTE command {command:02X} {opcode:08X}"),
        }
    }
}

fn sign_extend_i16(value: u32) -> u32 {
    value as i16 as u32
}
