#[allow(clippy::wildcard_imports)]
use crate::cd::macros::*;
use crate::cd::status::ErrorFlags;
use crate::cd::{CdController, CommandState};

impl CdController {
    // $0C: Demute() -> INT3(stat)
    // Demutes CD audio output, both CD-DA and ADPCM
    pub(super) fn execute_demute(&mut self) -> CommandState {
        log::warn!("Demute command not yet implemented");

        int3!(self, [self.status_code(ErrorFlags::NONE)]);

        CommandState::Idle
    }
}
