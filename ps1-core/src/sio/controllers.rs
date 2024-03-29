use crate::input::Ps1JoypadState;
use crate::sio::rxfifo::RxFifo;
use bincode::{Decode, Encode};

#[allow(clippy::enum_variant_names)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
enum DigitalState {
    SendingIdLow,
    SendingIdHigh,
    SendingInputsLow,
    SendingInputsHigh,
}

#[derive(Debug, Clone, Encode, Decode)]
pub struct DigitalController {
    state: DigitalState,
    joypad: Ps1JoypadState,
}

impl DigitalController {
    pub fn initial(joypad: Ps1JoypadState) -> Self {
        Self { state: DigitalState::SendingIdLow, joypad }
    }

    pub fn process(self, tx: u8, rx: &mut RxFifo) -> Option<Self> {
        match self.state {
            DigitalState::SendingIdLow => {
                // High nibble $4 = digital controller
                // Low nibble $1 = 1 halfword for buttons/switches
                rx.push(0x41);

                // $42 = read command
                // Abort communication on other commands; later PS1 games depend on this to
                // correctly detect that this is not a DualShock controller
                (tx == 0x42).then_some(self.with_state(DigitalState::SendingIdHigh))
            }
            DigitalState::SendingIdHigh => {
                // Controllers always respond with $5A to indicate ready to send input
                rx.push(0x5A);

                Some(self.with_state(DigitalState::SendingInputsLow))
            }
            DigitalState::SendingInputsLow => {
                rx.push(!u16::from(self.joypad) as u8);

                Some(self.with_state(DigitalState::SendingInputsHigh))
            }
            DigitalState::SendingInputsHigh => {
                rx.push((!u16::from(self.joypad) >> 8) as u8);

                None
            }
        }
    }

    fn with_state(self, state: DigitalState) -> Self {
        Self { state, joypad: self.joypad }
    }
}
