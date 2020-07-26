extern crate audio_barcode;
extern crate js_sys;

use audio_barcode::{
    Transceiver, ATTACK_TIME, BEEP_TIME, PACKET_LEN, PAYLOAD_LEN, RELEASE_TIME, SYMBOL_BITS,
    SYMBOL_COUNT, SYMBOL_MNEMONICS,
};
use wasm_bindgen::prelude::*;

pub const MAX_MESSAGE_LEN: usize = 255;
const MAX_TIME_BETWEEN_PACKETS: f32 = BEEP_TIME * (PACKET_LEN as f32);

// When the `wee_alloc` feature is enabled, this uses `wee_alloc` as the global
// allocator.
#[cfg(feature = "wee_alloc")]
#[global_allocator]
static ALLOC: wee_alloc::WeeAlloc = wee_alloc::WeeAlloc::INIT;

#[allow(unused_macros)]
macro_rules! log {
    ( $( $t:tt )* ) => {
        web_sys::console::log_1(&format!( $( $t )* ).into());
    }
}

#[wasm_bindgen]
pub fn init() {
    // This provides better error messages, when the `console_error_panic_hook`
    // feature is enabled. Unfortunately it bloats up the file size.
    #[cfg(feature = "console_error_panic_hook")]
    console_error_panic_hook::set_once();
}

#[wasm_bindgen]
pub struct MessageTransceiver {
    transceiver: Transceiver,
    on_transmit: Box<dyn FnMut([u8; PAYLOAD_LEN], [f32; PACKET_LEN])>,
    on_received: Box<dyn FnMut([u8; PAYLOAD_LEN])>,
    on_received_message: Box<dyn FnMut(Box<[u8]>)>,
    sample_count: u64,
    active_message: Vec<bool>,
    last_packet_received_at_sample: u64,
    max_samples_until_next_packet: u64,
}

#[wasm_bindgen]
impl MessageTransceiver {
    pub fn new(
        sample_rate: u32,
        on_received: js_sys::Function,
        on_received_message: js_sys::Function,
        on_transmit: js_sys::Function,
    ) -> Self {
        let on_received_wrapper = move |payload: [u8; PAYLOAD_LEN]| {
            let js_payload = js_sys::Array::new();
            for &c in payload.iter() {
                js_payload.push(&c.into());
            }
            on_received.call1(&JsValue::NULL, &js_payload).unwrap();
        };
        let on_received_message_wrapper = move |message: Box<[u8]>| {
            let js_message = js_sys::Array::new();
            for &c in message.iter() {
                js_message.push(&c.into());
            }
            on_received_message
                .call1(&JsValue::NULL, &js_message)
                .unwrap();
        };
        let on_transmit_wrapper =
            move |payload: [u8; PAYLOAD_LEN], frequencies: [f32; PACKET_LEN]| {
                let js_payload = js_sys::Array::new();
                for &c in payload.iter() {
                    js_payload.push(&c.into());
                }
                let js_frequencies = js_sys::Array::new();
                for &f in frequencies.iter() {
                    js_frequencies.push(&f.into());
                }
                on_transmit
                    .call2(&JsValue::NULL, &js_payload, &js_frequencies)
                    .unwrap();
            };

        Self::new_with_closures(
            sample_rate,
            Box::new(on_received_wrapper),
            Box::new(on_received_message_wrapper),
            Box::new(on_transmit_wrapper),
        )
    }

    pub fn send(&mut self, payload: &[u8]) {
        let mut payload_clone: [u8; PAYLOAD_LEN] = Default::default();
        payload_clone.clone_from_slice(payload);
        (self.on_transmit)(payload_clone, self.transceiver.send(&payload_clone));
    }

    pub fn send_message(&mut self, message: &[u8]) {
        if message.len() > MAX_MESSAGE_LEN {
            panic!("message too long");
        }
        // first bit is used to mark new message or continuation
        let useable_payload_bits = PAYLOAD_LEN * SYMBOL_BITS - 1;
        // prefix message with length
        let message_prefix = [message.len() as u8];
        let mut bits = Vec::<bool>::with_capacity(
            (message_prefix.len() + message.len()) * (8 + (8 / useable_payload_bits + 1)),
        );
        for (i, &byte) in message_prefix.iter().chain(message.iter()).enumerate() {
            for j in 0..8 {
                // insert a bit at beginning of each packet's payload
                if (i * 8 + j) % useable_payload_bits == 0 {
                    // set to 1 for start of message and 0 for following packets
                    bits.push(i == 0);
                }
                bits.push((byte >> (7 - j)) & 1 == 1);
            }
        }
        let mut symbols = Vec::<u8>::with_capacity(bits.len() / SYMBOL_BITS + 1);
        for i in (0..bits.len()).step_by(SYMBOL_BITS) {
            let mut symbol = 0u8;
            for j in 0..SYMBOL_BITS {
                if let Some(true) = bits.get(i + j) {
                    symbol += 1 << (SYMBOL_BITS - j - 1);
                }
            }
            debug_assert!((symbol as usize) < SYMBOL_COUNT);
            symbols.push(symbol);
        }
        for i in (0..symbols.len()).step_by(PAYLOAD_LEN) {
            let mut payload = [0u8; PAYLOAD_LEN];
            for j in 0..PAYLOAD_LEN {
                if let Some(&symbol) = symbols.get(i + j) {
                    payload[j] = symbol;
                }
            }
            (self.on_transmit)(payload, self.transceiver.send(&payload));
        }
    }

    pub fn push_sample(&mut self, sample: f32) {
        self.sample_count += 1;
        if let Some(payload) = self.transceiver.push_sample(sample) {
            (self.on_received)(payload);
            self.receive_message(payload);
        }
    }

    pub fn get_payload_len() -> usize {
        return PAYLOAD_LEN;
    }

    pub fn get_beep_time() -> f32 {
        BEEP_TIME
    }

    pub fn get_attack_time() -> f32 {
        ATTACK_TIME
    }

    pub fn get_release_time() -> f32 {
        RELEASE_TIME
    }

    pub fn get_symbol_mnemonics() -> String {
        SYMBOL_MNEMONICS.to_owned()
    }

    pub fn get_max_message_len() -> usize {
        MAX_MESSAGE_LEN
    }
}

impl MessageTransceiver {
    fn new_with_closures(
        sample_rate: u32,
        on_received: Box<dyn FnMut([u8; PAYLOAD_LEN])>,
        on_received_message: Box<dyn FnMut(Box<[u8]>)>,
        on_transmit: Box<dyn FnMut([u8; PAYLOAD_LEN], [f32; PACKET_LEN])>,
    ) -> Self {
        Self {
            transceiver: Transceiver::new(sample_rate),
            on_transmit: on_transmit,
            on_received: on_received,
            on_received_message: on_received_message,
            sample_count: 0,
            active_message: Vec::new(),
            last_packet_received_at_sample: 0,
            max_samples_until_next_packet: ((BEEP_TIME * (PACKET_LEN as f32)
                + MAX_TIME_BETWEEN_PACKETS)
                * (sample_rate as f32))
                .round() as u64,
        }
    }

    fn receive_message(&mut self, payload: [u8; PAYLOAD_LEN]) {
        let mut bits = Vec::<bool>::with_capacity(PAYLOAD_LEN * SYMBOL_BITS);
        for &symbol in payload.iter() {
            for i in (0..SYMBOL_BITS).rev() {
                bits.push((symbol >> i) & 1 == 1);
            }
        }
        // check for marker of new message
        if bits[0] == true {
            self.active_message.clear();
        } else if self.active_message.len() == 0 {
            return;
        } else {
            let age = self.sample_count - self.last_packet_received_at_sample;
            if age > self.max_samples_until_next_packet {
                self.active_message.clear();
                return;
            }
        }
        self.last_packet_received_at_sample = self.sample_count;
        self.active_message.reserve(bits.len() - 1);
        self.active_message.extend(bits.iter().skip(1));
        let mut message = Vec::<u8>::with_capacity(self.active_message.len() / 8);
        let mut message_len = -1;
        for i in (0..self.active_message.len() / 8 * 8).step_by(8) {
            let mut byte = 0u8;
            for j in 0..8 {
                if self.active_message[i + j] {
                    byte += 1 << (8 - j - 1);
                }
            }
            // first byte is message length
            if i == 0 {
                message_len = byte as isize;
            } else if message.len() as isize >= message_len {
                // check zero padding
                if byte != 0 {
                    self.active_message.clear();
                    return;
                }
            } else {
                message.push(byte)
            }
        }
        if message_len != -1 && message.len() as isize >= message_len {
            self.active_message.clear();
            (self.on_received_message)(message.into_boxed_slice());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use audio_barcode::test_utils::*;
    use std::sync::{Arc, Mutex};

    #[test]
    fn test_send_and_receive_message() {
        const SAMPLE_RATE: u32 = 44100;
        const MESSAGES: [&str; 5] = ["Test Message", "Another Message", "", "Hi", "😀😁😂😃😄😅"];
        let frequency_queue = Arc::new(Mutex::new(Vec::<f32>::new()));
        let on_transmit = {
            let frequency_queue = frequency_queue.clone();
            move |_payload, frequencies: [f32; PACKET_LEN]| {
                frequency_queue.lock().unwrap().extend(frequencies.iter());
            }
        };
        let received_message_count = Arc::new(Mutex::new(0));
        let on_received_message = {
            let received_message_count = received_message_count.clone();
            move |message: Box<[u8]>| {
                let mut received_message_count = received_message_count.lock().unwrap();
                assert!(*received_message_count < MESSAGES.len());
                assert_eq!(
                    *message,
                    *MESSAGES[*received_message_count].as_bytes(),
                    "wrong message received, expected {:?}",
                    MESSAGES[*received_message_count]
                );
                *received_message_count += 1;
            }
        };
        let mut message_transceiver = MessageTransceiver::new_with_closures(
            SAMPLE_RATE,
            Box::new(|_| ()),
            Box::new(on_received_message),
            Box::new(on_transmit),
        );
        let send_frequency_queue = |message_transceiver: &mut MessageTransceiver| {
            while !frequency_queue.lock().unwrap().is_empty() {
                let frequency = frequency_queue.lock().unwrap().remove(0);
                for &sample in message_transceiver
                    .transceiver
                    .generate_beep(frequency)
                    .iter()
                {
                    message_transceiver.push_sample(sample);
                }
            }
        };
        for &message in MESSAGES.iter() {
            message_transceiver.send_message(message.as_bytes());
            send_frequency_queue(&mut message_transceiver);
        }
        for _ in 0..((SAMPLE_RATE as f32) * BEEP_TIME).ceil() as u32 {
            message_transceiver.push_sample(0.);
        }
        send_frequency_queue(&mut message_transceiver);
        assert_eq!(*received_message_count.lock().unwrap(), MESSAGES.len());
    }

    #[test]
    fn test_send() {
        const SAMPLE_RATE: u32 = 44100;
        const SEND_COUNT: usize = 5;
        let transmit_count = Arc::new(Mutex::new(0));
        let on_transmit = {
            let transmit_count = transmit_count.clone();
            move |payload, _frequencies: [f32; PACKET_LEN]| {
                let mut transmit_count = transmit_count.lock().unwrap();
                assert_eq!(payload, rand_payload(*transmit_count));
                *transmit_count += 1;
            }
        };
        let mut message_transceiver = MessageTransceiver::new_with_closures(
            SAMPLE_RATE,
            Box::new(|_| panic!("should not be called")),
            Box::new(|_| panic!("should not be called")),
            Box::new(on_transmit),
        );
        for i in 0..SEND_COUNT {
            message_transceiver.send(&rand_payload(i));
        }
        assert_eq!(*transmit_count.lock().unwrap(), SEND_COUNT);
    }
}
