use crate::cpu::instructions::{parse_rd, parse_rs, parse_rt, parse_sa, parse_signed_immediate};

pub fn instruction_str(opcode: u32) -> String {
    match opcode >> 26 {
        0x00 => match opcode & 0x3F {
            0x00 => format!(
                "SLL R{}, R{}, {}",
                parse_rd(opcode),
                parse_rt(opcode),
                parse_sa(opcode)
            ),
            0x02 => format!(
                "SRL R{}, R{}, {}",
                parse_rd(opcode),
                parse_rt(opcode),
                parse_sa(opcode)
            ),
            0x03 => format!(
                "SRA R{}, R{}, {}",
                parse_rd(opcode),
                parse_rt(opcode),
                parse_sa(opcode)
            ),
            0x04 => format!(
                "SLLV R{}, R{}, R{}",
                parse_rd(opcode),
                parse_rt(opcode),
                parse_rs(opcode)
            ),
            0x06 => format!(
                "SRLV R{}, R{}, R{}",
                parse_rd(opcode),
                parse_rt(opcode),
                parse_rs(opcode)
            ),
            0x07 => format!(
                "SRAV R{}, R{}, R{}",
                parse_rd(opcode),
                parse_rt(opcode),
                parse_rs(opcode)
            ),
            0x08 => format!("JR R{}", parse_rs(opcode)),
            0x09 => format!("JALR R{}, R{}", parse_rd(opcode), parse_rs(opcode)),
            0x0C => "SYSCALL".into(),
            0x0D => "BREAK".into(),
            0x10 => format!("MFHI R{}", parse_rd(opcode)),
            0x11 => format!("MTHI R{}", parse_rs(opcode)),
            0x12 => format!("MFLO R{}", parse_rd(opcode)),
            0x13 => format!("MTLO R{}", parse_rs(opcode)),
            0x18 => format!("MULT R{}, R{}", parse_rs(opcode), parse_rt(opcode)),
            0x19 => format!("MULTU R{}, R{}", parse_rs(opcode), parse_rt(opcode)),
            0x1A => format!("DIV R{}, R{}", parse_rs(opcode), parse_rt(opcode)),
            0x1B => format!("DIVU R{}, R{}", parse_rs(opcode), parse_rt(opcode)),
            0x20 => format!(
                "ADD R{}, R{}, R{}",
                parse_rd(opcode),
                parse_rs(opcode),
                parse_rt(opcode)
            ),
            0x21 => format!(
                "ADDU R{}, R{}, R{}",
                parse_rd(opcode),
                parse_rs(opcode),
                parse_rt(opcode)
            ),
            0x22 => format!(
                "SUB R{}, R{}, R{}",
                parse_rd(opcode),
                parse_rs(opcode),
                parse_rt(opcode)
            ),
            0x23 => format!(
                "SUBU R{}, R{}, R{}",
                parse_rd(opcode),
                parse_rs(opcode),
                parse_rt(opcode)
            ),
            0x24 => format!(
                "AND R{}, R{}, R{}",
                parse_rd(opcode),
                parse_rs(opcode),
                parse_rt(opcode)
            ),
            0x25 => format!(
                "OR R{}, R{}, R{}",
                parse_rd(opcode),
                parse_rs(opcode),
                parse_rt(opcode)
            ),
            0x26 => format!(
                "XOR R{}, R{}, R{}",
                parse_rd(opcode),
                parse_rs(opcode),
                parse_rt(opcode)
            ),
            0x27 => format!(
                "NOR R{}, R{}, R{}",
                parse_rd(opcode),
                parse_rs(opcode),
                parse_rt(opcode)
            ),
            0x2A => format!(
                "SLT R{}, R{}, R{}",
                parse_rd(opcode),
                parse_rs(opcode),
                parse_rt(opcode)
            ),
            0x2B => format!(
                "SLTU R{}, R{}, R{}",
                parse_rd(opcode),
                parse_rs(opcode),
                parse_rt(opcode)
            ),
            _ => panic!("invalid opcode {opcode:08X}"),
        },
        0x01 => match (opcode >> 16) & 0x1F {
            0x00 => format!(
                "BLTZ R{}, {}",
                parse_rs(opcode),
                parse_signed_immediate(opcode) << 2
            ),
            0x01 => format!(
                "BGEZ R{}, {}",
                parse_rs(opcode),
                parse_signed_immediate(opcode) << 2
            ),
            0x10 => format!(
                "BLTZAL R{}, {}",
                parse_rs(opcode),
                parse_signed_immediate(opcode) << 2
            ),
            0x11 => format!(
                "BGEZAL R{}, {}",
                parse_rs(opcode),
                parse_signed_immediate(opcode) << 2
            ),
            _ => panic!("invalid opcode {opcode:08X}"),
        },
        0x02 => format!("J ${:07X}", (opcode & 0x3FFFFFF) << 2),
        0x03 => format!("JAL ${:07X}", (opcode & 0x3FFFFFF) << 2),
        0x04 => format!(
            "BEQ R{}, R{}, {}",
            parse_rs(opcode),
            parse_rt(opcode),
            parse_signed_immediate(opcode) << 2
        ),
        0x05 => format!(
            "BNE R{}, R{}, {}",
            parse_rs(opcode),
            parse_rt(opcode),
            parse_signed_immediate(opcode) << 2
        ),
        0x06 => format!(
            "BLEZ, R{}, {}",
            parse_rs(opcode),
            parse_signed_immediate(opcode) << 2
        ),
        0x07 => format!(
            "BGTZ R{}, {}",
            parse_rs(opcode),
            parse_signed_immediate(opcode) << 2
        ),
        0x08 => format!(
            "ADDI R{}, R{}, ${:04X}",
            parse_rt(opcode),
            parse_rs(opcode),
            opcode & 0xFFFF
        ),
        0x09 => format!(
            "ADDIU R{}, R{}, ${:04X}",
            parse_rt(opcode),
            parse_rs(opcode),
            opcode & 0xFFFF
        ),
        0x0A => format!(
            "SLTI R{}, R{}, ${:04X}",
            parse_rt(opcode),
            parse_rs(opcode),
            opcode & 0xFFFF
        ),
        0x0B => format!(
            "SLTIU R{}, R{}, ${:04X}",
            parse_rt(opcode),
            parse_rs(opcode),
            opcode & 0xFFFF
        ),
        0x0C => format!(
            "ANDI R{}, R{}, ${:04X}",
            parse_rt(opcode),
            parse_rs(opcode),
            opcode & 0xFFFF
        ),
        0x0D => format!(
            "ORI R{}, R{}, ${:04X}",
            parse_rt(opcode),
            parse_rs(opcode),
            opcode & 0xFFFF
        ),
        0x0E => format!(
            "XORI R{}, R{} ${:04X}",
            parse_rt(opcode),
            parse_rs(opcode),
            opcode & 0xFFFF
        ),
        0x0F => format!("LUI R{}, ${:04X}", parse_rt(opcode), opcode & 0xFFFF),
        0x10..=0x13 => {
            let cp = (opcode >> 26) & 0x3;
            match (opcode >> 21) & 0x1F {
                0x00 => format!("MFC{cp} R{}, R{}", parse_rt(opcode), parse_rd(opcode)),
                0x02 => format!("CFC{cp} R{}, R{}", parse_rt(opcode), parse_rd(opcode)),
                0x04 => format!("MTC{cp} R{}, R{}", parse_rt(opcode), parse_rd(opcode)),
                0x06 => format!("CTC{cp} R{}, R{}", parse_rt(opcode), parse_rd(opcode)),
                0x10..=0x1F => format!("COP{cp} {:06X}", opcode & 0xFFFFFF),
                _ => panic!("invalid opcode {opcode:08X}"),
            }
        }
        0x20 => format!(
            "LB R{}, {}(R{})",
            parse_rt(opcode),
            parse_signed_immediate(opcode),
            parse_rs(opcode)
        ),
        0x21 => format!(
            "LH R{}, {}(R{})",
            parse_rt(opcode),
            parse_signed_immediate(opcode),
            parse_rs(opcode)
        ),
        0x22 => format!(
            "LWL R{}, {}(R{})",
            parse_rt(opcode),
            parse_signed_immediate(opcode),
            parse_rs(opcode)
        ),
        0x23 => format!(
            "LW R{}, {}(R{})",
            parse_rt(opcode),
            parse_signed_immediate(opcode),
            parse_rs(opcode)
        ),
        0x24 => format!(
            "LBU R{}, {}(R{})",
            parse_rt(opcode),
            parse_signed_immediate(opcode),
            parse_rs(opcode)
        ),
        0x25 => format!(
            "LHU R{}, {}(R{})",
            parse_rt(opcode),
            parse_signed_immediate(opcode),
            parse_rs(opcode)
        ),
        0x26 => format!(
            "LWR R{}, {}(R{})",
            parse_rt(opcode),
            parse_signed_immediate(opcode),
            parse_rs(opcode)
        ),
        0x28 => format!(
            "SB R{}, {}(R{})",
            parse_rt(opcode),
            parse_signed_immediate(opcode),
            parse_rs(opcode)
        ),
        0x29 => format!(
            "SH R{}, {}(R{})",
            parse_rt(opcode),
            parse_signed_immediate(opcode),
            parse_rs(opcode)
        ),
        0x2A => format!(
            "SWL R{}, {}(R{})",
            parse_rt(opcode),
            parse_signed_immediate(opcode),
            parse_rs(opcode)
        ),
        0x2B => format!(
            "SW R{}, {}(R{})",
            parse_rt(opcode),
            parse_signed_immediate(opcode),
            parse_rs(opcode)
        ),
        0x2E => format!(
            "SWR R{}, {}(R{})",
            parse_rt(opcode),
            parse_signed_immediate(opcode),
            parse_rs(opcode)
        ),
        0x30..=0x33 => {
            let cp = (opcode >> 26) & 0x3;
            format!(
                "LWC{cp} R{}, {}(R{})",
                parse_rt(opcode),
                parse_signed_immediate(opcode),
                parse_rs(opcode)
            )
        }
        0x38..=0x3B => {
            let cp = (opcode >> 26) & 0x3;
            format!(
                "SWC{cp} R{}, {}(R{})",
                parse_rt(opcode),
                parse_signed_immediate(opcode),
                parse_rs(opcode)
            )
        }
        _ => panic!("invalid opcode {opcode:08X}"),
    }
}
