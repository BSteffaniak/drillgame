#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    reason = "procedural audio converts bounded sample positions to PCM values"
)]

use raylib::prelude::*;

use crate::game_state::SoundCue;

pub struct AudioBus {
    _audio: &'static RaylibAudio,
    drill: Sound<'static>,
    sell: Sound<'static>,
    upgrade: Sound<'static>,
    damage: Sound<'static>,
    milestone: Sound<'static>,
}

impl AudioBus {
    pub fn new() -> Result<Self, String> {
        let audio = Box::leak(Box::new(
            RaylibAudio::init_audio_device().map_err(|error| error.to_string())?,
        ));
        audio.set_master_volume(0.35);
        Ok(Self {
            _audio: audio,
            drill: sound(audio, 130.0, 0.045).ok_or("generated drill sound failed")?,
            sell: sound(audio, 680.0, 0.09).ok_or("generated sell sound failed")?,
            upgrade: sound(audio, 880.0, 0.11).ok_or("generated upgrade sound failed")?,
            damage: sound(audio, 90.0, 0.12).ok_or("generated damage sound failed")?,
            milestone: sound(audio, 520.0, 0.16).ok_or("generated milestone sound failed")?,
        })
    }

    pub fn play(&self, cues: &[SoundCue]) {
        for cue in cues {
            match cue {
                SoundCue::Drill => self.drill.play(),
                SoundCue::Sell => self.sell.play(),
                SoundCue::Upgrade => self.upgrade.play(),
                SoundCue::Damage => self.damage.play(),
                SoundCue::Milestone => self.milestone.play(),
            }
        }
    }
}

fn sound(audio: &'static RaylibAudio, frequency: f32, seconds: f32) -> Option<Sound<'static>> {
    let wave_bytes = wav_bytes(frequency, seconds);
    let wave = audio.new_wave_from_memory("wav", &wave_bytes).ok()?;
    audio.new_sound_from_wave(&wave).ok()
}

fn wav_bytes(frequency: f32, seconds: f32) -> Vec<u8> {
    let sample_rate = 22_050_u32;
    let samples = (sample_rate as f32 * seconds) as u32;
    let data_len = samples * 2;
    let mut bytes = Vec::with_capacity(44 + data_len as usize);
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data_len).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16_u32.to_le_bytes());
    bytes.extend_from_slice(&1_u16.to_le_bytes());
    bytes.extend_from_slice(&1_u16.to_le_bytes());
    bytes.extend_from_slice(&sample_rate.to_le_bytes());
    bytes.extend_from_slice(&(sample_rate * 2).to_le_bytes());
    bytes.extend_from_slice(&2_u16.to_le_bytes());
    bytes.extend_from_slice(&16_u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data_len.to_le_bytes());

    for sample in 0..samples {
        let t = sample as f32 / sample_rate as f32;
        let envelope = 1.0 - sample as f32 / samples as f32;
        let value = (t * frequency * std::f32::consts::TAU).sin() * envelope * 0.35;
        let pcm = (value * f32::from(i16::MAX)) as i16;
        bytes.extend_from_slice(&pcm.to_le_bytes());
    }

    bytes
}
