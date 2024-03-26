use bincode::{Decode, Encode};
use std::array;

// 4KB = 1024 32-bit opcodes
const OPCODES_LEN: usize = 1024;
const OPCODES_MASK: u32 = (OPCODES_LEN - 1) as u32;

// 4-word cache lines
const TAGS_LEN: usize = OPCODES_LEN / 4;
const TAGS_MASK: u32 = (TAGS_LEN - 1) as u32;

// Lowest 2 bits ignored + 10 bits used for cache index
const TAG_SHIFT: u8 = 12;

#[derive(Debug, Clone, Encode, Decode)]
pub struct InstructionCache {
    opcodes: [u32; OPCODES_LEN],
    tags: [u32; TAGS_LEN],
}

impl InstructionCache {
    pub fn new() -> Self {
        Self { opcodes: array::from_fn(|_| 0), tags: array::from_fn(|_| !0) }
    }

    pub fn check_cache(&self, address: u32) -> Option<u32> {
        let tag = address >> TAG_SHIFT;
        let cached_tag = self.tags[((address >> 4) & TAGS_MASK) as usize];
        if tag != cached_tag {
            return None;
        }

        Some(self.opcodes[((address >> 2) & OPCODES_MASK) as usize])
    }

    pub fn get_opcode_no_tag_check(&self, address: u32) -> u32 {
        self.opcodes[((address >> 2) & OPCODES_MASK) as usize]
    }

    pub fn update_tag(&mut self, address: u32) {
        let tag = address >> TAG_SHIFT;
        self.tags[((address >> 4) & TAGS_MASK) as usize] = tag;
    }

    pub fn write_opcode(&mut self, address: u32, value: u32) {
        self.opcodes[((address >> 2) & OPCODES_MASK) as usize] = value;
    }

    pub fn invalidate_tag(&mut self, address: u32) {
        // $FFFFFFFF will never match due to tags only being 20 bits
        self.tags[((address >> 4) & TAGS_MASK) as usize] = !0;
    }
}
