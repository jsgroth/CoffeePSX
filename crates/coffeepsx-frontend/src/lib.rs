use crate::emuthread::{Ps1AnalogInput, Ps1Button};
use std::path::PathBuf;

pub mod app;
pub mod config;
pub mod emustate;
pub mod emuthread;
pub mod guistate;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenFileType {
    Open,
    BiosPath,
    SearchDir,
}

#[derive(Debug)]
pub enum UserEvent {
    OpenFile { file_type: OpenFileType, initial_dir: Option<PathBuf> },
    FileOpened(OpenFileType, Option<PathBuf>),
    RunBios,
    AppConfigChanged,
    Close,
    ControllerButton { button: Ps1Button, pressed: bool },
    ControllerAnalog { input: Ps1AnalogInput, value: i16 },
}

// Enum with no variants cannot be instantiated
#[derive(Debug, Clone, Copy)]
pub enum Never {}
