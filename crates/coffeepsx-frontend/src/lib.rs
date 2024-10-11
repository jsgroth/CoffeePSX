pub mod app;
pub mod config;
pub mod emustate;
pub mod emuthread;
pub mod guistate;
pub mod input;

use crate::emuthread::{Player, Ps1AnalogInput, Ps1Button};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenFileType {
    Open,
    BiosPath,
    SearchDir,
    DiscChange,
}

#[derive(Debug)]
pub enum UserEvent {
    OpenFileDialog { file_type: OpenFileType, initial_dir: Option<PathBuf> },
    FileOpened(OpenFileType, Option<PathBuf>),
    RunBios,
    AppConfigChanged,
    Close,
    ControllerButton { player: Player, button: Ps1Button, pressed: bool },
    ControllerAnalog { player: Player, input: Ps1AnalogInput, value: i16 },
    RemoveDisc,
    SdlButtonPress { which: u32, button: sdl2::controller::Button },
    SdlAxisMotion { which: u32, axis: sdl2::controller::Axis, value: i16 },
}

// Enum with no variants cannot be instantiated
#[derive(Debug, Clone, Copy)]
pub enum Never {}
