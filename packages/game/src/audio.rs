use crate::game_state::SoundCue;

pub struct AudioBus;

impl AudioBus {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    pub const fn play(_cues: &[SoundCue]) {}
}
