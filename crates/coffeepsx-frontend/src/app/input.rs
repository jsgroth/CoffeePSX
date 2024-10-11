use crate::app::{App, AppEventResponse, AppState};
use crate::config::InputConfig;
use crate::config::input::{ControllerConfig, SdlGamepadInput, SingleInput};
use egui::{Grid, ScrollArea, Slider, Ui};
use sdl2::controller::Axis as SdlAxis;
use sdl2::controller::Button as SdlButton;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControllerNumber {
    One,
    Two,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputSet {
    One,
    Two,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigurableInput {
    DPadUp,
    DPadDown,
    DPadLeft,
    DPadRight,
    Cross,
    Circle,
    Square,
    Triangle,
    L1,
    L2,
    R1,
    R2,
    Start,
    Select,
    Analog,
    LStickUp,
    LStickDown,
    LStickLeft,
    LStickRight,
    RStickUp,
    RStickDown,
    RStickLeft,
    RStickRight,
    L3,
    R3,
}

impl ConfigurableInput {
    const DIGITAL: &'static [Self] = &[
        Self::DPadUp,
        Self::DPadDown,
        Self::DPadLeft,
        Self::DPadRight,
        Self::Cross,
        Self::Circle,
        Self::Square,
        Self::Triangle,
        Self::L1,
        Self::L2,
        Self::R1,
        Self::R2,
        Self::Start,
        Self::Select,
    ];

    const ANALOG: &'static [Self] = &[
        Self::Analog,
        Self::LStickUp,
        Self::LStickDown,
        Self::LStickLeft,
        Self::LStickRight,
        Self::RStickUp,
        Self::RStickDown,
        Self::RStickLeft,
        Self::RStickRight,
        Self::L3,
        Self::R3,
    ];

    fn button_label(self) -> &'static str {
        match self {
            Self::DPadUp => "D-Pad Up",
            Self::DPadDown => "D-Pad Down",
            Self::DPadLeft => "D-Pad Left",
            Self::DPadRight => "D-Pad Right",
            Self::Cross => "Cross",
            Self::Circle => "Circle",
            Self::Square => "Square",
            Self::Triangle => "Triangle",
            Self::L1 => "L1",
            Self::L2 => "L2",
            Self::R1 => "R1",
            Self::R2 => "R2",
            Self::Start => "Start",
            Self::Select => "Select",
            Self::Analog => "Analog Button",
            Self::LStickUp => "Left Stick - Up",
            Self::LStickDown => "Left Stick - Down",
            Self::LStickLeft => "Left Stick - Left",
            Self::LStickRight => "Left Stick - Right",
            Self::RStickUp => "Right Stick - Up",
            Self::RStickDown => "Right Stick - Down",
            Self::RStickLeft => "Right Stick - Left",
            Self::RStickRight => "Right Stick - Right",
            Self::L3 => "L3",
            Self::R3 => "R3",
        }
    }

    fn get_field(self, config: &mut ControllerConfig) -> &mut Option<SingleInput> {
        match self {
            Self::DPadUp => &mut config.d_pad_up,
            Self::DPadDown => &mut config.d_pad_down,
            Self::DPadLeft => &mut config.d_pad_left,
            Self::DPadRight => &mut config.d_pad_right,
            Self::Cross => &mut config.cross,
            Self::Circle => &mut config.circle,
            Self::Square => &mut config.square,
            Self::Triangle => &mut config.triangle,
            Self::L1 => &mut config.l1,
            Self::L2 => &mut config.l2,
            Self::R1 => &mut config.r1,
            Self::R2 => &mut config.r2,
            Self::Start => &mut config.start,
            Self::Select => &mut config.select,
            Self::Analog => &mut config.analog,
            Self::LStickUp => &mut config.l_stick_up,
            Self::LStickDown => &mut config.l_stick_down,
            Self::LStickLeft => &mut config.l_stick_left,
            Self::LStickRight => &mut config.l_stick_right,
            Self::RStickUp => &mut config.r_stick_up,
            Self::RStickDown => &mut config.r_stick_down,
            Self::RStickLeft => &mut config.r_stick_left,
            Self::RStickRight => &mut config.r_stick_right,
            Self::L3 => &mut config.l3,
            Self::R3 => &mut config.r3,
        }
    }
}

impl App {
    pub(super) fn render_input_set_settings(&mut self, ui: &mut Ui) {
        ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            let set_config = get_input_set_config(
                self.state.selected_controller,
                self.state.selected_input_set,
                &mut self.config.input,
            );

            ui.heading("Digital Controller Inputs");
            ui.add_space(5.0);

            Grid::new("digital_inputs_grid").show(ui, |ui| {
                for &input in ConfigurableInput::DIGITAL {
                    render_single_input_row(input, set_config, &mut self.state, ui);
                }
            });

            ui.add_space(10.0);

            ui.heading("Analog Controller Inputs");
            ui.add_space(5.0);

            Grid::new("analog_inputs_grid").show(ui, |ui| {
                for &input in ConfigurableInput::ANALOG {
                    render_single_input_row(input, set_config, &mut self.state, ui);
                }
            });

            ui.add_space(15.0);

            ui.horizontal(|ui| {
                ui.add(Slider::new(&mut set_config.gamepad_axis_deadzone, 0..=i16::MAX));
                ui.label("Gamepad joystick axis deadzone");
            });

            ui.add_space(5.0);

            ui.horizontal(|ui| {
                ui.add(Slider::new(&mut set_config.gamepad_trigger_threshold, 0..=i16::MAX));
                ui.label("Gamepad trigger press threshold");
            });

            ui.add_space(15.0);

            ui.horizontal(|ui| {
                if ui.button("Default Keyboard").clicked() {
                    *set_config = ControllerConfig::default_p1_keyboard();
                }

                if ui.button("Default Gamepad").clicked() {
                    *set_config = ControllerConfig::default_p1_gamepad();
                }

                if ui.button("Clear All").clicked() {
                    *set_config = ControllerConfig::none();
                }
            });
        });
    }

    pub(super) fn handle_sdl_button_press(
        &mut self,
        which: u32,
        button: SdlButton,
    ) -> AppEventResponse {
        let Some((controller_number, input_set, configurable_input)) =
            self.state.waiting_for_input.take()
        else {
            return AppEventResponse { repaint: false };
        };

        self.update_input(
            controller_number,
            input_set,
            configurable_input,
            Some(SingleInput::SdlGamepad {
                controller_idx: which,
                sdl_input: SdlGamepadInput::from_sdl_button(button),
            }),
        );

        AppEventResponse { repaint: true }
    }

    pub(super) fn handle_sdl_axis_motion(
        &mut self,
        which: u32,
        axis: SdlAxis,
        value: i16,
    ) -> AppEventResponse {
        let Some((controller_number, input_set, configurable_input)) = self.state.waiting_for_input
        else {
            return AppEventResponse { repaint: false };
        };

        // Require a fairly large press to register the input
        if value.saturating_abs() <= (i16::MAX / 2) {
            return AppEventResponse { repaint: false };
        }

        self.state.waiting_for_input = None;
        self.update_input(
            controller_number,
            input_set,
            configurable_input,
            Some(SingleInput::SdlGamepad {
                controller_idx: which,
                sdl_input: SdlGamepadInput::from_sdl_axis(axis, value),
            }),
        );

        AppEventResponse { repaint: true }
    }

    pub(super) fn update_input(
        &mut self,
        controller_number: ControllerNumber,
        input_set: InputSet,
        configurable_input: ConfigurableInput,
        input: Option<SingleInput>,
    ) {
        let set_config = get_input_set_config(controller_number, input_set, &mut self.config.input);
        *configurable_input.get_field(set_config) = input;
    }
}

fn render_single_input_row(
    input: ConfigurableInput,
    set_config: &mut ControllerConfig,
    app_state: &mut AppState,
    ui: &mut Ui,
) {
    ui.label(format!("{}:", input.button_label()));

    let field = input.get_field(set_config);

    let button_label = if app_state
        .waiting_for_input
        .is_some_and(|(_, _, waiting_input)| waiting_input == input)
    {
        "Waiting for input...".into()
    } else {
        field.map_or_else(|| "<None>".into(), stringify_input)
    };

    if ui.button(button_label).clicked() {
        app_state.waiting_for_input =
            Some((app_state.selected_controller, app_state.selected_input_set, input));
    }

    if ui.button("Clear").clicked() {
        *input.get_field(set_config) = None;
    }

    ui.end_row();
}

fn stringify_input(input: SingleInput) -> String {
    match input {
        SingleInput::Keyboard { keycode } => format!("Keyboard: {keycode:?}"),
        SingleInput::SdlGamepad { controller_idx, sdl_input } => {
            format!("SDL-{controller_idx}: {sdl_input}")
        }
    }
}

fn get_input_set_config(
    controller_number: ControllerNumber,
    input_set: InputSet,
    input_config: &mut InputConfig,
) -> &mut ControllerConfig {
    match (controller_number, input_set) {
        (ControllerNumber::One, InputSet::One) => &mut input_config.p1_set_1,
        (ControllerNumber::One, InputSet::Two) => &mut input_config.p1_set_2,
        (ControllerNumber::Two, InputSet::One) => &mut input_config.p2_set_1,
        (ControllerNumber::Two, InputSet::Two) => &mut input_config.p2_set_2,
    }
}
