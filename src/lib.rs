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

#[derive(Clone, Copy, Default)]
struct Symbol {
    value: u8,
    magnitude: f32,
    noise: f32,
}

#[derive(Clone, Copy, Default)]
struct Frame {
    active: bool,
    data: [Symbol; FRAME_LEN],
    data_pos: usize,
}

pub struct Transceiver {
    sample_rate: u32,
    on_received: Box<FnMut([u8; PAYLOAD_LEN])>,
    on_transmit: Box<FnMut([f32; FRAME_LEN])>,
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
    fn calc_freq(symbol: u8) -> f32 {
        BASE_FREQ * SEMITONE.powi(symbol as i32)
    }

    pub fn new(sample_rate: u32, on_received: Box<FnMut([u8; PAYLOAD_LEN])>, on_transmit: Box<FnMut([f32; FRAME_LEN])>) -> Self {
        let sample_buffer_len = (sample_rate as f32) * BEEP_LEN;
        let sample_buffer = vec![0.; sample_buffer_len.round() as usize];
        let mut window_weights = vec![0f32; sample_buffer.len()];
        for i in 0..window_weights.len() {
            // Hamming window
            window_weights[i] = 0.54 - 0.46 * (2.0 * PI * (i as f32) / (window_weights.len() as f32 - 1.0)).cos();
        }
        let window_weights = window_weights;
        let mut goertzel_filters: [goertzel::Parameters; SYMBOL_COUNT] = unsafe {std::mem::uninitialized()};
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
        for (i, symbol) in encoded_data.iter().enumerate() {
            frequencies[i] = Self::calc_freq(*symbol);
        }
        (self.on_transmit)(frequencies);
    }

    pub fn push_sample(&mut self, sample: f32) {
        self.sample_buffer[self.sample_buffer_pos] = sample;
        self.sample_buffer_pos = (self.sample_buffer_pos + 1) % self.sample_buffer.len();
        self.remaining_samples -= 1.;
        if self.remaining_samples > 0. {
            return;
        }
        self.remaining_samples += (self.sample_rate as f32) * BEEP_LEN / (MEASUREMENTS_PER_SYMBOL as f32);
        let mut sample_window = vec![0f32; self.sample_buffer.len()];
        for i in self.sample_buffer_pos..self.sample_buffer.len() {
            let j = i - self.sample_buffer_pos;
            sample_window[j] = self.sample_buffer[i] * self.window_weights[j];
        }
        for i in 0..self.sample_buffer_pos {
            let j = i + (self.sample_buffer.len() - self.sample_buffer_pos);
            sample_window[j] = self.sample_buffer[i] * self.window_weights[j];
        }
        let mut best_symbol: Symbol = Default::default();
        for (i, goertzel_filter) in self.goertzel_filters.iter().enumerate() {
            let magnitude = goertzel_filter.mag(&sample_window);
            if magnitude > best_symbol.magnitude {
                best_symbol = Symbol {
                    value: i as u8,
                    magnitude: magnitude,
                    noise: best_symbol.magnitude
                };
            } else if magnitude > best_symbol.noise {
                best_symbol.noise = magnitude;
            }
        }
        let completed_frame = self.frames[self.frames_pos];
        self.frames[self.frames_pos] = Default::default();
        self.frames[self.frames_pos].active = true;
        for i in 0..FRAME_LEN {
            let frame = &mut self.frames[(self.frames_pos + i * MEASUREMENTS_PER_SYMBOL) % self.frames.len()];
            if frame.active {
                frame.data[frame.data_pos] = best_symbol;
                frame.data_pos += 1;
            }
        }
        if completed_frame.active {
            let mut raw_data: [u8; FRAME_LEN] = Default::default();
            for (i, symbol) in completed_frame.data.iter().enumerate() {
                raw_data[i] = symbol.value;
            }
            if let Ok(corrected_data) = self.rs_decoder.correct(&mut raw_data, None) {
                let corrected_data = *corrected_data;
                let start_symbols_ok = corrected_data[..START_SYMBOLS_LEN] == START_SYMBOLS;
                if start_symbols_ok {
                    let mut payload: [u8; PAYLOAD_LEN] = Default::default();
                    for (i, &c) in corrected_data.iter().skip(START_SYMBOLS_LEN).take(PAYLOAD_LEN).enumerate() {
                        payload[i] = c;
                    }
                    let mut correct_symbols = 0;
                    for (i, &c) in corrected_data.iter().enumerate() {
                        if raw_data[i] == c {
                            correct_symbols += 1;
                        }
                    }
                    let mut signal_quality = 0.;
                    for symbol in completed_frame.data.iter() {
                        signal_quality += symbol.magnitude / symbol.noise;
                    }
                    let frame_quality = (correct_symbols, signal_quality);
                    if !self.active_frame || self.active_frame_quality < frame_quality {
                        self.active_frame_payload = payload;
                        self.active_frame_quality = frame_quality;
                        if !self.active_frame {
                            self.active_frame = true;
                            self.active_frame_age = 0;
                            // Skip following frames
                            for i in MEASUREMENTS_PER_SYMBOL..self.frames.len() {
                                let frame_pos = (self.frames_pos + i) % self.frames.len();
                                self.frames[frame_pos].active = false;
                            }
                        }
                    }
                }
            }
        }
        if self.active_frame {
            self.active_frame_age += 1;
            if self.active_frame_age == MEASUREMENTS_PER_SYMBOL {
                self.active_frame = false;
                (self.on_received)(self.active_frame_payload);
            }
        }
        self.frames_pos = (self.frames_pos + 1) % self.frames.len();
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
