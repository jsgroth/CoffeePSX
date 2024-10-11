//! SIO0 controller code

use crate::input::{AnalogJoypadState, AnalogMode, DigitalJoypadState};
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
    joypad: DigitalJoypadState,
}

impl DigitalController {
    pub fn initial(joypad: DigitalJoypadState) -> Self {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Encode, Decode)]
pub enum DualShockMode {
    #[default]
    Normal,
    Config,
}

#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode)]
pub struct DualShockControllerState {
    pub mode: DualShockMode,
    pub analog_mode: AnalogMode,
    pub rumble_config: [u8; 6],
    pub analog_mode_locked: bool,
    pub config_mode_entered: bool,
    pub analog_mode_changed: bool,
}

impl Default for DualShockControllerState {
    fn default() -> Self {
        Self {
            mode: DualShockMode::default(),
            analog_mode: AnalogMode::default(),
            rumble_config: [0xFF; 6],
            analog_mode_locked: false,
            config_mode_entered: false,
            analog_mode_changed: false,
        }
    }
}

impl DualShockControllerState {
    pub fn toggle_analog_mode(&mut self) {
        if !self.analog_mode_locked {
            self.analog_mode = self.analog_mode.toggle();
            self.analog_mode_changed = true;
            self.rumble_config.fill(0xFF);
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
pub enum DualShockConfigCommand {
    // 0x43
    ChangeMode,
    // 0x44
    SetLedState,
    // 0x45
    GetLedState,
    // 0x46
    VariableResponseA,
    // 0x47
    ConstantValues,
    // 0x4C
    VariableResponseB,
    // 0x4D
    ConfigureRumble,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
pub enum DualShockSioState {
    SendingIdLow,
    NormalSendingIdHigh { entering_config: bool },
    ConfigSendingIdHigh(DualShockConfigCommand),
    SendingDigitalInputsLow { entering_config: bool },
    SendingDigitalInputsHigh { send_analog: bool },
    SendingAnalogInputsRightX,
    SendingAnalogInputsRightY,
    SendingAnalogInputsLeftX,
    SendingAnalogInputsLeftY,
    ConfigReceivingModeChange,
    ReceivingLedState,
    ReceivingLedLockState,
    SendingLedState { remaining: u8 },
    ConfigConstantValues { remaining: u8 },
    VariableResponseAReceivingInput,
    SendingVariableResponseAZero { remaining: u8 },
    SendingVariableResponseAOne { remaining: u8 },
    VariableResponseBReceivingInput,
    SendingVariableResponseB { remaining: u8, input: u8 },
    ConfiguringRumble { remaining: u8 },
    SendingZeroes { remaining: u8 },
}

#[derive(Debug, Clone, Encode, Decode)]
pub struct DualShock {
    state: DualShockSioState,
    digital: DigitalJoypadState,
    analog: AnalogJoypadState,
}

impl DualShock {
    pub fn initial(digital: DigitalJoypadState, analog: AnalogJoypadState) -> Self {
        Self { state: DualShockSioState::SendingIdLow, digital, analog }
    }

    pub fn process(
        self,
        tx: u8,
        rx: &mut RxFifo,
        state: &mut DualShockControllerState,
    ) -> Option<Self> {
        use DualShockConfigCommand as ConfigCommand;
        use DualShockSioState as SioState;

        log::debug!("Received tx byte {tx:02X}, current state {:?}", self.state);

        match self.state {
            SioState::SendingIdLow => {
                let id_low = match (state.mode, state.analog_mode) {
                    (DualShockMode::Normal, AnalogMode::Digital) => 0x41,
                    (DualShockMode::Normal, AnalogMode::Analog) => 0x73,
                    (DualShockMode::Config, _) => 0xF3,
                };
                rx.push(id_low);

                match (state.mode, tx) {
                    (_, 0x42) => Some(
                        self.with_state(SioState::NormalSendingIdHigh { entering_config: false }),
                    ),
                    (DualShockMode::Normal, 0x43) => Some(
                        self.with_state(SioState::NormalSendingIdHigh { entering_config: true }),
                    ),
                    (DualShockMode::Config, 0x43) => Some(
                        self.with_state(SioState::ConfigSendingIdHigh(ConfigCommand::ChangeMode)),
                    ),
                    (DualShockMode::Config, 0x44) => Some(
                        self.with_state(SioState::ConfigSendingIdHigh(ConfigCommand::SetLedState)),
                    ),
                    (DualShockMode::Config, 0x45) => Some(
                        self.with_state(SioState::ConfigSendingIdHigh(ConfigCommand::GetLedState)),
                    ),
                    (DualShockMode::Config, 0x46) => Some(self.with_state(
                        SioState::ConfigSendingIdHigh(ConfigCommand::VariableResponseA),
                    )),
                    (DualShockMode::Config, 0x47) => {
                        Some(self.with_state(SioState::ConfigSendingIdHigh(
                            ConfigCommand::ConstantValues,
                        )))
                    }
                    (DualShockMode::Config, 0x4C) => Some(self.with_state(
                        SioState::ConfigSendingIdHigh(ConfigCommand::VariableResponseB),
                    )),
                    (DualShockMode::Config, 0x4D) => {
                        Some(self.with_state(SioState::ConfigSendingIdHigh(
                            ConfigCommand::ConfigureRumble,
                        )))
                    }
                    _ => None,
                }
            }
            SioState::NormalSendingIdHigh { entering_config } => {
                // DualShock sends 0x00 after analog mode is changed to signal to games that they
                // may need to reconfigure the controller
                rx.push(if state.analog_mode_changed && state.config_mode_entered {
                    0x00
                } else {
                    0x5A
                });

                Some(self.with_state(SioState::SendingDigitalInputsLow { entering_config }))
            }
            SioState::ConfigSendingIdHigh(command) => {
                // ID high byte is always 0x5A
                rx.push(0x5A);

                Some(match command {
                    ConfigCommand::ChangeMode => {
                        self.with_state(SioState::ConfigReceivingModeChange)
                    }
                    ConfigCommand::SetLedState => self.with_state(SioState::ReceivingLedState),
                    ConfigCommand::GetLedState => {
                        self.with_state(SioState::SendingLedState { remaining: 6 })
                    }
                    ConfigCommand::VariableResponseA => {
                        self.with_state(SioState::VariableResponseAReceivingInput)
                    }
                    ConfigCommand::ConstantValues => {
                        self.with_state(SioState::ConfigConstantValues { remaining: 6 })
                    }
                    ConfigCommand::VariableResponseB => {
                        self.with_state(SioState::VariableResponseBReceivingInput)
                    }
                    ConfigCommand::ConfigureRumble => {
                        self.with_state(SioState::ConfiguringRumble { remaining: 6 })
                    }
                })
            }
            SioState::SendingDigitalInputsLow { entering_config } => {
                let send_analog =
                    state.mode == DualShockMode::Config || state.analog_mode == AnalogMode::Analog;

                let mut digital_low = u16::from(self.digital) as u8;
                if send_analog {
                    // L3 and R3 are only sent in analog mode (or if entering config mode)
                    digital_low |=
                        (u8::from(self.analog.l3) << 1) | (u8::from(self.analog.r3) << 2);
                }
                rx.push(!digital_low);

                if entering_config && tx == 0x01 {
                    state.mode = DualShockMode::Config;
                    state.analog_mode_changed = false;
                    state.config_mode_entered = true;
                }

                Some(self.with_state(SioState::SendingDigitalInputsHigh { send_analog }))
            }
            SioState::SendingDigitalInputsHigh { send_analog } => {
                rx.push((!u16::from(self.digital) >> 8) as u8);

                send_analog.then_some(self.with_state(SioState::SendingAnalogInputsRightX))
            }
            SioState::SendingAnalogInputsRightX => {
                rx.push(self.analog.right_x);
                Some(self.with_state(SioState::SendingAnalogInputsRightY))
            }
            SioState::SendingAnalogInputsRightY => {
                rx.push(self.analog.right_y);
                Some(self.with_state(SioState::SendingAnalogInputsLeftX))
            }
            SioState::SendingAnalogInputsLeftX => {
                rx.push(self.analog.left_x);
                Some(self.with_state(SioState::SendingAnalogInputsLeftY))
            }
            SioState::SendingAnalogInputsLeftY => {
                rx.push(self.analog.left_y);
                None
            }
            SioState::ConfigReceivingModeChange => {
                rx.push(0x00);

                if tx == 0x00 {
                    state.mode = DualShockMode::Normal;
                }

                Some(self.with_state(SioState::SendingZeroes { remaining: 5 }))
            }
            SioState::ReceivingLedState => {
                rx.push(0x00);

                match tx {
                    0x00 => {
                        state.analog_mode = AnalogMode::Digital;
                        log::debug!("Analog mode forcibly disabled");
                    }
                    0x01 => {
                        state.analog_mode = AnalogMode::Analog;
                        log::debug!("Analog mode forcibly enabled");
                    }
                    _ => {}
                }

                Some(self.with_state(SioState::ReceivingLedLockState))
            }
            SioState::ReceivingLedLockState => {
                rx.push(0x00);

                state.analog_mode_locked = tx & 0x03 == 0x03;
                log::debug!("Analog mode locked: {}", state.analog_mode_locked);

                Some(self.with_state(SioState::SendingZeroes { remaining: 4 }))
            }
            SioState::SendingLedState { remaining } => {
                rx.push(match remaining {
                    6 | 2 => 0x01,
                    5 | 3 => 0x02,
                    4 => match state.analog_mode {
                        AnalogMode::Digital => 0x00,
                        AnalogMode::Analog => 0x01,
                    },
                    1 => 0x00,
                    _ => panic!("Invalid DualShock state: {:?}", self.state),
                });

                (remaining > 1).then_some(
                    self.with_state(SioState::SendingLedState { remaining: remaining - 1 }),
                )
            }
            SioState::ConfigConstantValues { remaining } => {
                rx.push(match remaining {
                    4 => 0x02,
                    2 => 0x01,
                    _ => 0x00,
                });

                (remaining > 1).then_some(
                    self.with_state(SioState::ConfigConstantValues { remaining: remaining - 1 }),
                )
            }
            SioState::VariableResponseAReceivingInput => {
                rx.push(0x00);

                Some(match tx {
                    0x00 => {
                        self.with_state(SioState::SendingVariableResponseAZero { remaining: 5 })
                    }
                    0x01 => self.with_state(SioState::SendingVariableResponseAOne { remaining: 5 }),
                    _ => self.with_state(SioState::SendingZeroes { remaining: 5 }),
                })
            }
            SioState::SendingVariableResponseAZero { remaining } => {
                rx.push(match remaining {
                    5 | 2 => 0x00,
                    4 => 0x01,
                    3 => 0x02,
                    1 => 0x0A,
                    _ => panic!("Invalid DualShock state: {:?}", self.state),
                });

                (remaining > 1).then_some(self.with_state(SioState::SendingVariableResponseAZero {
                    remaining: remaining - 1,
                }))
            }
            SioState::SendingVariableResponseAOne { remaining } => {
                rx.push(match remaining {
                    5 => 0x00,
                    2..=4 => 0x01,
                    1 => 0x14,
                    _ => panic!("Invalid DualShock state: {:?}", self.state),
                });

                (remaining > 1).then_some(
                    self.with_state(SioState::SendingVariableResponseAOne {
                        remaining: remaining - 1,
                    }),
                )
            }
            SioState::VariableResponseBReceivingInput => {
                rx.push(0x00);
                Some(
                    self.with_state(SioState::SendingVariableResponseB { remaining: 5, input: tx }),
                )
            }
            SioState::SendingVariableResponseB { remaining, input } => {
                if remaining == 3 {
                    rx.push(match input {
                        0x00 => 0x04,
                        0x01 => 0x07,
                        _ => 0x00,
                    });
                } else {
                    rx.push(0x00);
                }

                (remaining > 1).then_some(self.with_state(SioState::SendingVariableResponseB {
                    remaining: remaining - 1,
                    input,
                }))
            }
            SioState::SendingZeroes { remaining } => {
                rx.push(0x00);

                (remaining > 1).then_some(
                    self.with_state(SioState::SendingZeroes { remaining: remaining - 1 }),
                )
            }
            SioState::ConfiguringRumble { remaining } => {
                // Rumble configure command returns the previous values as it accepts new ones
                let idx = (6 - remaining) as usize;
                rx.push(state.rumble_config[idx]);
                state.rumble_config[idx] = tx;

                (remaining > 1).then_some(
                    self.with_state(SioState::ConfiguringRumble { remaining: remaining - 1 }),
                )
            }
        }
    }

    fn with_state(mut self, state: DualShockSioState) -> Self {
        self.state = state;
        self
    }
}
