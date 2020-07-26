extern crate goertzel;
extern crate reed_solomon;

use std::f32;
use std::f32::consts::PI;

const SEMITONE: f32 = 1.05946311;
const BASE_FREQ: f32 = 1760.0;
pub const BEEP_TIME: f32 = 0.0872;
pub const ATTACK_TIME: f32 = 0.012;
pub const RELEASE_TIME: f32 = 0.012;
pub const SYMBOL_BITS: usize = 5;
pub const SYMBOL_COUNT: usize = 32; // 2usize.pow(SYMBOL_BITS as u32)
pub const SYMBOL_MNEMONICS: &str = "0123456789abcdefghijklmnopqrstuv"; // .len() == SYMBOL_COUNT
type GF = reed_solomon::GF2_5; // SYMBOL_BITS
const START_SYMBOLS_LEN: usize = 2;
const START_SYMBOLS: [u8; START_SYMBOLS_LEN] = [17, 19];
pub const PAYLOAD_LEN: usize = 10;
const ECC_LEN: usize = 8;
pub const PACKET_LEN: usize = START_SYMBOLS_LEN + PAYLOAD_LEN + ECC_LEN;
const MEASUREMENTS_PER_SYMBOL: usize = 10;

macro_rules! mod_short {
    ($i:expr, $len:expr) => {{
        debug_assert!($i < 2 * $len);
        if $i < $len {
            $i
        } else {
            $i - $len
        }
    }};
}

#[derive(Clone, Copy, Default)]
struct Packet {
    data: [u8; PACKET_LEN],
    data_pos: usize,
    signal_quality: f32,
}

pub struct Transceiver {
    sample_rate: u32,
    measurement_count: u64,
    sample_buffer: Vec<f32>,
    sample_buffer_pos: usize,
    remaining_samples: f32,
    window_weights: Vec<f32>,
    rs_decoder: reed_solomon::Decoder<GF>,
    rs_encoder: reed_solomon::Encoder<GF>,
    goertzel_filters: [goertzel::Parameters; SYMBOL_COUNT],
    packets: [Packet; MEASUREMENTS_PER_SYMBOL * PACKET_LEN],
    packets_pos: usize,
    valid_packet: bool,
    valid_packet_payload: [u8; PAYLOAD_LEN],
    valid_packet_quality: (usize, f32),
    first_valid_packet_age: usize,
}

impl Transceiver {
    #[inline]
    fn calc_freq(symbol: u8) -> f32 {
        BASE_FREQ * SEMITONE.powi(symbol as i32)
    }

    pub fn new(sample_rate: u32) -> Self {
        let min_sampling_rate = (Self::calc_freq((SYMBOL_COUNT - 1) as u8) * 2.0).round() as u32;
        if sample_rate < min_sampling_rate {
            panic!(
                "sample rate is too low: must be atleast {} but is {}",
                min_sampling_rate, sample_rate
            );
        }
        let sample_buffer_len = ((sample_rate as f32) * BEEP_TIME).round() as usize;
        assert!(sample_buffer_len > 0);
        let window_weights = {
            let mut window_weights = vec![0f32; sample_buffer_len];
            for i in 0..window_weights.len() {
                // Hamming window
                window_weights[i] = 0.54
                    - 0.46 * (2.0 * PI * (i as f32) / (window_weights.len() as f32 - 1.0)).cos();
            }
            window_weights
        };
        let mut goertzel_filters: [goertzel::Parameters; SYMBOL_COUNT] =
            unsafe { std::mem::MaybeUninit::uninit().assume_init() };
        for i in 0..goertzel_filters.len() {
            goertzel_filters[i] =
                goertzel::Parameters::new(Self::calc_freq(i as u8), sample_rate, sample_buffer_len);
        }
        Self {
            sample_rate: sample_rate,
            measurement_count: 0,
            remaining_samples: (sample_rate as f32) * BEEP_TIME / (MEASUREMENTS_PER_SYMBOL as f32),
            sample_buffer: vec![0.; sample_buffer_len],
            sample_buffer_pos: 0,
            window_weights: window_weights,
            rs_decoder: reed_solomon::Decoder::new(ECC_LEN),
            rs_encoder: reed_solomon::Encoder::new(ECC_LEN),
            goertzel_filters: goertzel_filters,
            packets: [Default::default(); MEASUREMENTS_PER_SYMBOL * PACKET_LEN],
            packets_pos: 0,
            valid_packet: false,
            valid_packet_payload: Default::default(),
            valid_packet_quality: Default::default(),
            first_valid_packet_age: Default::default(),
        }
    }

    // Get the beep frequencies for the packet containing `payload`.
    // The timmings `BEEP_TIME`, `ATTACK_TIME` and `RELEASE_TIME` should be used for the beeps.
    // See method `generate_beep`
    pub fn send(&self, payload: &[u8; PAYLOAD_LEN]) -> [f32; PACKET_LEN] {
        if let Some(v) = payload.iter().find(|v| (**v as usize) > SYMBOL_COUNT) {
            panic!(
                "symbol out of bounds: must be smaller than {} but is {}",
                SYMBOL_COUNT, v
            );
        }
        let mut data: [u8; START_SYMBOLS_LEN + PAYLOAD_LEN] = Default::default();
        data[..START_SYMBOLS_LEN].copy_from_slice(&START_SYMBOLS);
        data[START_SYMBOLS_LEN..].copy_from_slice(payload);
        let encoded_data = self.rs_encoder.encode(&data);
        let mut frequencies: [f32; PACKET_LEN] = Default::default();
        debug_assert_eq!(encoded_data.len(), frequencies.len());
        for (i, symbol) in encoded_data.iter().enumerate() {
            frequencies[i] = Self::calc_freq(*symbol);
        }
        frequencies
    }

    // Commit an audio sample to the receiver
    pub fn push_sample(&mut self, sample: f32) -> Option<[u8; PAYLOAD_LEN]> {
        // Push new sample to ring buffer
        self.sample_buffer[self.sample_buffer_pos] = sample;
        self.sample_buffer_pos = mod_short!(self.sample_buffer_pos + 1, self.sample_buffer.len());
        self.remaining_samples -= 1.;
        if self.remaining_samples > 0. {
            return None;
        }
        self.remaining_samples +=
            (self.sample_rate as f32) * BEEP_TIME / (MEASUREMENTS_PER_SYMBOL as f32);
        self.measurement_count += 1;

        // Decode symbol
        let mut goertzel_partials: [goertzel::Partial; SYMBOL_COUNT] =
            unsafe { std::mem::MaybeUninit::uninit().assume_init() };
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
        for (i, &goertzel_partial) in goertzel_partials.iter().enumerate() {
            let magnitude = goertzel_partial.finish_fast();
            if magnitude > next_symbol_magnitude {
                next_symbol = i as u8;
                next_symbol_noise = next_symbol_magnitude;
                next_symbol_magnitude = magnitude;
            } else if magnitude > next_symbol_noise {
                next_symbol_noise = magnitude;
            }
        }
        let next_symbol_snr = next_symbol_magnitude / next_symbol_noise;

        // Reset packet
        self.packets[self.packets_pos] = Default::default();
        for i in 0..PACKET_LEN {
            let packet = &mut self.packets[mod_short!(
                self.packets_pos + i * MEASUREMENTS_PER_SYMBOL,
                self.packets.len()
            )];
            packet.data[packet.data_pos] = next_symbol;
            packet.data_pos += 1;
            packet.signal_quality += next_symbol_snr; // average SNR of symbols
        }
        let completed_packet_pos = mod_short!(
            self.packets_pos + MEASUREMENTS_PER_SYMBOL,
            self.packets.len()
        );
        let completed_packet = &self.packets[completed_packet_pos];
        self.packets_pos = mod_short!(self.packets_pos + 1, self.packets.len());

        // Process the completed packet unless we are just starting up or the packet was dropped
        if completed_packet.data_pos == PACKET_LEN {
            if let Ok(corrected_data) = self.rs_decoder.correct(&completed_packet.data, None) {
                let corrected_data = corrected_data.data();
                let start_symbols_ok = corrected_data[..START_SYMBOLS_LEN] == START_SYMBOLS;
                if start_symbols_ok {
                    let mut correct_symbols = 0;
                    for (i, &c) in corrected_data.iter().enumerate() {
                        if completed_packet.data[i] == c {
                            correct_symbols += 1;
                        }
                    }
                    let packet_quality = (correct_symbols, completed_packet.signal_quality);
                    // Replace old valid packet if new valid packet is of higher quality
                    if !self.valid_packet || self.valid_packet_quality < packet_quality {
                        self.valid_packet_payload
                            .copy_from_slice(&corrected_data[START_SYMBOLS_LEN..][..PAYLOAD_LEN]);
                        self.valid_packet_quality = packet_quality;
                    }
                    if !self.valid_packet {
                        self.valid_packet = true;
                        self.first_valid_packet_age = 0;
                        // Drop all old partial packets that are not capturing the same
                        // packet overlapping. The remaining packets are only missing
                        // the last symbol.
                        for i in MEASUREMENTS_PER_SYMBOL..self.packets.len() {
                            let packet_pos =
                                mod_short!(completed_packet_pos + i, self.packets.len());
                            self.packets[packet_pos].data_pos = 0;
                        }
                        #[cfg(debug_assertions)]
                        for i in 1..MEASUREMENTS_PER_SYMBOL {
                            let packet_pos =
                                mod_short!(completed_packet_pos + i, self.packets.len());
                            assert!(self.packets[packet_pos].data_pos == PACKET_LEN - 1);
                        }
                    }
                }
            }
        }

        // Delay returning the valid packet until all overlapping measurements that can capture
        // the same packet are complete, because we might receive a valid packet of better quality
        if self.valid_packet {
            self.first_valid_packet_age += 1;
            if self.first_valid_packet_age == MEASUREMENTS_PER_SYMBOL {
                self.valid_packet = false;
                return Some(self.valid_packet_payload);
            }
        }
        return None;
    }

    // Generate audio data for a beep with the specified `frequency`
    // The timings from the constants `BEEP_TIME`, `ATTACK_TIME`, `RELEASE_TIME` and
    // the `sampling_rate` of the Transceiver are used.
    pub fn generate_beep(&self, frequency: f32) -> Vec<f32> {
        let samples_len = (BEEP_TIME * (self.sample_rate as f32)).round() as usize;
        (0..samples_len)
            .map(|i| {
                let t = BEEP_TIME / (samples_len as f32) * (i as f32);
                let window = (t / ATTACK_TIME).min(1.) * ((BEEP_TIME - t) / RELEASE_TIME).min(1.);
                (t * frequency * 2. * PI).sin() * window
            })
            .collect()
    }
}

#[cfg(feature = "test-utils")]
pub mod test_utils {
    use super::*;
    use rand::prelude::*;

    // Generate deterministic random payload based on `seed`
    pub fn rand_payload(seed: usize) -> [u8; PAYLOAD_LEN] {
        let mut rng = SmallRng::seed_from_u64(seed as u64);
        let mut payload = [0u8; PAYLOAD_LEN];
        for i in 0..payload.len() {
            payload[i] = rng.gen_range(0, SYMBOL_COUNT) as u8;
        }
        payload
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use test_utils::*;

    // Test coherency of global constants
    #[test]
    fn test_constants() {
        assert_eq!(2usize.pow(SYMBOL_BITS as u32), SYMBOL_COUNT);
        assert_eq!(SYMBOL_MNEMONICS.len(), SYMBOL_COUNT);
        assert_eq!(format!("{:?}", GF {}), format!("GF2_{}", SYMBOL_BITS));
    }

    // Test sending and subsequent receiving of multiple successive packets
    #[test]
    fn test_send_and_receive() {
        const SAMPLE_RATE: u32 = 44100;
        const SEND_COUNT: usize = 5;
        let mut transceiver = Transceiver::new(SAMPLE_RATE);
        let mut received_count = 0;
        let mut push_sample_and_receive = |transceiver: &mut Transceiver, sample| {
            if let Some(payload) = transceiver.push_sample(sample) {
                assert_eq!(payload, rand_payload(received_count));
                received_count += 1;
            }
        };
        for i in 0..SEND_COUNT {
            for &frequency in transceiver.send(&rand_payload(i)).iter() {
                for &sample in transceiver.generate_beep(frequency).iter() {
                    push_sample_and_receive(&mut transceiver, sample);
                }
            }
        }
        for _ in 0..((SAMPLE_RATE as f32) * BEEP_TIME).ceil() as u32 {
            push_sample_and_receive(&mut transceiver, 0.);
        }
        assert_eq!(received_count, SEND_COUNT);
    }
}
