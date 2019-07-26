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

#[derive(Clone)]
#[derive(Default)]
struct Symbol {
    value: u8,
    magnitude: f32,
    noise: f32,
}

#[derive(Default)]
struct Frame {
    start_measurement: u64,
    data: [Symbol; FRAME_LEN],
    data_pos: usize
}

pub struct Transceiver {
    sample_rate: u32,
    on_received: Box<FnMut([u8; PAYLOAD_LEN])>,
    on_transmit: Box<FnMut([f32; FRAME_LEN])>,
    measurements_count: u64,
    sample_buffer: Vec<f32>,
    sample_buffer_pos: usize,
    remaining_samples: f32,
    window_weights: Vec<f32>,
    rs_decoder: reed_solomon::Decoder<GF>,
    rs_encoder: reed_solomon::Encoder<GF>,
    goertzel_filters: [goertzel::Parameters; SYMBOL_COUNT],
    partial_frames: Vec<Frame>,
    frames: Vec<Frame>,
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
            measurements_count: 0,
            remaining_samples: sample_buffer_len as f32,
            sample_buffer: sample_buffer,
            sample_buffer_pos: 0,
            window_weights: window_weights,
            rs_decoder: reed_solomon::Decoder::new(ECC_LEN),
            rs_encoder: reed_solomon::Encoder::new(ECC_LEN),
            goertzel_filters: goertzel_filters,
            partial_frames: vec![],
            frames: vec![]
        }
    }

    pub fn send(&mut self, payload: &[u8; PAYLOAD_LEN]) {
        let encoded_data = self.rs_encoder.encode(payload);
        let mut frequencies: [f32; FRAME_LEN] = Default::default();
        for (i, symbol) in START_SYMBOLS.iter().enumerate() {
            frequencies[i] = Self::calc_freq(*symbol);
        }
        for (i, symbol) in encoded_data.iter().enumerate() {
            frequencies[i + START_SYMBOLS.len()] = Self::calc_freq(*symbol);
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
        self.partial_frames.push(Frame {
            start_measurement: self.measurements_count,
            data: Default::default(),
            data_pos: 0
        });
        for frame in self.partial_frames.iter_mut() {
            if (self.measurements_count - frame.start_measurement) % (MEASUREMENTS_PER_SYMBOL as u64) == 0 {
                frame.data[frame.data_pos] = best_symbol.clone();
                frame.data_pos += 1;
            }
        }
        let mut i = 0;
        while i < self.partial_frames.len() {
            let mut remove_frame = false;
            {
                let frame = &self.partial_frames[i];
                if frame.data_pos == START_SYMBOLS_LEN {
                    for (j, start_symbol) in START_SYMBOLS.iter().enumerate() {
                        if frame.data[j].value != *start_symbol {
                            remove_frame = true;
                            break;
                        }
                    }
                }
                if frame.data_pos == FRAME_LEN {
                    remove_frame = true;
                }
            }
            if remove_frame {
                let frame = self.partial_frames.remove(i);
                if frame.data_pos == FRAME_LEN {
                    self.frames.push(frame);
                }
                continue;
            }
            i += 1;
        }
        self.measurements_count += 1;
        if self.frames.len() == 0 ||
                self.frames[0].start_measurement +
                (FRAME_LEN as u64) * (MEASUREMENTS_PER_SYMBOL as u64) > self.measurements_count {
            return;
        }
        {
            let mut best_frame_quality = 0.0;
            let mut best_frame = &Default::default();
            for frame in self.frames.iter() {
                let mut frame_quality = 0f32;
                for symbol in frame.data.iter() {
                    frame_quality += symbol.magnitude / symbol.noise;
                }
                if best_frame_quality < frame_quality {
                    best_frame = frame;
                    best_frame_quality = frame_quality;
                }
            }
            let mut raw_data: [u8; PAYLOAD_LEN + ECC_LEN] = Default::default();
            for (i, symbol) in best_frame.data.iter().skip(START_SYMBOLS_LEN).enumerate() {
                raw_data[i] = symbol.value;
            }
            if let Ok(corrected_data) = self.rs_decoder.correct(&mut raw_data, None) {
                let mut payload_data: [u8; PAYLOAD_LEN] = Default::default();
                for (i, c) in (*corrected_data).iter().take(PAYLOAD_LEN).enumerate() {
                    payload_data[i] = *c;
                }
                (self.on_received)(payload_data);
            }
        }
        self.frames.clear();
    }
}
