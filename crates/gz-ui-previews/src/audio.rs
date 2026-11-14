//! The Sound Library's playback: one audio thread that owns the output stream (`cpal`'s stream
//! is not `Send` everywhere), fed decoded sounds over a channel. Decoding happens on the pools;
//! nothing here blocks a frame.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use crossbeam_channel::{Receiver, Sender, unbounded};
use gz_editor::Editor;
use gz_fmt_sound::Pcm;
use parking_lot::Mutex;

enum Msg {
    Play(Arc<Pcm>),
    Stop,
}

/// What the callback reads: the sound and where it is in it, in source frames (16.16).
#[derive(Default)]
struct Voice {
    pcm: Option<Arc<Pcm>>,
    pos: u64,
}

/// Fills `out` (interleaved, `channels` a frame, `rate` frames a second) from the voice: the
/// sound resampled by its nearest frame, mono spread over every channel, a missing channel
/// taken from the last one. Returns whether the sound ended.
fn mix(v: &mut Voice, out: &mut [f32], channels: u16, rate: u32) -> bool {
    out.fill(0.0);
    let Some(pcm) = v.pcm.clone() else { return false };
    let ch_out = usize::from(channels.max(1));
    let ch_in = usize::from(pcm.channels.max(1));
    let total = pcm.samples.len() / ch_in;
    let step = (u64::from(pcm.rate.max(1)) << 16) / u64::from(rate.max(1));
    for frame in out.chunks_exact_mut(ch_out) {
        let at = usize::try_from(v.pos >> 16).unwrap_or(usize::MAX);
        if at >= total {
            v.pcm = None;
            v.pos = 0;
            return true;
        }
        for (c, s) in frame.iter_mut().enumerate() {
            *s = f32::from(pcm.samples.get(at * ch_in + c.min(ch_in - 1)).copied().unwrap_or(0)) / 32768.0;
        }
        v.pos += step;
    }
    false
}

/// The editor's player: an `Editor` service, made on first use.
pub struct Player {
    tx: Sender<Msg>,
    playing: Arc<AtomicBool>,
    error: Arc<Mutex<Option<String>>>,
}

impl Player {
    #[must_use]
    pub fn of(editor: &Editor) -> Arc<Player> {
        let wake = editor.exec.wake().clone();
        let log = editor.log.clone();
        // View > Mute Sounds and a mute held for a while: the sound plays on, unheard
        let muted = gz_app::gpu::Gpu::of(editor).mute_flag();
        editor.services.get_or_init(move || {
            let (tx, rx) = unbounded();
            let playing = Arc::new(AtomicBool::new(false));
            let error = Arc::new(Mutex::new(None));
            let (p, e) = (playing.clone(), error.clone());
            if let Err(err) = std::thread::Builder::new()
                .name("gz-audio".into())
                .spawn(move || run(&rx, &p, &e, &wake, &muted))
            {
                log.warn(format!("the audio thread did not start: {err}"));
            }
            Player { tx, playing, error }
        })
    }

    pub fn play(&self, pcm: Arc<Pcm>) {
        self.playing.store(true, Ordering::Release);
        let _ = self.tx.send(Msg::Play(pcm));
    }

    pub fn stop(&self) {
        self.playing.store(false, Ordering::Release);
        let _ = self.tx.send(Msg::Stop);
    }

    #[must_use]
    pub fn is_playing(&self) -> bool {
        self.playing.load(Ordering::Acquire)
    }

    /// Why nothing can be played, once the device refused.
    #[must_use]
    pub fn error(&self) -> Option<String> {
        self.error.lock().clone()
    }
}

/// The stream, opened when the first sound is played and kept while the thread lives.
fn open(
    voice: &Arc<Mutex<Voice>>,
    playing: &Arc<AtomicBool>,
    wake: &gz_task::Wake,
    muted: &Arc<AtomicBool>,
) -> Result<cpal::Stream, String> {
    let host = cpal::default_host();
    let device = host.default_output_device().ok_or_else(|| "There is no audio output device.".to_owned())?;
    let supported = device
        .default_output_config()
        .map_err(|e| format!("The audio device refused its own settings: {e}"))?;
    let format = supported.sample_format();
    let config = supported.config();
    let (rate, channels) = (config.sample_rate, config.channels);
    let ended = {
        let (playing, wake) = (playing.clone(), wake.clone());
        move || {
            playing.store(false, Ordering::Release);
            wake.ping();
        }
    };
    // a stream error ends nothing here: the device is opened again by the next play when it went
    let on_error = |_e: cpal::Error| {};
    let stream = match format {
        cpal::SampleFormat::F32 => {
            let (voice, muted) = (voice.clone(), muted.clone());
            device.build_output_stream::<f32, _, _>(
                config,
                move |data: &mut [f32], _| {
                    let done = mix(&mut voice.lock(), data, channels, rate);
                    if muted.load(Ordering::Acquire) {
                        data.fill(0.0);
                    }
                    if done {
                        ended();
                    }
                },
                on_error,
                None,
            )
        }
        cpal::SampleFormat::I16 => {
            let (voice, muted) = (voice.clone(), muted.clone());
            let mut scratch: Vec<f32> = Vec::new();
            device.build_output_stream::<i16, _, _>(
                config,
                move |data: &mut [i16], _| {
                    scratch.resize(data.len(), 0.0);
                    let done = mix(&mut voice.lock(), &mut scratch, channels, rate);
                    if muted.load(Ordering::Acquire) {
                        scratch.fill(0.0);
                    }
                    for (d, s) in data.iter_mut().zip(&scratch) {
                        *d = (s.clamp(-1.0, 1.0) * 32767.0) as i16;
                    }
                    if done {
                        ended();
                    }
                },
                on_error,
                None,
            )
        }
        other => {
            return Err(format!(
                "The audio device wants samples as {other}, which the editor does not play."
            ));
        }
    }
    .map_err(|e| format!("The audio device did not open: {e}"))?;
    stream.play().map_err(|e| format!("The audio device did not start: {e}"))?;
    Ok(stream)
}

fn run(
    rx: &Receiver<Msg>,
    playing: &Arc<AtomicBool>,
    error: &Arc<Mutex<Option<String>>>,
    wake: &gz_task::Wake,
    muted: &Arc<AtomicBool>,
) {
    let voice = Arc::new(Mutex::new(Voice::default()));
    let mut stream: Option<cpal::Stream> = None;
    while let Ok(msg) = rx.recv() {
        match msg {
            Msg::Play(pcm) => {
                if stream.is_none() {
                    match open(&voice, playing, wake, muted) {
                        Ok(s) => {
                            stream = Some(s);
                            *error.lock() = None;
                        }
                        Err(e) => {
                            *error.lock() = Some(e);
                            playing.store(false, Ordering::Release);
                            wake.ping();
                            continue;
                        }
                    }
                }
                let mut v = voice.lock();
                v.pcm = Some(pcm);
                v.pos = 0;
            }
            Msg::Stop => {
                let mut v = voice.lock();
                v.pcm = None;
                v.pos = 0;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mono_spreads_and_the_end_is_said() {
        let pcm = Arc::new(Pcm { channels: 1, rate: 100, samples: vec![16384, -16384] });
        let mut v = Voice { pcm: Some(pcm), pos: 0 };
        let mut out = [0.0f32; 4];
        // 2 output frames at the same rate: both samples, on both channels
        assert!(!mix(&mut v, &mut out, 2, 100));
        assert_eq!(out, [0.5, 0.5, -0.5, -0.5]);
        assert!(mix(&mut v, &mut out, 2, 100));
        assert!(v.pcm.is_none());
    }
}
