use crate::bus::BusInterface;
use crate::R3000;

trait U32Ext {
    fn bit(self, i: u8) -> bool;

    fn sign_bit(self) -> bool;
}

impl U32Ext for u32 {
    fn bit(self, i: u8) -> bool {
        self & (1 << i) != 0
    }

    fn sign_bit(self) -> bool {
        self.bit(31)
    }
}

macro_rules! impl_branch {
    ($name:ident, |$rs:ident $(, $rt:ident)?| $cond:expr $(, link: $link:literal)?) => {
        fn $name(&mut self, opcode: u32) {
            let $rs = self.registers.gpr[parse_rs(opcode) as usize];
            $(
                let $rt = self.registers.gpr[parse_rt(opcode) as usize];
            )?
            if $cond {
                let offset = parse_offset(opcode);
                let target = self.registers.pc.wrapping_add(offset);
                self.registers.delayed_branch = Some(target);
            }

            $(
                if $link {
                    self.registers.gpr[31] = self.registers.pc.wrapping_add(4);
                }
            )?
        }
    }
}

impl R3000 {
    pub(super) fn execute_opcode<B: BusInterface>(&mut self, opcode: u32, bus: &mut B) {
        // First 6 bits of opcode identify operation
        match opcode >> 26 {
            // If highest 6 bits are all 0, the lowest 6 bits are used to specify the operation
            0x00 => match opcode & 0x3F {
                0x0D => todo!("BREAK opcode"),
                0x08 => self.jr(opcode),
                0x09 => self.jalr(opcode),
                0x1A => self.div(opcode),
                0x1B => self.divu(opcode),
                0x20 => self.add(opcode),
                0x21 => self.addu(opcode),
                0x24 => self.and(opcode),
                _ => todo!("opcode {opcode:08X}"),
            },
            // If highest 6 bits are $01, bits 16-20 are used to specify the operation
            0x01 => match (opcode >> 16) & 0x1F {
                0x00 => self.bltz(opcode),
                0x01 => self.bgez(opcode),
                0x10 => self.bltzal(opcode),
                0x11 => self.bgezal(opcode),
                _ => todo!("opcode {opcode:08X}"),
            },
            0x02 => self.j(opcode),
            0x03 => self.jal(opcode),
            0x04 => self.beq(opcode),
            0x05 => self.bne(opcode),
            0x06 => self.blez(opcode),
            0x07 => self.bgtz(opcode),
            0x08 => self.addi(opcode),
            0x09 => self.addiu(opcode),
            0x0C => self.andi(opcode),
            0x0F => self.lui(opcode),
            // If highest 6 bits are $10-$13, this is a coprocessor opcode and bits 21-25 specify
            // the operation
            0x10..=0x13 => match (opcode >> 21) & 0x1F {
                0x02 => todo!("CFCz opcode {opcode:08X}"),
                0x06 => todo!("CTCz opcode {opcode:08X}"),
                0x10..=0x1F => todo!("COPz opcode {opcode:08X}"),
                _ => todo!("coprocessor opcode {opcode:08X}"),
            },
            _ => todo!("opcode {opcode:08X}"),
        }
    }

    // ADD: Add word
    fn add(&mut self, opcode: u32) {
        let operand_l = self.registers.gpr[parse_rs(opcode) as usize];
        let operand_r = self.registers.gpr[parse_rt(opcode) as usize];
        let (sum, overflowed) = (operand_l as i32).overflowing_add(operand_r as i32);
        if overflowed {
            todo!("integer overflow exception")
        }

        self.registers.write_gpr(parse_rd(opcode), sum as u32);
    }

    // ADDU: Add unsigned word
    fn addu(&mut self, opcode: u32) {
        let operand_l = self.registers.gpr[parse_rs(opcode) as usize];
        let operand_r = self.registers.gpr[parse_rt(opcode) as usize];
        self.registers
            .write_gpr(parse_rd(opcode), operand_l.wrapping_add(operand_r));
    }

    // ADDI: Add immediate word
    fn addi(&mut self, opcode: u32) {
        let operand_l = self.registers.gpr[parse_rs(opcode) as usize] as i32;
        let operand_r = parse_signed_immediate(opcode);
        let (sum, overflowed) = operand_l.overflowing_add(operand_r);
        if overflowed {
            todo!("integer overflow exception")
        }

        self.registers.write_gpr(parse_rt(opcode), sum as u32);
    }

    // ADDIU: Add immediate unsigned word
    fn addiu(&mut self, opcode: u32) {
        let operand_l = self.registers.gpr[parse_rs(opcode) as usize];
        let operand_r = parse_signed_immediate(opcode) as u32;
        self.registers
            .write_gpr(parse_rt(opcode), operand_l.wrapping_add(operand_r));
    }

    // AND: And
    fn and(&mut self, opcode: u32) {
        let operand_l = self.registers.gpr[parse_rs(opcode) as usize];
        let operand_r = self.registers.gpr[parse_rt(opcode) as usize];
        self.registers
            .write_gpr(parse_rd(opcode), operand_l & operand_r);
    }

    // ANDI: And immediate
    fn andi(&mut self, opcode: u32) {
        let operand_l = self.registers.gpr[parse_rs(opcode) as usize];
        let operand_r = parse_unsigned_immediate(opcode);
        self.registers
            .write_gpr(parse_rt(opcode), operand_l & operand_r);
    }

    // BEQ: Branch on equal
    impl_branch!(beq, |rs, rt| rs == rt);

    // BNE: Branch on not equal
    impl_branch!(bne, |rs, rt| rs != rt);

    // BGEZ: Branch on greater than or equal to zero
    impl_branch!(bgez, |rs| !rs.sign_bit());

    // BGEZAL: Branch on greater than or equal to zero and link
    impl_branch!(bgezal, |rs| !rs.sign_bit(), link: true);

    // BGTZ: Branch on greater than zero
    impl_branch!(bgtz, |rs| rs != 0 && !rs.sign_bit());

    // BLEZ: Branch on less than or equal to zero
    impl_branch!(blez, |rs| rs == 0 || rs.sign_bit());

    // BLTZ: Branch on less than zero
    impl_branch!(bltz, |rs| rs.sign_bit());

    // BLTZAL: Branch on less than zero and link
    impl_branch!(bltzal, |rs| rs.sign_bit(), link: true);

    // DIV: Divide word
    fn div(&mut self, opcode: u32) {
        // TODO timing?
        let dividend = self.registers.gpr[parse_rs(opcode) as usize] as i32;
        let divisor = self.registers.gpr[parse_rt(opcode) as usize] as i32;
        if divisor == 0 {
            // TODO divide by zero behavior?
            self.registers.lo = u32::MAX;
            self.registers.hi = divisor as u32;
            return;
        }

        self.registers.lo = dividend.wrapping_div(divisor) as u32;
        self.registers.hi = dividend.wrapping_rem(divisor) as u32;
    }

    // DIVU: Divide unsigned word
    fn divu(&mut self, opcode: u32) {
        // TODO timing?
        let dividend = self.registers.gpr[parse_rs(opcode) as usize];
        let divisor = self.registers.gpr[parse_rt(opcode) as usize];
        if divisor == 0 {
            // TODO divide by zero behavior?
            self.registers.lo = u32::MAX;
            self.registers.hi = divisor;
            return;
        }

        self.registers.lo = dividend.wrapping_div(divisor);
        self.registers.hi = dividend.wrapping_rem(divisor);
    }

    // J: Jump
    fn j(&mut self, opcode: u32) {
        self.registers.delayed_branch = Some(compute_jump_address(self.registers.pc, opcode));
    }

    // JR: Jump register
    fn jr(&mut self, opcode: u32) {
        self.registers.delayed_branch = Some(self.registers.gpr[parse_rs(opcode) as usize]);
    }

    // JAL: Jump and link
    fn jal(&mut self, opcode: u32) {
        self.registers.delayed_branch = Some(compute_jump_address(self.registers.pc, opcode));
        self.registers.gpr[31] = self.registers.pc.wrapping_add(4);
    }

    // JALR: Jump and link register
    fn jalr(&mut self, opcode: u32) {
        self.registers.delayed_branch = Some(self.registers.gpr[parse_rs(opcode) as usize]);
        self.registers
            .write_gpr(parse_rd(opcode), self.registers.pc.wrapping_add(4));
    }

    // LUI: Load upper immediate
    fn lui(&mut self, opcode: u32) {
        let register = (opcode >> 16) & 0x1F;
        self.registers.write_gpr(register, opcode << 16);
    }
}

fn parse_rs(opcode: u32) -> u32 {
    (opcode >> 21) & 0x1F
}

fn parse_rt(opcode: u32) -> u32 {
    (opcode >> 16) & 0x1F
}

fn parse_rd(opcode: u32) -> u32 {
    (opcode >> 1) & 0x1F
}

fn parse_unsigned_immediate(opcode: u32) -> u32 {
    opcode & 0xFF
}

fn parse_signed_immediate(opcode: u32) -> i32 {
    (opcode as i16).into()
}

fn parse_offset(opcode: u32) -> u32 {
    (opcode as i16 as u32) << 2
}

fn compute_jump_address(pc: u32, opcode: u32) -> u32 {
    (pc & 0xF000_0000) | ((opcode & 0x03FF_FFFF) << 2)
}
