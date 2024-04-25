use crate::sio::rxfifo::RxFifo;
use bincode::{Decode, Encode};

pub const MEMORY_CARD_LEN: usize = 128 * 1024;

pub type MemoryCardData = [u8; MEMORY_CARD_LEN];

#[derive(Debug, Clone, Encode, Decode)]
pub struct MemoryCard {
    written_since_load: bool,
    data: Box<MemoryCardData>,
    dirty: bool,
}

impl MemoryCard {
    pub fn new(data: Option<Vec<u8>>) -> Self {
        let data = match data {
            Some(data) if data.len() == MEMORY_CARD_LEN => data,
            Some(data) => {
                log::error!(
                    "Expected memory card data of size {MEMORY_CARD_LEN}, was {}; formatting card",
                    data.len()
                );
                new_formatted_memory_card()
            }
            None => new_formatted_memory_card(),
        };

        Self {
            written_since_load: false,
            data: data.into_boxed_slice().try_into().unwrap(),
            dirty: true,
        }
    }

    pub fn get_and_clear_dirty(&mut self) -> bool {
        let dirty = self.dirty;
        self.dirty = false;
        dirty
    }

    pub fn data(&self) -> &MemoryCardData {
        &self.data
    }

    fn flag_byte(&self) -> u8 {
        u8::from(!self.written_since_load) << 3
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
enum Command {
    Read,
    Write,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
enum ChecksumStatus {
    Ok = 0x47,
    Invalid = 0x4E,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
enum MemoryCardState {
    AwaitingCommand,
    SendingId1(Command),
    SendingId2(Command),
    ReceivingAddressHigh(Command),
    ReceivingAddressLow(Command),
    ReadAcking1,
    ReadAcking2,
    ReadConfirmingAddressHigh,
    ReadConfirmingAddressLow,
    SendingData { bytes_remaining: u8 },
    ReadSendingChecksum,
    ReadSendingEnd,
    ReceivingData { bytes_remaining: u8 },
    WriteReceivingChecksum,
    WriteAcking1(ChecksumStatus),
    WriteAcking2(ChecksumStatus),
    WriteSendingEnd(ChecksumStatus),
}

#[derive(Debug, Clone, Encode, Decode)]
pub struct ConnectedMemoryCard {
    state: MemoryCardState,
    sector: u16,
    checksum: u8,
    last_tx: u8,
}

impl ConnectedMemoryCard {
    pub fn initial() -> Self {
        Self { state: MemoryCardState::AwaitingCommand, sector: 0, checksum: 0, last_tx: 0 }
    }

    pub fn process(mut self, tx: u8, rx: &mut RxFifo, card: &mut MemoryCard) -> Option<Self> {
        self.last_tx = tx;

        self.state = match self.state {
            // Shared early states
            MemoryCardState::AwaitingCommand => {
                rx.push(card.flag_byte());

                let command = match tx {
                    0x52 => Command::Read,
                    0x57 => Command::Write,
                    _ => return None,
                };
                MemoryCardState::SendingId1(command)
            }
            MemoryCardState::SendingId1(command) => {
                rx.push(0x5A);
                MemoryCardState::SendingId2(command)
            }
            MemoryCardState::SendingId2(command) => {
                rx.push(0x5D);
                MemoryCardState::ReceivingAddressHigh(command)
            }
            MemoryCardState::ReceivingAddressHigh(command) => {
                rx.push(0x00);
                self.sector |= u16::from(tx) << 8;
                MemoryCardState::ReceivingAddressLow(command)
            }
            MemoryCardState::ReceivingAddressLow(command) => {
                rx.push(self.last_tx);
                self.sector |= u16::from(tx);
                self.sector &= 0x3FF;

                log::debug!(
                    "Received memory card sector number {:04X}, command {command:?}",
                    self.sector
                );

                self.checksum ^= (self.sector >> 8) as u8;
                self.checksum ^= self.sector as u8;

                match command {
                    Command::Read => MemoryCardState::ReadAcking1,
                    Command::Write => MemoryCardState::ReceivingData { bytes_remaining: 128 },
                }
            }
            // Read states
            MemoryCardState::ReadAcking1 => {
                rx.push(0x5C);
                MemoryCardState::ReadAcking2
            }
            MemoryCardState::ReadAcking2 => {
                rx.push(0x5D);
                MemoryCardState::ReadConfirmingAddressHigh
            }
            MemoryCardState::ReadConfirmingAddressHigh => {
                rx.push((self.sector >> 8) as u8);
                MemoryCardState::ReadConfirmingAddressLow
            }
            MemoryCardState::ReadConfirmingAddressLow => {
                rx.push(self.sector as u8);
                MemoryCardState::SendingData { bytes_remaining: 128 }
            }
            MemoryCardState::SendingData { bytes_remaining } => {
                let addr = memory_card_address(self.sector, bytes_remaining);
                let byte = card.data[addr];
                rx.push(byte);
                self.checksum ^= byte;

                if bytes_remaining == 1 {
                    MemoryCardState::ReadSendingChecksum
                } else {
                    MemoryCardState::SendingData { bytes_remaining: bytes_remaining - 1 }
                }
            }
            MemoryCardState::ReadSendingChecksum => {
                rx.push(self.checksum);
                MemoryCardState::ReadSendingEnd
            }
            MemoryCardState::ReadSendingEnd => {
                rx.push(0x47);
                return None;
            }
            // Write states
            MemoryCardState::ReceivingData { bytes_remaining } => {
                rx.push(self.last_tx);

                let addr = memory_card_address(self.sector, bytes_remaining);
                card.data[addr] = tx;
                self.checksum ^= tx;

                if bytes_remaining == 1 {
                    card.written_since_load = true;
                    card.dirty = true;
                    MemoryCardState::WriteReceivingChecksum
                } else {
                    MemoryCardState::ReceivingData { bytes_remaining: bytes_remaining - 1 }
                }
            }
            MemoryCardState::WriteReceivingChecksum => {
                rx.push(self.last_tx);

                let checksum_status =
                    if tx == self.checksum { ChecksumStatus::Ok } else { ChecksumStatus::Invalid };
                MemoryCardState::WriteAcking1(checksum_status)
            }
            MemoryCardState::WriteAcking1(checksum_status) => {
                rx.push(0x5C);
                MemoryCardState::WriteAcking2(checksum_status)
            }
            MemoryCardState::WriteAcking2(checksum_status) => {
                rx.push(0x5D);
                MemoryCardState::WriteSendingEnd(checksum_status)
            }
            MemoryCardState::WriteSendingEnd(checksum_status) => {
                rx.push(checksum_status as u8);
                return None;
            }
        };

        Some(self)
    }
}

fn memory_card_address(sector: u16, bytes_remaining: u8) -> usize {
    ((u32::from(sector) << 7) | u32::from(128 - bytes_remaining)) as usize
}

fn new_formatted_memory_card() -> Vec<u8> {
    let mut data = vec![0; MEMORY_CARD_LEN];

    // Header sector (block 0 sector 0): first two bytes are ASCII "MC", last byte is a checksum
    data[0x00] = b'M';
    data[0x01] = b'C';
    data[0x7F] = 0x0E;

    // Directory sectors (block 0 sectors 1-15): Mark all free
    for sector in 1..16 {
        let sector_addr = sector << 7;

        // $00-$03: Allocation state, $A0 = free and freshly formatted
        data[sector_addr] = 0xA0;

        // $08-$09: Pointer to next block in file, $FFFF for last block in file (or no file)
        data[sector_addr | 0x08] = 0xFF;
        data[sector_addr | 0x09] = 0xFF;

        // $7F: Checksum
        data[sector_addr | 0x7F] = 0xA0;
    }

    // Broken sector list (block 0 sectors 16-35): Mark all sectors good
    for sector in 16..36 {
        // $00-$03: Broken sector number, or $FFFFFFFF for none
        let sector_addr = sector << 7;
        data[sector_addr..sector_addr + 4].fill(0xFF);

        // $08-$09: Unknown, but the BIOS seems to write $FFFF here
        data[sector_addr | 0x08] = 0xFF;
        data[sector_addr | 0x09] = 0xFF;

        // $7F: Checksum, will be $00
    }

    // The rest of the card is zero-filled, so return as-is
    data
}
