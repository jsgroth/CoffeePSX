//! PS1 Macroblock Decoder (MDEC), a hardware image decompressor

use crate::num::U32Ext;
use bincode::{Decode, Encode};
use std::collections::VecDeque;

#[derive(Debug, Clone, Copy, Encode, Decode)]
enum CommandState {
    Idle,
    ReceivingLuminanceTable { bytes_remaining: u8, color_table_after: bool },
    ReceivingColorTable { bytes_remaining: u8 },
    ReceivingScaleTable { halfwords_remaining: u8 },
}

#[derive(Debug, Clone, Encode, Decode)]
pub struct MacroblockDecoder {
    command_state: CommandState,
    data_in: Vec<u32>,
    data_out: VecDeque<u32>,
    enable_data_in: bool,
    enable_data_out: bool,
    luminance_quant_table: [u8; 64],
    color_quant_table: [u8; 64],
}

impl MacroblockDecoder {
    pub fn new() -> Self {
        Self {
            command_state: CommandState::Idle,
            data_in: Vec::with_capacity(16 * 240),
            data_out: VecDeque::with_capacity(16 * 240),
            enable_data_in: false,
            enable_data_out: false,
            luminance_quant_table: [0; 64],
            color_quant_table: [0; 64],
        }
    }

    // $1F801820 W: MDEC command/parameter register
    pub fn write_command(&mut self, value: u32) {
        log::trace!("MDEC command write: {value:08X}");

        self.command_state = match self.command_state {
            CommandState::Idle => match value >> 29 {
                // MDEC(0) is a no-op and MDEC(4-7) are invalid commands
                0 | 4..=7 => {
                    log::debug!("MDEC no-op command: {value:08X}");
                    CommandState::Idle
                }
                // MDEC(2): Set quant tables
                2 => {
                    log::debug!("MDEC set quant tables command: {value:08X}");
                    CommandState::ReceivingLuminanceTable {
                        bytes_remaining: 64,
                        color_table_after: value.bit(0),
                    }
                }
                // MDEC(3): Set scale table
                // This command is not really implemented because it is assumed that every game will
                // send the same values here
                3 => {
                    log::debug!("MDEC set scale table command: {value:08X}");
                    CommandState::ReceivingScaleTable { halfwords_remaining: 64 }
                }
                _ => todo!("MDEC command {value:08X}"),
            },
            CommandState::ReceivingLuminanceTable { mut bytes_remaining, color_table_after } => {
                self.data_in.push(value);
                bytes_remaining -= 4;
                if bytes_remaining == 0 {
                    self.populate_luminance_table();
                    if color_table_after {
                        CommandState::ReceivingColorTable { bytes_remaining: 64 }
                    } else {
                        CommandState::Idle
                    }
                } else {
                    CommandState::ReceivingLuminanceTable { bytes_remaining, color_table_after }
                }
            }
            CommandState::ReceivingColorTable { mut bytes_remaining } => {
                self.data_in.push(value);
                bytes_remaining -= 4;
                if bytes_remaining == 0 {
                    self.populate_color_table();
                    CommandState::Idle
                } else {
                    CommandState::ReceivingColorTable { bytes_remaining }
                }
            }
            CommandState::ReceivingScaleTable { mut halfwords_remaining } => {
                halfwords_remaining -= 2;
                if halfwords_remaining == 0 {
                    log::debug!("Scale table fully received");
                    CommandState::Idle
                } else {
                    CommandState::ReceivingScaleTable { halfwords_remaining }
                }
            }
        };
    }

    fn populate_luminance_table(&mut self) {
        log::debug!("Populating luminance quant table");

        for (i, &word) in self.data_in.iter().enumerate() {
            self.luminance_quant_table[4 * i..4 * (i + 1)].copy_from_slice(&word.to_le_bytes());
        }
        self.data_in.clear();
    }

    fn populate_color_table(&mut self) {
        log::debug!("Populating color quant table");

        for (i, &word) in self.data_in.iter().enumerate() {
            self.color_quant_table[4 * i..4 * (i + 1)].copy_from_slice(&word.to_le_bytes());
        }
        self.data_in.clear();
    }

    // $1F801824 R: MDEC status register
    pub fn read_status(&self) -> u32 {
        // TODO bit 30: data in FIFO full
        // TODO bit 29: command busy
        // TODO bit 27: data out request
        // TODO bits 26-25: data output depth
        // TODO bit 24: data output signed
        // TODO bit 23: data output bit 15
        // TODO bits 18-16: current block
        // TODO bits 15-0: number of parameter words remaining minus 1
        let value = (u32::from(self.data_out.is_empty()) << 31)
            | (u32::from(self.enable_data_in) << 28)
            | 0x00040000;

        log::debug!("MDEC status read: {value:08X}");

        value
    }

    // $1F801824 W: MDEC control/reset register
    pub fn write_control(&mut self, value: u32) {
        if value.bit(31) {
            // TODO actually do reset
            // aborts all commands and makes status read $80040000
            self.command_state = CommandState::Idle;
            self.data_in.clear();
            self.data_out.clear();
        }

        self.enable_data_in = value.bit(30);
        self.enable_data_out = value.bit(29);

        log::debug!("MDEC control write: {value:08X}");
        log::debug!("  MDEC reset: {}", value.bit(31));
        log::debug!("  Data in request enabled: {}", self.enable_data_in);
        log::debug!("  Data out request enabled: {}", self.enable_data_out);
    }
}
