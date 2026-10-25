//! Sound output for hosts that pull it from a real-time audio callback.
//!
//! The callback must not wait on a lock or allocate: the game thread holds
//! the mixer's lock while it opens sounds (Ogg headers, seeks), and mixing
//! decodes Vorbis, which allocates. So a feeder thread of our own mixes
//! ahead into a lock-free ring, and the callback only copies out of it.
//! While the host's audio is stopped the ring stays full and the feeder
//! waits, so sounds pause with it.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use yuris_host::audio::Mixer;

/// Frames mixed per lock of the mixer.
const CHUNK: usize = 256;
/// Frames held ahead of the callback: about 93 ms at 44.1 kHz. Sound
/// positions the game reads (SOUNDINFO TIME) run this far ahead of what
/// is heard.
const AHEAD: usize = 4096;

/// The feeder thread and the ring's reading end.
pub struct AudioOutput {
    ring: rtrb::Consumer<f32>,
    stop: Arc<AtomicBool>,
    feeder: Option<JoinHandle<()>>,
}

impl AudioOutput {
    /// Starts feeding from `mixer`.
    pub fn new(mixer: Arc<Mutex<Mixer>>) -> std::io::Result<Self> {
        let (mut producer, ring) = rtrb::RingBuffer::<f32>::new(2 * AHEAD);
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();
        let feeder = std::thread::Builder::new().name("yuris-audio".into()).spawn(move || {
            let mut buf = vec![0.0f32; 2 * CHUNK];
            while !stopping.load(Ordering::Relaxed) {
                if producer.slots() < buf.len() {
                    std::thread::sleep(Duration::from_millis(3));
                    continue;
                }
                match mixer.lock() {
                    Ok(mut m) => m.render(&mut buf),
                    Err(p) => p.into_inner().render(&mut buf),
                }
                if let Ok(chunk) = producer.write_chunk_uninit(buf.len()) {
                    chunk.fill_from_iter(buf.iter().copied());
                }
            }
        })?;
        Ok(Self { ring, stop, feeder: Some(feeder) })
    }

    /// Fills `out` (interleaved stereo) from the ring; what the feeder has
    /// not mixed yet is silence. Never blocks or allocates.
    pub fn render(&mut self, out: &mut [f32]) {
        let n = self.ring.slots().min(out.len()) & !1;
        let mut done = 0;
        if let Ok(chunk) = self.ring.read_chunk(n) {
            let (a, b) = chunk.as_slices();
            out[..a.len()].copy_from_slice(a);
            out[a.len()..a.len() + b.len()].copy_from_slice(b);
            done = a.len() + b.len();
            chunk.commit_all();
        }
        out[done..].fill(0.0);
    }
}

impl Drop for AudioOutput {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.feeder.take() {
            let _ = t.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn feeds_ahead_and_stops() {
        let mixer = Arc::new(Mutex::new(Mixer::default()));
        let mut out = AudioOutput::new(mixer.clone()).unwrap();
        let mut buf = vec![1.0f32; 512];
        // An empty mixer gives silence, through the ring or as underrun.
        for _ in 0..20 {
            out.render(&mut buf);
            assert!(buf.iter().all(|&s| s == 0.0));
            std::thread::sleep(Duration::from_millis(5));
        }
        // The callback never waits on the mixer's lock.
        let held = mixer.lock().unwrap();
        let t = std::time::Instant::now();
        out.render(&mut buf);
        assert!(t.elapsed() < Duration::from_millis(50));
        drop(held);
        drop(out);
    }
}
