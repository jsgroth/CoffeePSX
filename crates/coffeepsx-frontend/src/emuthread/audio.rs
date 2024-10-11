use crate::Never;
use crate::config::AudioConfig;
use ps1_core::api::AudioOutput;
use sdl2::audio::{AudioCallback, AudioSpecDesired};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

pub const FREQUENCY: i32 = 44100;
pub const CHANNELS: u8 = 2;

pub fn new_spec(config: &AudioConfig) -> AudioSpecDesired {
    AudioSpecDesired {
        freq: Some(FREQUENCY),
        channels: Some(CHANNELS),
        samples: Some(config.device_queue_size),
    }
}

pub type AudioQueue = Arc<Mutex<VecDeque<(i16, i16)>>>;

pub struct QueueAudioCallback {
    audio_queue: AudioQueue,
}

impl QueueAudioCallback {
    pub fn new(audio_queue: AudioQueue) -> Self {
        Self { audio_queue }
    }
}

impl AudioCallback for QueueAudioCallback {
    type Channel = i16;

    fn callback(&mut self, out: &mut [Self::Channel]) {
        let mut queue = self.audio_queue.lock().unwrap();
        for chunk in out.chunks_exact_mut(2) {
            let (l, r) = queue.pop_front().unwrap_or((0, 0));
            chunk[0] = l;
            chunk[1] = r;
        }
    }
}

pub struct QueueAudioOutput {
    audio_queue: AudioQueue,
}

impl QueueAudioOutput {
    pub fn new(audio_queue: AudioQueue) -> Self {
        Self { audio_queue }
    }

    pub fn samples_len(&self) -> usize {
        self.audio_queue.lock().unwrap().len()
    }

    pub fn truncate_front(&self, len: usize) {
        let mut audio_queue = self.audio_queue.lock().unwrap();
        while audio_queue.len() > len {
            audio_queue.pop_front();
        }
    }
}

impl AudioOutput for QueueAudioOutput {
    type Err = Never;

    fn queue_samples(&mut self, samples: &[(i16, i16)]) -> Result<(), Self::Err> {
        let mut queue = self.audio_queue.lock().unwrap();
        for &(sample_l, sample_r) in samples {
            queue.push_back((sample_l, sample_r));
        }

        Ok(())
    }
}
