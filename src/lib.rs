extern crate reed_solomon;
extern crate goertzel;

use std::f32::consts::PI;
use std::f32;

const SEMITONE: f32 = 1.05946311;
const BASE_FREQ: f32 = 1760.0;
pub const BEEP_TIME: f32 = 0.0872;
pub const ATTACK_TIME: f32 = 0.012;
pub const RELEASE_TIME: f32 = 0.012;
type GF = reed_solomon::GF2_5;
const SYMBOL_BITS: usize = 5;
const SYMBOL_COUNT: usize = 32;  //2usize.pow(SYMBOL_BITS)
const START_SYMBOLS_LEN: usize = 2;
const START_SYMBOLS: [u8; START_SYMBOLS_LEN] = [17, 19];
pub const PAYLOAD_LEN: usize = 10;
const ECC_LEN: usize = 8;
pub const PACKET_LEN: usize = START_SYMBOLS_LEN + PAYLOAD_LEN + ECC_LEN;
const MEASUREMENTS_PER_SYMBOL: usize = 10;
pub const SYMBOL_MNEMONICS: &str = "0123456789abcdefghijklmnopqrstuv";
pub const MAX_MESSAGE_LEN: usize = 255;
pub const TIME_BETWEEN_PACKETS: f32 = BEEP_TIME * 6.5;
const MAX_MEASUREMENTS_BETWEEN_PACKETS: usize = 13 * MEASUREMENTS_PER_SYMBOL;

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
struct Packet {
    active: bool,
    data: [u8; PACKET_LEN],
    data_pos: usize,
    signal_quality: f32,
}

pub struct Transceiver {
    sample_rate: u32,
    on_received: Box<dyn FnMut([u8; PAYLOAD_LEN])>,
    on_received_message: Box<dyn FnMut(Box<[u8]>)>,
    on_transmit: Box<dyn FnMut([f32; PACKET_LEN])>,
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
    active_packet: bool,
    active_packet_payload: [u8; PAYLOAD_LEN],
    active_packet_quality: (usize, f32),
    active_packet_age: usize,
    active_message: Vec<bool>,
    active_message_received_at_measurement: u64,
}

impl Transceiver {
    #[inline]
    fn calc_freq(symbol: u8) -> f32 {
        BASE_FREQ * SEMITONE.powi(symbol as i32)
    }

    pub fn new(sample_rate: u32, on_received: Box<dyn FnMut([u8; PAYLOAD_LEN])>, on_received_message: Box<dyn FnMut(Box<[u8]>)>, on_transmit: Box<dyn FnMut([f32; PACKET_LEN])>) -> Self {
        let sample_buffer_len = (sample_rate as f32) * BEEP_TIME;
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
            on_received_message: on_received_message,
            on_transmit: on_transmit,
            measurement_count: 0,
            remaining_samples: sample_buffer_len as f32,
            sample_buffer: sample_buffer,
            sample_buffer_pos: 0,
            window_weights: window_weights,
            rs_decoder: reed_solomon::Decoder::new(ECC_LEN),
            rs_encoder: reed_solomon::Encoder::new(ECC_LEN),
            goertzel_filters: goertzel_filters,
            packets: [Default::default(); MEASUREMENTS_PER_SYMBOL * PACKET_LEN],
            packets_pos: 0,
            active_packet: false,
            active_packet_payload: Default::default(),
            active_packet_quality: Default::default(),
            active_packet_age: Default::default(),
            active_message: Vec::new(),
            active_message_received_at_measurement: 0,
        }
    }

    pub fn send(&mut self, payload: &[u8; PAYLOAD_LEN]) {
        let mut data: [u8; START_SYMBOLS_LEN + PAYLOAD_LEN] = Default::default();
        data[..START_SYMBOLS_LEN].copy_from_slice(&START_SYMBOLS);
        data[START_SYMBOLS_LEN..].copy_from_slice(payload);
        let encoded_data = self.rs_encoder.encode(&data);
        let mut frequencies: [f32; PACKET_LEN] = Default::default();
        debug_assert_eq!(encoded_data.len(), frequencies.len());
        for (i, symbol) in encoded_data.iter().enumerate() {
            frequencies[i] = Self::calc_freq(*symbol);
        }
        (self.on_transmit)(frequencies);
    }

    pub fn send_message(&mut self, message: &[u8]) -> std::result::Result<(), &'static str> {
        if message.len() > MAX_MESSAGE_LEN {
            return Err("message too long");
        }
        let payload_bits = PAYLOAD_LEN * SYMBOL_BITS - 1;
        let mut bits: Vec<bool> = Vec::with_capacity((message.len()+1)*8+message.len()*8/payload_bits+1);
        for (i, &byte) in [message.len() as u8].iter().chain(message.iter()).enumerate() {
            for j in 0..8 {
                // insert bit at beginning of each packet's payload
                if (i*8+j)%payload_bits == 0 {
                    bits.push(false);
                }
                bits.push((byte>>(7-j))&1 == 1);
            }
        }
        // mark start of new message
        bits[0] = true;
        let mut symbols: Vec<u8> = Vec::with_capacity(bits.len()/SYMBOL_BITS+1);
        for i in (0..bits.len()).step_by(SYMBOL_BITS) {
            let mut symbol = 0_u8;
            for j in 0..SYMBOL_BITS {
                if let Some(true) = bits.get(i+j) {
                    symbol += 1<<(SYMBOL_BITS-j-1);
                }
            }
            debug_assert!((symbol as usize) < SYMBOL_COUNT);
            symbols.push(symbol);
        }
        for i in (0..symbols.len()).step_by(PAYLOAD_LEN) {
            let mut payload: [u8; PAYLOAD_LEN] = Default::default();
            for j in 0..PAYLOAD_LEN {
                if let Some(&symbol) = symbols.get(i+j) {
                    payload[j] = symbol;
                }
            }
            self.send(&payload);
        }
        return Ok(());
    }

    fn receive_message(&mut self, payload: [u8; PAYLOAD_LEN]) {
        let mut bits: Vec<bool> = Vec::with_capacity(PAYLOAD_LEN*SYMBOL_BITS);
        for &symbol in payload.iter() {
            for i in (0..SYMBOL_BITS).rev() {
                bits.push((symbol>>i)&1 == 1);
            }
        }
        // check for marker of new message
        if bits[0] == true {
            self.active_message.clear();
        } else if self.active_message.len() == 0 {
            return;
        } else {
            let age = self.measurement_count - self.active_message_received_at_measurement;
            if age > (PACKET_LEN * MEASUREMENTS_PER_SYMBOL + MAX_MEASUREMENTS_BETWEEN_PACKETS) as u64 {
                self.active_message.clear();
                return;
            }
        }
        self.active_message_received_at_measurement = self.measurement_count;
        self.active_message.extend(bits.iter().skip(1));
        let mut message: Vec<u8> = Vec::with_capacity(self.active_message.len()/8);
        let mut message_len = -1;
        for i in (0..self.active_message.len()/8*8).step_by(8) {
            let mut byte = 0_u8;
            for j in 0..8 {
                if self.active_message[i+j] {
                    byte += 1<<(8-j-1);
                }
            }
            // first byte is message length
            if i==0 {
                message_len = byte as isize;
            } else if message.len() as isize > message_len {
                // check zero padding
                if byte != 0 {
                    self.active_message.clear();
                    return;
                }
            } else {
                message.push(byte)
            }
        }
        if message.len() as isize >= message_len {
            self.active_message.clear();
            (self.on_received_message)(message.into_boxed_slice());
        }
    }

    pub fn push_sample(&mut self, sample: f32) {
        // Push new sample to ring buffer
        self.sample_buffer[self.sample_buffer_pos] = sample;
        self.sample_buffer_pos = mod_short!(self.sample_buffer_pos + 1, self.sample_buffer.len());
        self.remaining_samples -= 1.;
        if self.remaining_samples > 0. {
            return;
        }
        self.remaining_samples += (self.sample_rate as f32) * BEEP_TIME / (MEASUREMENTS_PER_SYMBOL as f32);
        self.measurement_count += 1;

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

        // Add new symbol to partial packets
        let completed_packet = self.packets[self.packets_pos];
        self.packets[self.packets_pos] = Default::default();
        self.packets[self.packets_pos].active = true;
        let next_symbol_snr = next_symbol_magnitude / next_symbol_noise;
        for i in 0..PACKET_LEN {
            let packet_pos = mod_short!(self.packets_pos + i * MEASUREMENTS_PER_SYMBOL, self.packets.len());
            let packet = &mut self.packets[packet_pos];
            if packet.active {
                packet.data[packet.data_pos] = next_symbol;
                packet.data_pos += 1;
                packet.signal_quality += next_symbol_snr;
            }
        }
        self.packets_pos = mod_short!(self.packets_pos + 1, self.packets.len());

        // Replace active packet, if completed packet is higher quality
        if completed_packet.active {
            debug_assert_eq!(completed_packet.data_pos, completed_packet.data.len());
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
                    if !self.active_packet || self.active_packet_quality < packet_quality {
                        self.active_packet_payload.copy_from_slice(&corrected_data[START_SYMBOLS_LEN..][..PAYLOAD_LEN]);
                        self.active_packet_quality = packet_quality;
                        if !self.active_packet {
                            self.active_packet = true;
                            self.active_packet_age = 0;
                            // Skip all packets after complete symbol
                            for i in MEASUREMENTS_PER_SYMBOL..self.packets.len() {
                                self.packets[mod_short!(self.packets_pos + i - 1, self.packets.len())].active = false;
                            }
                        }
                    }
                }
            }
        }

        // Delay receiving of active packet until symbol is complete
        if self.active_packet {
            self.active_packet_age += 1;
            if self.active_packet_age == MEASUREMENTS_PER_SYMBOL {
                self.active_packet = false;
                (self.on_received)(self.active_packet_payload);
                self.receive_message(self.active_packet_payload);
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
        let mut wav_reader = hound::WavReader::open("testsamples/packet.wav").unwrap();
        let mut expected_payload = [0u8; PAYLOAD_LEN];
        for (i, mnemonic) in "test".chars().enumerate() {
            expected_payload[i] = SYMBOL_MNEMONICS.find(mnemonic).unwrap() as u8;
        }
        let expected_payload = expected_payload;
        let received_payload = Arc::new(Mutex::new(None));
        let received_count = Arc::new(Mutex::new(0));
        let on_received = {
            let received_payload = received_payload.clone();
            let received_count = received_count.clone();
            move |payload| {
                *received_payload.lock().unwrap() = Some(payload);
                *received_count.lock().unwrap() += 1;
            }
        };
        let mut transceiver = Transceiver::new(
            wav_reader.spec().sample_rate,
            Box::new(on_received),
            Box::new(|_| panic!("should not be called")),
            Box::new(|_| panic!("should not be called")));
        for sample in wav_reader.samples::<f32>() {
            transceiver.push_sample(sample.unwrap());
        }
        assert_eq!(*received_count.lock().unwrap(), 1);
        assert_eq!(received_payload.lock().unwrap().unwrap(), expected_payload);
    }

    #[test]
    fn test_transmitter() {
        let mut payload = [0u8; PAYLOAD_LEN];
        for (i, mnemonic) in "test".chars().enumerate() {
            payload[i] = SYMBOL_MNEMONICS.find(mnemonic).unwrap() as u8;
        }
        let payload = payload;
        let transmit_frequencies = Arc::new(Mutex::new(None));
        let transmit_count = Arc::new(Mutex::new(0));
        let on_transmit = {
            let transmit_frequencies = transmit_frequencies.clone();
            let transmit_count = transmit_count.clone();
            move |frequencies| {
                *transmit_frequencies.lock().unwrap() = Some(frequencies);
                *transmit_count.lock().unwrap() += 1;
            }
        };
        let mut transceiver = Transceiver::new(
            48000,
            Box::new(|_| panic!("should not be called")),
            Box::new(|_| panic!("should not be called")),
            Box::new(on_transmit));
        transceiver.send(&payload);
        assert_eq!(*transmit_count.lock().unwrap(), 1);
        for &frequency in transmit_frequencies.lock().unwrap().unwrap().iter() {
            assert_ne!(frequency, 0.);
        }
    }

    #[test]
    fn test_receiver_message() {
        let mut wav_reader = hound::WavReader::open("testsamples/message.wav").unwrap();
        let received_message = Arc::new(Mutex::new(None));
        let received_count = Arc::new(Mutex::new(0));
        let on_received_message = {
            let received_message = received_message.clone();
            let received_count = received_count.clone();
            move |message: Box<[u8]>| {
                *received_message.lock().unwrap() = Some(message);
                *received_count.lock().unwrap() += 1;
            }
        };
        let mut transceiver = Transceiver::new(
            wav_reader.spec().sample_rate,
            Box::new(|_| ()),
            Box::new(on_received_message),
            Box::new(|_| panic!("should not be called")));
        for sample in wav_reader.samples::<f32>() {
            transceiver.push_sample(sample.unwrap());
        }
        assert_eq!(*received_count.lock().unwrap(), 1);
        assert_eq!(received_message.lock().unwrap().as_ref().unwrap().as_ref(), "Test Message 😀".as_bytes());
    }

    #[test]
    fn test_transmitter_message() {
        let transmit_frequencies = Arc::new(Mutex::new(vec![[0f32; PACKET_LEN]; 0]));
        let on_transmit = {
            let transmit_frequencies = transmit_frequencies.clone();
            move |frequencies| {
                transmit_frequencies.lock().unwrap().push(frequencies);
            }
        };
        let mut transceiver = Transceiver::new(
            48000,
            Box::new(|_| panic!("should not be called")),
            Box::new(|_| panic!("should not be called")),
            Box::new(on_transmit));
        transceiver.send_message("Test Message 😀".as_bytes()).unwrap();
        assert!(transmit_frequencies.lock().unwrap().len() > 0);
        for &frequencies in transmit_frequencies.lock().unwrap().iter() {
            for &frequency in frequencies.iter() {
                assert_ne!(frequency, 0.);
            }
        }
    }
}
