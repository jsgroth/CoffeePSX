mod disassemble;

use crate::cpu::bus::{BusInterface, OpSize};
use crate::cpu::{Exception, R3000};
use crate::num::U32Ext;

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
    pub(super) fn execute_opcode<B: BusInterface>(
        &mut self,
        opcode: u32,
        pc: u32,
        bus: &mut B,
    ) -> Result<(), Exception> {
        log::trace!(
            "opcode {opcode:08X} at PC {pc:08X}: {}",
            disassemble::instruction_str(opcode)
        );

        // First 6 bits of opcode identify operation
        match opcode >> 26 {
            // If highest 6 bits are all 0, the lowest 6 bits are used to specify the operation
            0x00 => match opcode & 0x3F {
                0x00 => self.sll(opcode),
                0x02 => self.srl(opcode),
                0x03 => self.sra(opcode),
                0x04 => self.sllv(opcode),
                0x06 => self.srlv(opcode),
                0x07 => self.srav(opcode),
                0x0C => return Err(Exception::Syscall),
                0x0D => todo!("BREAK opcode"),
                0x08 => self.jr(opcode),
                0x09 => self.jalr(opcode),
                0x10 => self.mfhi(opcode),
                0x11 => self.mthi(opcode),
                0x12 => self.mflo(opcode),
                0x13 => self.mtlo(opcode),
                0x18 => self.mult(opcode),
                0x19 => self.multu(opcode),
                0x1A => self.div(opcode),
                0x1B => self.divu(opcode),
                0x20 => self.add(opcode),
                0x21 => self.addu(opcode),
                0x22 => self.sub(opcode),
                0x23 => self.subu(opcode),
                0x24 => self.and(opcode),
                0x25 => self.or(opcode),
                0x26 => self.xor(opcode),
                0x27 => self.nor(opcode),
                0x2A => self.slt(opcode),
                0x2B => self.sltu(opcode),
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
            0x0A => self.slti(opcode),
            0x0B => self.sltiu(opcode),
            0x0C => self.andi(opcode),
            0x0D => self.ori(opcode),
            0x0E => self.xori(opcode),
            0x0F => self.lui(opcode),
            // If highest 6 bits are $10-$13, this is a coprocessor opcode and bits 21-25 specify
            // the operation
            0x10..=0x13 => match (opcode >> 21) & 0x1F {
                0x00 => self.mfcz(opcode),
                0x02 => todo!("CFCz opcode {opcode:08X}"),
                0x04 => self.mtcz(opcode),
                0x06 => todo!("CTCz opcode {opcode:08X}"),
                0x10..=0x1F => self.copz(opcode),
                _ => todo!("coprocessor opcode {opcode:08X}"),
            },
            0x20 => self.lb(opcode, bus),
            0x21 => self.lh(opcode, bus),
            0x22 => self.lwl(opcode, bus),
            0x23 => self.lw(opcode, bus),
            0x24 => self.lbu(opcode, bus),
            0x25 => self.lhu(opcode, bus),
            0x26 => self.lwr(opcode, bus),
            0x28 => self.sb(opcode, bus),
            0x29 => self.sh(opcode, bus),
            0x2A => self.swl(opcode, bus),
            0x2B => self.sw(opcode, bus),
            0x2E => self.swr(opcode, bus),
            0x30..=0x33 => todo!("LWCz opcode {opcode:08X}"),
            0x38..=0x3B => todo!("SWCz opcode {opcode:08X}"),
            _ => todo!("opcode {opcode:08X}"),
        }

        Ok(())
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

    // LB: Load byte
    fn lb<B: BusInterface>(&mut self, opcode: u32, bus: &mut B) {
        let base_addr = self.registers.gpr[parse_rs(opcode) as usize];
        let address = base_addr.wrapping_add(parse_signed_immediate(opcode) as u32);
        let byte = self.bus_read(bus, address, OpSize::Byte);
        self.registers
            .write_gpr(parse_rt(opcode), byte as i8 as u32);
    }

    // LBU: Load byte unsigned
    fn lbu<B: BusInterface>(&mut self, opcode: u32, bus: &mut B) {
        let base_addr = self.registers.gpr[parse_rs(opcode) as usize];
        let address = base_addr.wrapping_add(parse_signed_immediate(opcode) as u32);
        let byte = self.bus_read(bus, address, OpSize::Byte);
        self.registers.write_gpr(parse_rt(opcode), byte);
    }

    // LH: Load halfword
    fn lh<B: BusInterface>(&mut self, opcode: u32, bus: &mut B) {
        let base_addr = self.registers.gpr[parse_rs(opcode) as usize];
        let address = base_addr.wrapping_add(parse_signed_immediate(opcode) as u32);
        let halfword = self.bus_read(bus, address, OpSize::HalfWord);
        self.registers
            .write_gpr(parse_rt(opcode), halfword as i16 as u32);
    }

    // LHU: Load halfword unsigned
    fn lhu<B: BusInterface>(&mut self, opcode: u32, bus: &mut B) {
        let base_addr = self.registers.gpr[parse_rs(opcode) as usize];
        let address = base_addr.wrapping_add(parse_signed_immediate(opcode) as u32);
        let halfword = self.bus_read(bus, address, OpSize::HalfWord);
        self.registers.write_gpr(parse_rt(opcode), halfword);
    }

    // LUI: Load upper immediate
    fn lui(&mut self, opcode: u32) {
        let register = (opcode >> 16) & 0x1F;
        self.registers.write_gpr(register, opcode << 16);
    }

    // LW: Load word
    fn lw<B: BusInterface>(&mut self, opcode: u32, bus: &mut B) {
        let base_addr = self.registers.gpr[parse_rs(opcode) as usize];
        let address = base_addr.wrapping_add(parse_signed_immediate(opcode) as u32);
        let word = self.bus_read(bus, address, OpSize::Word);
        self.registers.write_gpr(parse_rt(opcode), word);
    }

    // LWL: Load word left
    fn lwl<B: BusInterface>(&mut self, opcode: u32, bus: &mut B) {
        let base_addr = self.registers.gpr[parse_rs(opcode) as usize];
        let address = base_addr.wrapping_add(parse_signed_immediate(opcode) as u32);

        let rt = parse_rt(opcode);
        let existing_value = self.registers.gpr[rt as usize];

        let memory_word = self.bus_read(bus, address & !3, OpSize::Word).swap_bytes();
        let shift = 8 * ((address & 3) ^ 3);
        let mask: u32 = 0xFFFF_FFFF << shift;

        let new_value = (existing_value & !mask) | (memory_word << shift);
        self.registers.write_gpr(rt, new_value);
    }

    // LWR: Load word right
    fn lwr<B: BusInterface>(&mut self, opcode: u32, bus: &mut B) {
        let base_addr = self.registers.gpr[parse_rs(opcode) as usize];
        let address = base_addr.wrapping_add(parse_signed_immediate(opcode) as u32);

        let rt = parse_rt(opcode);
        let existing_value = self.registers.gpr[rt as usize];

        let memory_word = self.bus_read(bus, address & !3, OpSize::Word).swap_bytes();
        let shift = 8 * (address & 3);
        let mask = 0xFFFF_FFFF >> shift;

        let new_value = (existing_value & !mask) | (memory_word >> shift);
        self.registers.write_gpr(rt, new_value);
    }

    // MFHI: Move from HI
    fn mfhi(&mut self, opcode: u32) {
        self.registers
            .write_gpr(parse_rd(opcode), self.registers.hi);
    }

    // MFLO: Move from LO
    fn mflo(&mut self, opcode: u32) {
        self.registers
            .write_gpr(parse_rd(opcode), self.registers.lo);
    }

    // MTHI: Move to HI
    fn mthi(&mut self, opcode: u32) {
        self.registers.hi = self.registers.gpr[parse_rs(opcode) as usize];
    }

    // MTLO: Move to LO
    fn mtlo(&mut self, opcode: u32) {
        self.registers.lo = self.registers.gpr[parse_rs(opcode) as usize];
    }

    // MULT: Multiply word
    fn mult(&mut self, opcode: u32) {
        // TODO timing?
        let operand_a: i64 = self.registers.gpr[parse_rs(opcode) as usize].into();
        let operand_b: i64 = self.registers.gpr[parse_rt(opcode) as usize].into();
        let product = operand_a * operand_b;

        self.registers.lo = product as u32;
        self.registers.hi = (product >> 32) as u32;
    }

    // MULTU: Multiply unsigned word
    fn multu(&mut self, opcode: u32) {
        // TODO timing?
        let operand_a: u64 = self.registers.gpr[parse_rs(opcode) as usize].into();
        let operand_b: u64 = self.registers.gpr[parse_rs(opcode) as usize].into();
        let product = operand_a * operand_b;

        self.registers.lo = product as u32;
        self.registers.hi = (product >> 32) as u32;
    }

    // NOR: Nor
    fn nor(&mut self, opcode: u32) {
        let operand_a = self.registers.gpr[parse_rs(opcode) as usize];
        let operand_b = self.registers.gpr[parse_rt(opcode) as usize];
        self.registers
            .write_gpr(parse_rd(opcode), !(operand_a | operand_b));
    }

    // OR: Or
    fn or(&mut self, opcode: u32) {
        let operand_a = self.registers.gpr[parse_rs(opcode) as usize];
        let operand_b = self.registers.gpr[parse_rt(opcode) as usize];
        self.registers
            .write_gpr(parse_rd(opcode), operand_a | operand_b);
    }

    // ORI: Or immediate
    fn ori(&mut self, opcode: u32) {
        let operand_a = self.registers.gpr[parse_rs(opcode) as usize];
        let operand_b = parse_unsigned_immediate(opcode);
        self.registers
            .write_gpr(parse_rt(opcode), operand_a | operand_b);
    }

    // SB: Store byte
    fn sb<B: BusInterface>(&mut self, opcode: u32, bus: &mut B) {
        let base_addr = self.registers.gpr[parse_rs(opcode) as usize];
        let address = base_addr.wrapping_add(parse_signed_immediate(opcode) as u32);
        let byte = self.registers.gpr[parse_rt(opcode) as usize];
        self.bus_write(bus, address, byte, OpSize::Byte);
    }

    // SH: Store halfword
    fn sh<B: BusInterface>(&mut self, opcode: u32, bus: &mut B) {
        let base_addr = self.registers.gpr[parse_rs(opcode) as usize];
        let address = base_addr.wrapping_add(parse_signed_immediate(opcode) as u32);
        let halfword = self.registers.gpr[parse_rt(opcode) as usize];
        self.bus_write(bus, address, halfword, OpSize::HalfWord);
    }

    // SW: Store word
    fn sw<B: BusInterface>(&mut self, opcode: u32, bus: &mut B) {
        let base_addr = self.registers.gpr[parse_rs(opcode) as usize];
        let address = base_addr.wrapping_add(parse_signed_immediate(opcode) as u32);
        let word = self.registers.gpr[parse_rt(opcode) as usize];
        self.bus_write(bus, address, word, OpSize::Word);
    }

    // SWL: Store word left
    fn swl<B: BusInterface>(&mut self, opcode: u32, bus: &mut B) {
        let base_addr = self.registers.gpr[parse_rs(opcode) as usize];
        let address = base_addr.wrapping_add(parse_signed_immediate(opcode) as u32);

        let existing_word = self.bus_read(bus, address & !3, OpSize::Word).swap_bytes();
        let register_word = self.registers.gpr[parse_rt(opcode) as usize];

        let shift = 8 * ((address & 3) ^ 3);
        let mask: u32 = 0xFFFF_FFFF >> shift;

        let new_value = (existing_word & !mask) | (register_word >> shift);
        self.bus_write(bus, address & !3, new_value.swap_bytes(), OpSize::Word);
    }

    // SWR: Store word right
    fn swr<B: BusInterface>(&mut self, opcode: u32, bus: &mut B) {
        let base_addr = self.registers.gpr[parse_rs(opcode) as usize];
        let address = base_addr.wrapping_add(parse_signed_immediate(opcode) as u32);

        let existing_word = self.bus_read(bus, address & !3, OpSize::Word).swap_bytes();
        let register_word = self.registers.gpr[parse_rt(opcode) as usize];

        let shift = 8 * (address & 3);
        let mask: u32 = 0xFFFF_FFFF << shift;

        let new_value = (existing_word & !mask) | (register_word << shift);
        self.bus_write(bus, address & !3, new_value.swap_bytes(), OpSize::Word);
    }

    // SLL: Shift word left logical
    fn sll(&mut self, opcode: u32) {
        let value = self.registers.gpr[parse_rt(opcode) as usize] << parse_sa(opcode);
        self.registers.write_gpr(parse_rd(opcode), value);
    }

    // SLLV: Shift word left logical variable
    fn sllv(&mut self, opcode: u32) {
        let shift_amount = self.registers.gpr[parse_rs(opcode) as usize] & 0x1F;
        let value = self.registers.gpr[parse_rt(opcode) as usize] << shift_amount;
        self.registers.write_gpr(parse_rd(opcode), value);
    }

    // SRA: Shift word right arithmetic
    fn sra(&mut self, opcode: u32) {
        let shift_amount = parse_sa(opcode);
        let rt = self.registers.gpr[parse_rt(opcode) as usize] as i32;
        self.registers
            .write_gpr(parse_rd(opcode), (rt >> shift_amount) as u32);
    }

    // SRAV: Shift word right arithmetic variable
    fn srav(&mut self, opcode: u32) {
        let shift_amount = self.registers.gpr[parse_rs(opcode) as usize] & 0x1F;
        let rt = self.registers.gpr[parse_rt(opcode) as usize] as i32;
        self.registers
            .write_gpr(parse_rd(opcode), (rt >> shift_amount) as u32);
    }

    // SRL: Shift word right logical
    fn srl(&mut self, opcode: u32) {
        let value = self.registers.gpr[parse_rt(opcode) as usize] >> parse_sa(opcode);
        self.registers.write_gpr(parse_rd(opcode), value);
    }

    // SRLV: Shift word right logical variable
    fn srlv(&mut self, opcode: u32) {
        let shift_amount = self.registers.gpr[parse_rs(opcode) as usize] & 0x1F;
        let value = self.registers.gpr[parse_rt(opcode) as usize] >> shift_amount;
        self.registers.write_gpr(parse_rd(opcode), value);
    }

    // SLT: Set on less than
    fn slt(&mut self, opcode: u32) {
        let rs = self.registers.gpr[parse_rs(opcode) as usize] as i32;
        let rt = self.registers.gpr[parse_rt(opcode) as usize] as i32;
        self.registers.write_gpr(parse_rd(opcode), (rs < rt).into());
    }

    // SLTU: Set on less than unsigned
    fn sltu(&mut self, opcode: u32) {
        let rs = self.registers.gpr[parse_rs(opcode) as usize];
        let rt = self.registers.gpr[parse_rt(opcode) as usize];
        self.registers.write_gpr(parse_rd(opcode), (rs < rt).into());
    }

    // SLTI: Set on less than immediate
    fn slti(&mut self, opcode: u32) {
        let rs = self.registers.gpr[parse_rs(opcode) as usize] as i32;
        let immediate = parse_signed_immediate(opcode);
        self.registers
            .write_gpr(parse_rt(opcode), (rs < immediate).into());
    }

    // SLTIU: Set on less than immediate unsigned
    fn sltiu(&mut self, opcode: u32) {
        let rs = self.registers.gpr[parse_rs(opcode) as usize];
        let immediate = parse_signed_immediate(opcode) as u32;
        self.registers
            .write_gpr(parse_rt(opcode), (rs < immediate).into());
    }

    // SUB: Subtract word
    fn sub(&mut self, opcode: u32) {
        let rs = self.registers.gpr[parse_rs(opcode) as usize] as i32;
        let rt = self.registers.gpr[parse_rt(opcode) as usize] as i32;
        let (difference, overflowed) = rs.overflowing_sub(rt);
        if overflowed {
            todo!("integer overflow exception")
        }

        self.registers
            .write_gpr(parse_rd(opcode), difference as u32);
    }

    // SUBU: Subtract unsigned word
    fn subu(&mut self, opcode: u32) {
        let rs = self.registers.gpr[parse_rs(opcode) as usize];
        let rt = self.registers.gpr[parse_rt(opcode) as usize];
        self.registers
            .write_gpr(parse_rd(opcode), rs.wrapping_sub(rt));
    }

    // XOR: Exclusive or
    fn xor(&mut self, opcode: u32) {
        let rs = self.registers.gpr[parse_rs(opcode) as usize];
        let rt = self.registers.gpr[parse_rt(opcode) as usize];
        self.registers.write_gpr(parse_rd(opcode), rs ^ rt);
    }

    // XORI: Exclusive or immediate
    fn xori(&mut self, opcode: u32) {
        let rs = self.registers.gpr[parse_rs(opcode) as usize];
        let immediate = parse_unsigned_immediate(opcode);
        self.registers.write_gpr(parse_rd(opcode), rs ^ immediate);
    }

    // MFCz: Move from coprocessor
    fn mfcz(&mut self, opcode: u32) {
        let register = parse_rd(opcode);
        let value = match parse_coprocessor(opcode) {
            0 => self.cp0.read_register(register),
            cp => todo!("MFC{cp} {register}"),
        };
        self.registers.write_gpr(parse_rt(opcode), value);
    }

    // MTCz: Move to coprocessor
    fn mtcz(&mut self, opcode: u32) {
        let register = parse_rd(opcode);
        let value = self.registers.gpr[parse_rt(opcode) as usize];
        match parse_coprocessor(opcode) {
            0 => self.cp0.write_register(register, value),
            cp => todo!("MTC{cp} {register} {value:08X}"),
        }
    }

    // COPz: Coprocessor operation
    fn copz(&mut self, opcode: u32) {
        let operation = opcode & 0xFFFFFF;
        match parse_coprocessor(opcode) {
            0 => self.cp0.execute_operation(operation),
            cp => todo!("COP{cp} {opcode:08X}"),
        }
    }
}

fn parse_rs(opcode: u32) -> u32 {
    (opcode >> 21) & 0x1F
}

fn parse_rt(opcode: u32) -> u32 {
    (opcode >> 16) & 0x1F
}

fn parse_rd(opcode: u32) -> u32 {
    (opcode >> 11) & 0x1F
}

fn parse_sa(opcode: u32) -> u32 {
    (opcode >> 6) & 0x1F
}

fn parse_unsigned_immediate(opcode: u32) -> u32 {
    opcode & 0xFFFF
}

fn parse_signed_immediate(opcode: u32) -> i32 {
    (opcode as i16).into()
}

fn parse_offset(opcode: u32) -> u32 {
    (opcode as i16 as u32) << 2
}

fn parse_coprocessor(opcode: u32) -> u32 {
    (opcode >> 26) & 0x3
}

fn compute_jump_address(pc: u32, opcode: u32) -> u32 {
    (pc & 0xF000_0000) | ((opcode & 0x03FF_FFFF) << 2)
}