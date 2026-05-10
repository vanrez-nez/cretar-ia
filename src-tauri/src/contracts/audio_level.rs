use serde::Serialize;
use tokio::sync::mpsc;

pub const AUDIO_LEVEL_QUEUE_CAPACITY: usize = 16;
pub const AUDIO_SPECTRUM_BANDS: usize = 12;

#[derive(Clone, Copy, Debug, Serialize)]
pub struct AudioLevelSample {
    pub levels: [f32; AUDIO_SPECTRUM_BANDS],
}

pub type AudioLevelSender = mpsc::Sender<AudioLevelSample>;
pub type AudioLevelReceiver = mpsc::Receiver<AudioLevelSample>;

pub fn bounded_audio_level_channel(capacity: usize) -> (AudioLevelSender, AudioLevelReceiver) {
    mpsc::channel(capacity.max(1))
}
