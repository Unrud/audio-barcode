extern crate reed_solomon;
extern crate goertzel;

use std::f32::consts::PI;
use std::f32;

const SEMITONE: f32 = 1.05946311;
const BASE_FREQ: f32 = 1760.0;
pub const BEEP_LEN: f32 = 0.0872;
pub const ATTACK_LEN: f32 = 0.012;
pub const RELEASE_LEN: f32 = 0.012;
type GF = reed_solomon::GF2_5;
const SYMBOL_COUNT: usize = 32;  //2usize.pow(5)
const START_SYMBOLS: [u8; 2] = [17, 19];
const START_SYMBOLS_LEN: usize = 2;
pub const PAYLOAD_LEN: usize = 10;
const ECC_LEN: usize = 8;
pub const FRAME_LEN: usize = START_SYMBOLS_LEN + PAYLOAD_LEN + ECC_LEN;
const MEASUREMENTS_PER_SYMBOL: usize = 10;
pub const SYMBOL_MNEMONICS: &str = "0123456789abcdefghijklmnopqrstuv";

macro_rules! mod_short {
    ($i:expr, $len:expr) => ({
        debug_assert!($i < 2 * $len);
        if $i < $len {
            $i
        } else {
            $i - $len
        }
    })
}

#[derive(Clone, Copy, Default)]
struct Frame {
    active: bool,
    data: [u8; FRAME_LEN],
    data_pos: usize,
    signal_quality: f32,
}

pub struct Transceiver {
    sample_rate: u32,
    on_received: Box<dyn FnMut([u8; PAYLOAD_LEN])>,
    on_transmit: Box<dyn FnMut([f32; FRAME_LEN])>,
    sample_buffer: Vec<f32>,
    sample_buffer_pos: usize,
    remaining_samples: f32,
    window_weights: Vec<f32>,
    rs_decoder: reed_solomon::Decoder<GF>,
    rs_encoder: reed_solomon::Encoder<GF>,
    goertzel_filters: [goertzel::Parameters; SYMBOL_COUNT],
    frames: [Frame; MEASUREMENTS_PER_SYMBOL * FRAME_LEN],
    frames_pos: usize,
    active_frame: bool,
    active_frame_payload: [u8; PAYLOAD_LEN],
    active_frame_quality: (usize, f32),
    active_frame_age: usize,
}

impl Transceiver {
    #[inline]
    fn calc_freq(symbol: u8) -> f32 {
        BASE_FREQ * SEMITONE.powi(symbol as i32)
    }

    pub fn new(sample_rate: u32, on_received: Box<dyn FnMut([u8; PAYLOAD_LEN])>, on_transmit: Box<dyn FnMut([f32; FRAME_LEN])>) -> Self {
        let sample_buffer_len = (sample_rate as f32) * BEEP_LEN;
        let sample_buffer = vec![0.; sample_buffer_len.round() as usize];
        let mut window_weights = vec![0f32; sample_buffer.len()];
        for i in 0..window_weights.len() {
            // Hamming window
            window_weights[i] = 0.54 - 0.46 * (2.0 * PI * (i as f32) / (window_weights.len() as f32 - 1.0)).cos();
        }
        let window_weights = window_weights;
        let mut goertzel_filters: [goertzel::Parameters; SYMBOL_COUNT] = unsafe {std::mem::MaybeUninit::uninit().assume_init()};
        for i in 0..goertzel_filters.len() {
            goertzel_filters[i] = goertzel::Parameters::new(Self::calc_freq(i as u8), sample_rate, sample_buffer.len());
        }
        return Transceiver {
            sample_rate: sample_rate,
            on_received: on_received,
            on_transmit: on_transmit,
            remaining_samples: sample_buffer_len as f32,
            sample_buffer: sample_buffer,
            sample_buffer_pos: 0,
            window_weights: window_weights,
            rs_decoder: reed_solomon::Decoder::new(ECC_LEN),
            rs_encoder: reed_solomon::Encoder::new(ECC_LEN),
            goertzel_filters: goertzel_filters,
            frames: [Default::default(); MEASUREMENTS_PER_SYMBOL * FRAME_LEN],
            frames_pos: 0,
            active_frame: false,
            active_frame_payload: Default::default(),
            active_frame_quality: Default::default(),
            active_frame_age: Default::default(),
        }
    }

    pub fn send(&mut self, payload: &[u8; PAYLOAD_LEN]) {
        let mut data: [u8; START_SYMBOLS_LEN + PAYLOAD_LEN] = Default::default();
        data[..START_SYMBOLS_LEN].copy_from_slice(&START_SYMBOLS);
        data[START_SYMBOLS_LEN..].copy_from_slice(payload);
        let encoded_data = self.rs_encoder.encode(&data);
        let mut frequencies: [f32; FRAME_LEN] = Default::default();
        debug_assert_eq!(encoded_data.len(), frequencies.len());
        for (i, symbol) in encoded_data.iter().enumerate() {
            frequencies[i] = Self::calc_freq(*symbol);
        }
        (self.on_transmit)(frequencies);
    }

    pub fn push_sample(&mut self, sample: f32) {
        // Push new sample to ring buffer
        self.sample_buffer[self.sample_buffer_pos] = sample;
        self.sample_buffer_pos = mod_short!(self.sample_buffer_pos + 1, self.sample_buffer.len());
        self.remaining_samples -= 1.;
        if self.remaining_samples > 0. {
            return;
        }
        self.remaining_samples += (self.sample_rate as f32) * BEEP_LEN / (MEASUREMENTS_PER_SYMBOL as f32);

        // Decode symbol
        let mut goertzel_partials: [goertzel::Partial; SYMBOL_COUNT] = unsafe {std::mem::MaybeUninit::uninit().assume_init()};
        for (i, goertzel_filter) in self.goertzel_filters.iter().enumerate() {
            goertzel_partials[i] = goertzel_filter.start();
        }
        for i in 0..self.sample_buffer.len() {
            let j = mod_short!(self.sample_buffer_pos + i, self.sample_buffer.len());
            let window_sample = self.sample_buffer[j] * self.window_weights[i];
            for goertzel_partial in goertzel_partials.iter_mut() {
                goertzel_partial.push(window_sample);
            }
        }
        // Find symbol with highest magnitude
        let mut next_symbol = 0u8;
        let mut next_symbol_magnitude = 0.;
        let mut next_symbol_noise = 0.;
        for (i, &goertzel_partial) in goertzel_partials.into_iter().enumerate() {
            let magnitude = goertzel_partial.finish_fast();
            if magnitude > next_symbol_magnitude {
                next_symbol = i as u8;
                next_symbol_noise = next_symbol_magnitude;
                next_symbol_magnitude = magnitude;
            } else if magnitude > next_symbol_noise {
                next_symbol_noise = magnitude;
            }
        }

        // Add new symbol to partial frames
        let completed_frame = self.frames[self.frames_pos];
        self.frames[self.frames_pos] = Default::default();
        self.frames[self.frames_pos].active = true;
        let next_symbol_snr = next_symbol_magnitude / next_symbol_noise;
        for i in 0..FRAME_LEN {
            let frame_pos = mod_short!(self.frames_pos + i * MEASUREMENTS_PER_SYMBOL, self.frames.len());
            let frame = &mut self.frames[frame_pos];
            if frame.active {
                frame.data[frame.data_pos] = next_symbol;
                frame.data_pos += 1;
                frame.signal_quality += next_symbol_snr;
            }
        }
        self.frames_pos = mod_short!(self.frames_pos + 1, self.frames.len());

        // Replace active frame, if completed frame is higher quality
        if completed_frame.active {
            debug_assert_eq!(completed_frame.data_pos, completed_frame.data.len());
            if let Ok(corrected_data) = self.rs_decoder.correct(&completed_frame.data, None) {
                let corrected_data = corrected_data.data();
                let start_symbols_ok = corrected_data[..START_SYMBOLS_LEN] == START_SYMBOLS;
                if start_symbols_ok {
                    let mut correct_symbols = 0;
                    for (i, &c) in corrected_data.iter().enumerate() {
                        if completed_frame.data[i] == c {
                            correct_symbols += 1;
                        }
                    }
                    let frame_quality = (correct_symbols, completed_frame.signal_quality);
                    if !self.active_frame || self.active_frame_quality < frame_quality {
                        self.active_frame_payload.copy_from_slice(&corrected_data[START_SYMBOLS_LEN..][..PAYLOAD_LEN]);
                        self.active_frame_quality = frame_quality;
                        if !self.active_frame {
                            self.active_frame = true;
                            self.active_frame_age = 0;
                            // Skip all frames after complete symbol
                            for i in MEASUREMENTS_PER_SYMBOL..self.frames.len() {
                                self.frames[mod_short!(self.frames_pos + i - 1, self.frames.len())].active = false;
                            }
                        }
                    }
                }
            }
        }

        // Delay receiving of active frame until symbol is complete
        if self.active_frame {
            self.active_frame_age += 1;
            if self.active_frame_age == MEASUREMENTS_PER_SYMBOL {
                self.active_frame = false;
                (self.on_received)(self.active_frame_payload);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[test]
    fn test_receiver() {
        let mut wav_reader = hound::WavReader::open("testsamples/test.wav").unwrap();
        let mut expected_payload = [0u8; PAYLOAD_LEN];
        for (i, mnemonic) in "test".chars().enumerate() {
            expected_payload[i] = SYMBOL_MNEMONICS.find(mnemonic).unwrap() as u8;
        }
        let expected_payload = expected_payload;
        let received_payload = Arc::new(Mutex::new([0u8; PAYLOAD_LEN]));
        let received_count = Arc::new(Mutex::new(0));
        let on_received = {
            let received_payload = received_payload.clone();
            let received_count = received_count.clone();
            move |payload| {
                *received_payload.lock().unwrap() = payload;
                *received_count.lock().unwrap() += 1;
            }
        };
        let mut transceiver = Transceiver::new(
            wav_reader.spec().sample_rate,
            Box::new(on_received),
            Box::new(|_| panic!("should not be called")));
        for sample in wav_reader.samples::<f32>() {
            transceiver.push_sample(sample.unwrap());
        }
        assert_eq!(*received_count.lock().unwrap(), 1);
        assert_eq!(*received_payload.lock().unwrap(), expected_payload);
    }

    #[test]
    fn test_transmitter() {
        let mut payload = [0u8; PAYLOAD_LEN];
        for (i, mnemonic) in "test".chars().enumerate() {
            payload[i] = SYMBOL_MNEMONICS.find(mnemonic).unwrap() as u8;
        }
        let payload = payload;
        let transmit_frequencies = Arc::new(Mutex::new([0f32; FRAME_LEN]));
        let transmit_count = Arc::new(Mutex::new(0));
        let on_transmit = {
            let transmit_frequencies = transmit_frequencies.clone();
            let transmit_count = transmit_count.clone();
            move |frequencies| {
                *transmit_frequencies.lock().unwrap() = frequencies;
                *transmit_count.lock().unwrap() += 1;
            }
        };
        let mut transceiver = Transceiver::new(
            48000,
            Box::new(|_| panic!("should not be called")),
            Box::new(on_transmit));
        transceiver.send(&payload);
        assert_eq!(*transmit_count.lock().unwrap(), 1);
        for &frequency in transmit_frequencies.lock().unwrap().iter() {
            assert_ne!(frequency, 0.);
        }
    }
}
