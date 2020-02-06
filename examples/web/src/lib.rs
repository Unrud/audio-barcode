extern crate audio_barcode;
extern crate js_sys;

use wasm_bindgen::prelude::*;
use audio_barcode::{PAYLOAD_LEN, PACKET_LEN, TIME_BETWEEN_PACKETS, SYMBOL_MNEMONICS, MAX_MESSAGE_LEN, BEEP_TIME, ATTACK_TIME, RELEASE_TIME};

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
    // This provides better error messages in debug mode.
    // It's disabled in release mode so it doesn't bloat up the file size.
    #[cfg(debug_assertions)]
    console_error_panic_hook::set_once();
}

#[wasm_bindgen]
pub struct Transceiver (audio_barcode::Transceiver);

#[wasm_bindgen]
impl Transceiver {
    pub fn new(sample_rate: u32, on_received: js_sys::Function, on_received_message: js_sys::Function, on_transmit: js_sys::Function) -> Self {
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
            on_received_message.call1(&JsValue::NULL, &js_message).unwrap();
        };
        let on_transmit_wrapper = move |frequencies: [f32; PACKET_LEN]| {
            let js_frequencies = js_sys::Array::new();
            for &f in frequencies.iter() {
                js_frequencies.push(&f.into());
            }
            on_transmit.call1(&JsValue::NULL, &js_frequencies).unwrap();
        };
        
        Self(audio_barcode::Transceiver::new(
            sample_rate,
            Box::new(on_received_wrapper),
            Box::new(on_received_message_wrapper),
            Box::new(on_transmit_wrapper)))
    }

    pub fn send(&mut self, payload: &[u8]) {
        let mut payload_clone: [u8; PAYLOAD_LEN] = Default::default();
        payload_clone.clone_from_slice(payload);
        self.0.send(&payload_clone);
    }

    pub fn send_message(&mut self, message: &[u8]) {
        self.0.send_message(message).unwrap();
    }

    pub fn push_sample(&mut self, sample: f32) {
        self.0.push_sample(sample);
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

    pub fn get_time_between_packets() -> f32 {
        TIME_BETWEEN_PACKETS
    }

    pub fn get_symbol_mnemonics() -> String {
        SYMBOL_MNEMONICS.to_owned()
    }

    pub fn get_max_message_len() -> usize {
        MAX_MESSAGE_LEN
    }
}
