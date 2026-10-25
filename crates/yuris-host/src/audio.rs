//! SOUND layers and the mixer.
//!
//! From the manual (サウンド系命令): a SOUND layer holds one OGG or WAVE
//! file (FILE may leave out the extension: tried as is, then `.ogg`, then
//! `.wav`). PLAY=1 plays and PLAY=0 stops; LOOP 0 loops forever, n plays n
//! times. START is where play begins in ms (negative: start that much
//! later), RESTART where later loops begin, END the end / loop point.
//! VOLUME is 0-256, PAN 0-512 (left to right, 256 centre), PITCH 0-512
//! (0.5x to 1.5x). FADE is the time a PLAY or VOLUME change takes. E=0
//! mutes a layer whatever its volume. SOUNDEND deletes the layer.
//!
//! Layer IDs are paths: Erewhon sets the master volume on `:/S0/` and
//! plays on IDs below it. A layer's gain is its own volume times that of
//! every existing layer whose ID is a `/`-ended prefix of it, and a
//! disabled prefix mutes it. Not in the manual: volume is taken as linear
//! gain (VOLUME/256), the pan law is linear, a FADE on PLAY=1 starts from
//! silence, a FADE on PLAY=0 stops at its end, and PLAY=1 on a layer that
//! is already playing (no FILE in the same call) only applies the other
//! parameters (the manual's example 12 fades a playing BGM that way).
//!
//! The mixer is pulled: [`Mixer::render`] writes interleaved stereo f32 at
//! [`OUTPUT_RATE`] and advances every layer. Positions, ends and fades only
//! move when it is called, so a player calls it from its audio callback
//! and headless tools call it as their clock advances.

use std::collections::BTreeMap;
use std::io::Cursor;
use std::sync::Arc;

use lewton::inside_ogg::OggStreamReader;

/// Output sample rate of [`Mixer::render`].
pub const OUTPUT_RATE: u32 = 44_100;

/// Frames decoded per step for WAVE data.
const WAV_CHUNK: usize = 4096;

/// A decoded stream: interleaved i16 frames.
trait Decoder: Send {
    fn channels(&self) -> usize;
    fn rate(&self) -> u32;
    /// Total length in frames, when known.
    fn total_frames(&self) -> Option<u64>;
    /// Appends the next decoded frames to `out`; false at the end.
    fn decode(&mut self, out: &mut Vec<i16>) -> bool;
    /// Moves to `frame`; returns the frame the next decode starts at.
    fn seek(&mut self, frame: u64) -> u64;
}

/// Uncompressed WAVE (PCM 8/16/24/32-bit or float).
struct Wav {
    data: Vec<u8>,
    /// Byte range of the samples.
    start: usize,
    end: usize,
    pos: usize,
    channels: usize,
    rate: u32,
    bits: u16,
    float: bool,
}

impl Wav {
    /// Parses a RIFF WAVE file. `Err(true)` is a WAVE file with a
    /// compressed codec (CODEC answers 2 for those), `Err(false)` not a
    /// WAVE file.
    fn parse(data: Vec<u8>) -> Result<Self, bool> {
        if data.len() < 12 || &data[0..4] != b"RIFF" || &data[8..12] != b"WAVE" {
            return Err(false);
        }
        let mut fmt = None;
        let mut i = 12;
        while i + 8 <= data.len() {
            let id = &data[i..i + 4];
            let len = u32::from_le_bytes(data[i + 4..i + 8].try_into().unwrap()) as usize;
            let body = i + 8;
            let end = body.saturating_add(len).min(data.len());
            if id == b"fmt " && end - body >= 16 {
                let f = &data[body..end];
                let mut tag = u16::from_le_bytes([f[0], f[1]]);
                if tag == 0xfffe && f.len() >= 26 {
                    tag = u16::from_le_bytes([f[24], f[25]]);
                }
                let channels = u16::from_le_bytes([f[2], f[3]]) as usize;
                let rate = u32::from_le_bytes(f[4..8].try_into().unwrap());
                let bits = u16::from_le_bytes([f[14], f[15]]);
                fmt = Some((tag, channels, rate, bits));
            } else if id == b"data" {
                let (tag, channels, rate, bits) = fmt.ok_or(false)?;
                let float = tag == 3 && bits == 32;
                if !(tag == 1 && matches!(bits, 8 | 16 | 24 | 32) || float) || channels == 0 || rate == 0 {
                    return Err(true);
                }
                let frame = channels * usize::from(bits / 8);
                let end = body + (end - body) / frame * frame;
                return Ok(Self { data, start: body, end, pos: body, channels, rate, bits, float });
            }
            i = body + len + (len & 1);
        }
        Err(false)
    }

    fn frame_bytes(&self) -> usize {
        self.channels * usize::from(self.bits / 8)
    }
}

impl Decoder for Wav {
    fn channels(&self) -> usize {
        self.channels
    }

    fn rate(&self) -> u32 {
        self.rate
    }

    fn total_frames(&self) -> Option<u64> {
        Some(((self.end - self.start) / self.frame_bytes()) as u64)
    }

    fn decode(&mut self, out: &mut Vec<i16>) -> bool {
        if self.pos >= self.end {
            return false;
        }
        let stop = (self.pos + WAV_CHUNK * self.frame_bytes()).min(self.end);
        let b = &self.data[self.pos..stop];
        match (self.bits, self.float) {
            (8, _) => out.extend(b.iter().map(|&s| (i16::from(s) - 128) << 8)),
            (16, _) => out.extend(b.chunks_exact(2).map(|s| i16::from_le_bytes([s[0], s[1]]))),
            (24, _) => out.extend(b.chunks_exact(3).map(|s| i16::from_le_bytes([s[1], s[2]]))),
            (32, false) => out.extend(b.chunks_exact(4).map(|s| i16::from_le_bytes([s[2], s[3]]))),
            _ => out.extend(b.chunks_exact(4).map(|s| {
                let f = f32::from_le_bytes([s[0], s[1], s[2], s[3]]);
                (f.clamp(-1.0, 1.0) * 32767.0) as i16
            })),
        }
        self.pos = stop;
        true
    }

    fn seek(&mut self, frame: u64) -> u64 {
        let fb = self.frame_bytes();
        let max = ((self.end - self.start) / fb) as u64;
        let f = frame.min(max);
        self.pos = self.start + f as usize * fb;
        f
    }
}

/// Ogg Vorbis through lewton.
struct Ogg {
    data: Arc<[u8]>,
    reader: OggStreamReader<Cursor<Arc<[u8]>>>,
    total: Option<u64>,
    /// Frames still to drop (after a seek).
    skip: u64,
    /// Frames decoded since the start of the stream.
    decoded: u64,
}

impl Ogg {
    fn open(data: Vec<u8>) -> Option<Self> {
        let total = last_granule(&data);
        let data: Arc<[u8]> = data.into();
        let reader = OggStreamReader::new(Cursor::new(data.clone())).ok()?;
        Some(Self { data, reader, total, skip: 0, decoded: 0 })
    }
}

/// The granule position of the last Ogg page: the stream's length in
/// frames.
fn last_granule(data: &[u8]) -> Option<u64> {
    let from = data.len().saturating_sub(64 * 1024);
    let at = data[from..].windows(4).rposition(|w| w == b"OggS")? + from;
    let g = data.get(at + 6..at + 14)?;
    let g = u64::from_le_bytes(g.try_into().ok()?);
    (g != u64::MAX).then_some(g)
}

impl Decoder for Ogg {
    fn channels(&self) -> usize {
        usize::from(self.reader.ident_hdr.audio_channels)
    }

    fn rate(&self) -> u32 {
        self.reader.ident_hdr.audio_sample_rate
    }

    fn total_frames(&self) -> Option<u64> {
        self.total
    }

    fn decode(&mut self, out: &mut Vec<i16>) -> bool {
        let ch = self.channels().max(1);
        loop {
            match self.reader.read_dec_packet_itl() {
                Ok(Some(p)) => {
                    // The last page's granule position marks the real end;
                    // the final packet decodes past it.
                    let mut frames = (p.len() / ch) as u64;
                    if let Some(t) = self.total {
                        frames = frames.min(t.saturating_sub(self.decoded));
                    }
                    self.decoded += frames;
                    if frames == 0 && self.total.is_some_and(|t| self.decoded >= t) {
                        return false;
                    }
                    if self.skip >= frames {
                        self.skip -= frames;
                        continue;
                    }
                    out.extend_from_slice(&p[self.skip as usize * ch..frames as usize * ch]);
                    self.skip = 0;
                    return true;
                }
                Ok(None) | Err(_) => return false,
            }
        }
    }

    /// Restarts the stream and skips `frame` frames as they decode:
    /// exact, which loop points need, and Vorbis decodes far faster than
    /// real time.
    fn seek(&mut self, frame: u64) -> u64 {
        match OggStreamReader::new(Cursor::new(self.data.clone())) {
            Ok(r) => self.reader = r,
            Err(_) => return 0,
        }
        self.decoded = 0;
        self.skip = frame;
        frame
    }
}

/// Opens a sound file: Ogg Vorbis or WAVE. The second value is the CODEC
/// answer (0 PCM WAVE, 1 Ogg Vorbis, 2 compressed WAVE).
fn open_decoder(data: Vec<u8>) -> (Option<Box<dyn Decoder>>, i64) {
    if data.starts_with(b"OggS") {
        return (Ogg::open(data).map(|d| Box::new(d) as Box<dyn Decoder>), 1);
    }
    match Wav::parse(data) {
        Ok(w) => (Some(Box::new(w)), 0),
        Err(true) => (None, 2),
        Err(false) => (None, 0),
    }
}

/// A volume ramp toward `target` (a gain 0..=1).
#[derive(Debug, Clone, Copy)]
struct Fade {
    target: f32,
    /// Gain change per output frame.
    step: f32,
    /// Stop playing when the ramp ends (PLAY=0 with FADE).
    stop: bool,
}

/// One SOUND layer.
pub struct SoundLayer {
    /// FILE as given.
    pub file: Vec<u8>,
    /// E.
    pub enabled: bool,
    /// Whether it is playing (including a START delay and a fade-out).
    pub playing: bool,
    /// PAUSE.
    pub paused: bool,
    /// LOOP: 0 forever, n plays n times.
    pub loop_count: i64,
    /// Completed plays since PLAY=1 (LOOPCOUNT).
    pub loops_done: i64,
    /// START, RESTART, END in ms.
    pub start_ms: i64,
    /// RESTART in ms.
    pub restart_ms: i64,
    /// END in ms (0: the end of the file).
    pub end_ms: i64,
    /// VOLUME, 0..=256.
    pub volume: i64,
    /// PAN, 0..=512.
    pub pan: i64,
    /// PITCH, 0..=512.
    pub pitch: i64,
    /// CODEC answer.
    pub codec: i64,
    decoder: Option<Box<dyn Decoder>>,
    channels: usize,
    rate: u32,
    total: Option<u64>,
    /// Decoded frames from `buf_start` on.
    buf: Vec<i16>,
    buf_start: u64,
    /// Read position in source frames.
    pos: f64,
    /// The decoder has nothing more after `buf`.
    eof: bool,
    /// Output frames still to wait before playing (negative START).
    delay: u64,
    /// Current gain from VOLUME and fades, 0..=1.
    gain: f32,
    fade: Option<Fade>,
    /// Peak levels of the last render, 0..=32767 (DATALEFT/DATARIGHT).
    peak: [i64; 2],
}

impl std::fmt::Debug for SoundLayer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SoundLayer")
            .field("file", &String::from_utf8_lossy(&self.file))
            .field("playing", &self.playing)
            .field("volume", &self.volume)
            .field("pos", &self.pos)
            .finish()
    }
}

impl Default for SoundLayer {
    fn default() -> Self {
        Self {
            file: Vec::new(),
            enabled: true,
            playing: false,
            paused: false,
            loop_count: 1,
            loops_done: 0,
            start_ms: 0,
            restart_ms: 0,
            end_ms: 0,
            volume: 256,
            pan: 256,
            pitch: 256,
            codec: 0,
            decoder: None,
            channels: 0,
            rate: 0,
            total: None,
            buf: Vec::new(),
            buf_start: 0,
            pos: 0.0,
            eof: true,
            delay: 0,
            gain: 1.0,
            fade: None,
            peak: [0; 2],
        }
    }
}

fn ms_to_frames(ms: i64, rate: u32) -> u64 {
    (ms.max(0) as u64).saturating_mul(u64::from(rate)) / 1000
}

impl SoundLayer {
    /// Loads decoded data (`None`: missing or unreadable; the layer then
    /// plays silence that ends at once).
    pub fn load(&mut self, file: Vec<u8>, data: Option<Vec<u8>>) {
        self.file = file;
        self.playing = false;
        self.paused = false;
        let (dec, codec) = data.map_or((None, 0), open_decoder);
        self.codec = codec;
        self.channels = dec.as_ref().map_or(0, |d| d.channels());
        self.rate = dec.as_ref().map_or(0, |d| d.rate());
        self.total = dec.as_ref().and_then(|d| d.total_frames());
        self.decoder = dec;
        self.reset_to(0);
    }

    fn reset_to(&mut self, frame: u64) {
        self.buf.clear();
        self.eof = self.decoder.is_none();
        let at = self.decoder.as_mut().map_or(0, |d| d.seek(frame));
        self.buf_start = at;
        self.pos = frame as f64;
    }

    /// Whether a decoder is loaded.
    pub fn has_data(&self) -> bool {
        self.decoder.is_some()
    }

    fn target_gain(&self) -> f32 {
        self.volume.clamp(0, 256) as f32 / 256.0
    }

    fn start_fade(&mut self, target: f32, ms: i64, stop: bool) {
        let frames = ms_to_frames(ms, OUTPUT_RATE).max(1) as f32;
        self.fade = Some(Fade { target, step: (target - self.gain) / frames, stop });
    }

    /// PLAY=1: from START, with an optional fade-in.
    fn play(&mut self, fade_ms: i64) {
        self.loops_done = 0;
        self.paused = false;
        if self.start_ms < 0 {
            self.delay = ms_to_frames(-self.start_ms, OUTPUT_RATE);
            self.reset_to(0);
        } else {
            self.delay = 0;
            let f = ms_to_frames(self.start_ms, self.rate);
            self.reset_to(f);
        }
        self.playing = true;
        if fade_ms > 0 {
            self.gain = 0.0;
            self.start_fade(self.target_gain(), fade_ms, false);
        } else {
            self.gain = self.target_gain();
            self.fade = None;
        }
    }

    fn stop(&mut self) {
        self.playing = false;
        self.fade = None;
        self.delay = 0;
        self.reset_to(0);
    }

    fn end_frame(&self) -> Option<u64> {
        if self.end_ms > 0 {
            let e = ms_to_frames(self.end_ms, self.rate);
            Some(self.total.map_or(e, |t| e.min(t)))
        } else {
            self.total
        }
    }

    /// Current position in ms (TIME).
    pub fn time_ms(&self) -> i64 {
        if self.rate == 0 {
            return 0;
        }
        (self.pos / f64::from(self.rate) * 1000.0) as i64
    }

    /// Length in ms (TOTALTIME).
    pub fn total_ms(&self) -> i64 {
        match (self.total, self.rate) {
            (Some(t), r) if r > 0 => (t as f64 / f64::from(r) * 1000.0) as i64,
            _ => 0,
        }
    }

    /// Channel count, bits and sample rate of the data (CHANNEL, BIT,
    /// SAMPLING).
    pub fn format(&self) -> (i64, i64, i64) {
        (self.channels as i64, if self.decoder.is_some() { 16 } else { 0 }, i64::from(self.rate))
    }

    /// Peak levels of the last render (DATALEFT, DATARIGHT).
    pub fn levels(&self) -> [i64; 2] {
        self.peak
    }

    /// Source frame `f` (channels folded to stereo), decoding as needed.
    fn frame(&mut self, f: u64) -> Option<[f32; 2]> {
        let ch = self.channels.max(1);
        while f >= self.buf_start + (self.buf.len() / ch) as u64 {
            if self.eof {
                return None;
            }
            // Drop what is behind the read position before growing.
            let behind = (self.pos as u64).saturating_sub(self.buf_start).saturating_sub(1) as usize;
            if behind > 0 && behind * ch <= self.buf.len() {
                self.buf.drain(..behind * ch);
                self.buf_start += behind as u64;
            }
            let more = self.decoder.as_mut().is_some_and(|d| d.decode(&mut self.buf));
            if !more {
                self.eof = true;
            }
        }
        let i = (f.checked_sub(self.buf_start)? as usize) * ch;
        let l = f32::from(self.buf[i]) / 32768.0;
        let r = if ch > 1 { f32::from(self.buf[i + 1]) / 32768.0 } else { l };
        Some([l, r])
    }

    /// The layer reached its end (or END): loop or stop.
    fn wrap(&mut self) {
        self.loops_done += 1;
        if self.loop_count == 0 || self.loops_done < self.loop_count {
            if self.restart_ms < 0 {
                self.delay = ms_to_frames(-self.restart_ms, OUTPUT_RATE);
                self.reset_to(0);
            } else {
                let f = ms_to_frames(self.restart_ms, self.rate);
                self.reset_to(f);
            }
        } else {
            self.stop();
        }
    }

    /// Mixes `frames` output frames into `out` (stereo, interleaved) with
    /// `parent` gain from the layers above.
    fn mix(&mut self, out: &mut [f32], parent: f32) {
        self.peak = [0; 2];
        if !self.playing || self.paused {
            return;
        }
        let frames = out.len() / 2;
        let speed = (0.5 + self.pitch.clamp(0, 512) as f64 / 512.0)
            * if self.rate > 0 { f64::from(self.rate) / f64::from(OUTPUT_RATE) } else { 1.0 };
        let pan = self.pan.clamp(0, 512) as f32;
        let (pl, pr) = (((512.0 - pan) / 256.0).min(1.0), (pan / 256.0).min(1.0));
        let audible = self.enabled && parent > 0.0;
        for n in 0..frames {
            if !self.playing {
                break;
            }
            if self.delay > 0 {
                self.delay -= 1;
                continue;
            }
            if let Some(f) = self.fade {
                self.gain += f.step;
                let done = if f.step >= 0.0 { self.gain >= f.target } else { self.gain <= f.target };
                if done {
                    self.gain = f.target;
                    self.fade = None;
                    if f.stop {
                        self.stop();
                        break;
                    }
                }
            }
            if self.decoder.is_none() || self.end_frame().is_some_and(|e| self.pos >= e as f64) {
                self.wrap();
                continue;
            }
            let i = self.pos.floor() as u64;
            let t = (self.pos - i as f64) as f32;
            let Some(a) = self.frame(i) else {
                self.wrap();
                continue;
            };
            let b = self.frame(i + 1).unwrap_or(a);
            self.pos += speed;
            if !audible {
                continue;
            }
            let g = self.gain * parent;
            let l = (a[0] + (b[0] - a[0]) * t) * g * pl;
            let r = (a[1] + (b[1] - a[1]) * t) * g * pr;
            out[2 * n] += l;
            out[2 * n + 1] += r;
            self.peak[0] = self.peak[0].max((l.abs() * 32767.0) as i64);
            self.peak[1] = self.peak[1].max((r.abs() * 32767.0) as i64);
        }
    }
}

/// All SOUND layers.
#[derive(Debug, Default)]
pub struct Mixer {
    /// Layers by ID.
    pub layers: BTreeMap<Vec<u8>, SoundLayer>,
    /// Parent gains of the layers during [`Mixer::render`].
    gains: Vec<f32>,
}

/// The parameters of one SOUND call, in the engine's terms.
#[derive(Debug, Default, Clone)]
pub struct SoundCall {
    /// FILE with the file's bytes (None: not found).
    pub file: Option<(Vec<u8>, Option<Vec<u8>>)>,
    pub enabled: Option<bool>,
    pub play: Option<bool>,
    pub pause: Option<bool>,
    pub fade: Option<i64>,
    pub start: Option<i64>,
    pub restart: Option<i64>,
    pub end: Option<i64>,
    pub loop_count: Option<i64>,
    pub volume: Option<i64>,
    pub pan: Option<i64>,
    pub pitch: Option<i64>,
}

impl Mixer {
    /// Applies a SOUND call to layer `id`, creating it.
    pub fn sound(&mut self, id: &[u8], c: SoundCall) {
        let l = self.layers.entry(id.to_vec()).or_default();
        let new_file = c.file.is_some();
        if let Some((name, data)) = c.file {
            l.load(name, data);
        }
        if let Some(e) = c.enabled {
            l.enabled = e;
        }
        if let Some(n) = c.loop_count {
            l.loop_count = n.max(0);
        }
        if let Some(n) = c.start {
            l.start_ms = n;
        }
        if let Some(n) = c.restart {
            l.restart_ms = n;
        }
        if let Some(n) = c.end {
            l.end_ms = n;
        }
        if let Some(n) = c.pan {
            l.pan = n.clamp(0, 512);
        }
        if let Some(n) = c.pitch {
            l.pitch = n.clamp(0, 512);
        }
        let fade = c.fade.unwrap_or(0);
        if let Some(v) = c.volume {
            l.volume = v.clamp(0, 256);
        }
        match c.play {
            Some(true) if new_file || !l.playing => l.play(fade),
            Some(false) if l.playing && fade > 0 => l.start_fade(0.0, fade, true),
            Some(false) => l.stop(),
            _ => {
                if c.volume.is_some() {
                    if fade > 0 && l.playing {
                        l.start_fade(l.target_gain(), fade, false);
                    } else {
                        l.gain = l.target_gain();
                        l.fade = None;
                    }
                }
            }
        }
        if let Some(p) = c.pause {
            l.paused = p;
        }
    }

    /// SOUNDEND: deletes a layer (all of them for None).
    pub fn remove(&mut self, id: Option<&[u8]>) {
        match id {
            Some(id) => {
                self.layers.remove(id);
            }
            None => self.layers.clear(),
        }
    }

    /// Gain from the layers whose IDs are `/`-ended prefixes of `id`
    /// (0 when one of them is disabled).
    fn parent_gain(&self, id: &[u8]) -> f32 {
        let mut g = 1.0;
        for (i, &b) in id.iter().enumerate() {
            if b != b'/' || i + 1 == id.len() {
                continue;
            }
            if let Some(p) = self.layers.get(&id[..=i]) {
                if !p.enabled {
                    return 0.0;
                }
                g *= p.target_gain_now();
            }
        }
        g
    }

    /// Mixes the next `out.len() / 2` stereo frames at [`OUTPUT_RATE`]
    /// into `out` (cleared first) and advances every layer.
    pub fn render(&mut self, out: &mut [f32]) {
        out.fill(0.0);
        // The parent gains first, into a buffer kept between calls, then
        // the layers in the same (key) order: no allocation per call.
        let mut gains = std::mem::take(&mut self.gains);
        gains.clear();
        for (id, l) in &self.layers {
            gains.push(if l.playing { self.parent_gain(id) } else { 0.0 });
        }
        for (l, &parent) in self.layers.values_mut().zip(&gains) {
            if l.playing {
                l.mix(out, parent);
            }
        }
        self.gains = gains;
        for s in out.iter_mut() {
            *s = s.clamp(-1.0, 1.0);
        }
    }

    /// Whether anything is playing.
    pub fn busy(&self) -> bool {
        self.layers.values().any(|l| l.playing)
    }
}

impl SoundLayer {
    /// The gain this layer passes to the layers below it.
    fn target_gain_now(&self) -> f32 {
        if self.playing || self.fade.is_some() {
            self.gain
        } else {
            self.target_gain()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wav(frames: usize, rate: u32) -> Vec<u8> {
        let mut d = Vec::new();
        let data_len = (frames * 2) as u32;
        d.extend_from_slice(b"RIFF");
        d.extend_from_slice(&(36 + data_len).to_le_bytes());
        d.extend_from_slice(b"WAVEfmt ");
        d.extend_from_slice(&16u32.to_le_bytes());
        d.extend_from_slice(&1u16.to_le_bytes());
        d.extend_from_slice(&1u16.to_le_bytes());
        d.extend_from_slice(&rate.to_le_bytes());
        d.extend_from_slice(&(rate * 2).to_le_bytes());
        d.extend_from_slice(&2u16.to_le_bytes());
        d.extend_from_slice(&16u16.to_le_bytes());
        d.extend_from_slice(b"data");
        d.extend_from_slice(&data_len.to_le_bytes());
        for _ in 0..frames {
            d.extend_from_slice(&16384i16.to_le_bytes());
        }
        d
    }

    fn call(file: Option<Vec<u8>>) -> SoundCall {
        SoundCall { file: file.map(|d| (b"x".to_vec(), Some(d))), play: Some(true), ..SoundCall::default() }
    }

    #[test]
    fn plays_once_then_stops() {
        let mut m = Mixer::default();
        m.sound(b"SE", call(Some(wav(4410, OUTPUT_RATE))));
        assert_eq!(m.layers[&b"SE".to_vec()].total_ms(), 100);
        let mut out = vec![0.0; 2 * 2205];
        m.render(&mut out);
        assert!(m.busy());
        assert!((out[0] - 0.5).abs() < 1e-3 && (out[1] - 0.5).abs() < 1e-3);
        assert_eq!(m.layers[&b"SE".to_vec()].time_ms(), 50);
        m.render(&mut out);
        m.render(&mut out);
        assert!(!m.busy());
    }

    #[test]
    fn loops_volume_and_parent() {
        let mut m = Mixer::default();
        m.sound(b":/S0/", SoundCall { volume: Some(128), ..SoundCall::default() });
        let mut c = call(Some(wav(441, OUTPUT_RATE)));
        c.loop_count = Some(0);
        c.volume = Some(128);
        m.sound(b":/S0/BGM", c);
        let mut out = vec![0.0; 2 * 4410];
        m.render(&mut out);
        let l = &m.layers[&b":/S0/BGM".to_vec()];
        assert!(l.playing && l.loops_done >= 9);
        // 0.5 * 0.5 (own) * 0.5 (parent).
        assert!((out[2 * 4000] - 0.125).abs() < 1e-3);
        m.sound(b":/S0/", SoundCall { enabled: Some(false), ..SoundCall::default() });
        m.render(&mut out);
        assert!(out.iter().all(|&s| s == 0.0));
    }

    #[test]
    fn fade_out_stops_and_delay() {
        let mut m = Mixer::default();
        let mut c = call(Some(wav(44100, OUTPUT_RATE)));
        c.start = Some(-100);
        m.sound(b"V", c);
        let mut out = vec![0.0; 2 * 4410];
        m.render(&mut out);
        assert_eq!(out[2 * 4000], 0.0);
        assert_eq!(m.layers[&b"V".to_vec()].time_ms(), 0);
        m.sound(b"V", SoundCall { play: Some(false), fade: Some(50), ..SoundCall::default() });
        m.render(&mut out);
        assert!(!m.busy());
    }

    #[test]
    fn missing_file_ends_at_once() {
        let mut m = Mixer::default();
        m.sound(b"VO", SoundCall { file: Some((b"vo/x".to_vec(), None)), play: Some(true), ..SoundCall::default() });
        let mut out = vec![0.0; 64];
        m.render(&mut out);
        assert!(!m.busy());
    }

    fn sine_ogg() -> Vec<u8> {
        // 0.5 s of 440 Hz, stereo, 22050 Hz (generated with ffmpeg).
        std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/sine440.ogg")).unwrap()
    }

    #[test]
    fn ogg_decodes_and_seeks_exactly() {
        let (dec, codec) = open_decoder(sine_ogg());
        let mut dec = dec.unwrap();
        assert_eq!((codec, dec.channels(), dec.rate()), (1, 2, 22050));
        assert_eq!(dec.total_frames(), Some(11025));
        let mut all = Vec::new();
        while dec.decode(&mut all) {}
        assert_eq!(all.len() / 2, 11025);
        assert!(all.iter().map(|s| s.unsigned_abs()).max().unwrap() > 1000);
        assert_eq!(dec.seek(5000), 5000);
        let mut part = Vec::new();
        while dec.decode(&mut part) {}
        assert_eq!(part[..], all[5000 * 2..]);
    }

    #[test]
    fn ogg_layer_resamples_and_loops() {
        let mut m = Mixer::default();
        let mut c = SoundCall { file: Some((b"bgm".to_vec(), Some(sine_ogg()))), play: Some(true), ..SoundCall::default() };
        c.loop_count = Some(2);
        c.restart = Some(250);
        m.sound(b"BGM", c);
        assert_eq!(m.layers[&b"BGM".to_vec()].total_ms(), 500);
        // 0.5 s, then 0.25 s from RESTART: 0.75 s at the output rate.
        let mut out = vec![0.0; 2 * (OUTPUT_RATE as usize * 74 / 100)];
        m.render(&mut out);
        assert!(m.busy());
        assert_eq!(m.layers[&b"BGM".to_vec()].loops_done, 1);
        let mut out = vec![0.0; 2 * (OUTPUT_RATE as usize * 2 / 100)];
        m.render(&mut out);
        assert!(!m.busy());
    }
}
