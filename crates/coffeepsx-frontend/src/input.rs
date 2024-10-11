use crate::UserEvent;
use crate::config::InputConfig;
use crate::config::input::{AxisDirection, SdlGamepadInput, SingleInput};
use crate::emuthread::{Ps1AnalogInput, Ps1Button};
use sdl2::controller::Axis as SdlAxis;
use sdl2::controller::Button as SdlButton;
use std::collections::HashMap;
use winit::event_loop::EventLoopProxy;
use winit::keyboard::KeyCode;

struct DigitalValue {
    button: Ps1Button,
    axis_deadzone: i16,
    trigger_threshold: i16,
}

struct AnalogValue {
    input: Ps1AnalogInput,
    direction: AxisDirection,
    axis_deadzone: i16,
}

pub struct InputMapper {
    digital_inputs: HashMap<SingleInput, Vec<DigitalValue>>,
    analog_inputs: HashMap<SingleInput, Vec<AnalogValue>>,
}

impl InputMapper {
    #[must_use]
    pub fn new(input_config: &InputConfig) -> Self {
        let mut digital_inputs: HashMap<SingleInput, Vec<DigitalValue>> = HashMap::new();
        let mut analog_inputs: HashMap<SingleInput, Vec<AnalogValue>> = HashMap::new();

        // TODO P2 inputs
        for set in [&input_config.p1_set_1, &input_config.p1_set_2] {
            for (button, field) in [
                (Ps1Button::Up, set.d_pad_up),
                (Ps1Button::Down, set.d_pad_down),
                (Ps1Button::Left, set.d_pad_left),
                (Ps1Button::Right, set.d_pad_right),
                (Ps1Button::Cross, set.cross),
                (Ps1Button::Circle, set.circle),
                (Ps1Button::Square, set.square),
                (Ps1Button::Triangle, set.triangle),
                (Ps1Button::L1, set.l1),
                (Ps1Button::L2, set.l2),
                (Ps1Button::R1, set.r1),
                (Ps1Button::R2, set.r2),
                (Ps1Button::Start, set.start),
                (Ps1Button::Select, set.select),
                (Ps1Button::Analog, set.analog),
                // TODO L3/R3
            ] {
                let Some(field) = field else { continue };
                digital_inputs.entry(field).or_default().push(DigitalValue {
                    button,
                    axis_deadzone: set.gamepad_axis_deadzone,
                    trigger_threshold: set.gamepad_trigger_threshold,
                });
            }

            for ((analog_input, direction), field) in [
                ((Ps1AnalogInput::LeftStickX, AxisDirection::Negative), set.l_stick_left),
                ((Ps1AnalogInput::LeftStickX, AxisDirection::Positive), set.l_stick_right),
                ((Ps1AnalogInput::LeftStickY, AxisDirection::Negative), set.l_stick_up),
                ((Ps1AnalogInput::LeftStickY, AxisDirection::Positive), set.l_stick_down),
                ((Ps1AnalogInput::RightStickX, AxisDirection::Negative), set.r_stick_left),
                ((Ps1AnalogInput::RightStickX, AxisDirection::Positive), set.r_stick_right),
                ((Ps1AnalogInput::RightStickY, AxisDirection::Negative), set.r_stick_up),
                ((Ps1AnalogInput::RightStickY, AxisDirection::Positive), set.r_stick_down),
            ] {
                let Some(field) = field else { continue };
                analog_inputs.entry(field).or_default().push(AnalogValue {
                    input: analog_input,
                    direction,
                    axis_deadzone: set.gamepad_axis_deadzone,
                });
            }
        }

        Self { digital_inputs, analog_inputs }
    }

    pub fn map_keyboard(&self, keycode: KeyCode, pressed: bool, proxy: &EventLoopProxy<UserEvent>) {
        let input = SingleInput::Keyboard { keycode };

        if let Some(digital_inputs) = self.digital_inputs.get(&input) {
            send_digital_events(digital_inputs, pressed, proxy);
        }

        if let Some(analog_inputs) = self.analog_inputs.get(&input) {
            send_analog_events(analog_inputs, pressed, proxy);
        }
    }

    pub fn map_sdl_button(
        &self,
        which: u32,
        button: SdlButton,
        pressed: bool,
        proxy: &EventLoopProxy<UserEvent>,
    ) {
        let input = SingleInput::SdlGamepad {
            controller_idx: which,
            sdl_input: SdlGamepadInput::from_sdl_button(button),
        };

        if let Some(digital_inputs) = self.digital_inputs.get(&input) {
            send_digital_events(digital_inputs, pressed, proxy);
        }

        if let Some(analog_inputs) = self.analog_inputs.get(&input) {
            send_analog_events(analog_inputs, pressed, proxy);
        }
    }

    #[allow(clippy::missing_panics_doc)]
    pub fn map_sdl_axis(
        &self,
        which: u32,
        axis: SdlAxis,
        value: i16,
        proxy: &EventLoopProxy<UserEvent>,
    ) {
        let input = SingleInput::SdlGamepad {
            controller_idx: which,
            sdl_input: SdlGamepadInput::from_sdl_axis(axis, value),
        };

        if let Some(digital_inputs) = self.digital_inputs.get(&input) {
            for digital_input in digital_inputs {
                let pressed = match axis {
                    SdlAxis::TriggerLeft | SdlAxis::TriggerRight => {
                        value.saturating_abs() > digital_input.trigger_threshold
                    }
                    _ => value.saturating_abs() > digital_input.axis_deadzone,
                };
                proxy
                    .send_event(UserEvent::ControllerButton {
                        button: digital_input.button,
                        pressed,
                    })
                    .unwrap();
            }
        }

        if let Some(analog_inputs) = self.analog_inputs.get(&input) {
            for analog_input in analog_inputs {
                let clamped_value =
                    if value.saturating_abs() > analog_input.axis_deadzone { value } else { 0 };
                proxy
                    .send_event(UserEvent::ControllerAnalog {
                        input: analog_input.input,
                        value: clamped_value,
                    })
                    .unwrap();
            }
        }
    }
}

fn send_digital_events(
    digital_inputs: &[DigitalValue],
    pressed: bool,
    proxy: &EventLoopProxy<UserEvent>,
) {
    for &DigitalValue { button, .. } in digital_inputs {
        proxy.send_event(UserEvent::ControllerButton { button, pressed }).unwrap();
    }
}

fn send_analog_events(
    analog_inputs: &[AnalogValue],
    pressed: bool,
    proxy: &EventLoopProxy<UserEvent>,
) {
    for &AnalogValue { input, direction, .. } in analog_inputs {
        let value = if pressed { direction.max_value() } else { 0 };
        proxy.send_event(UserEvent::ControllerAnalog { input, value }).unwrap();
    }
}
