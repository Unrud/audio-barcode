extern crate audio_barcode;
extern crate js_sys;

use wasm_bindgen::prelude::*;
use audio_barcode::{PAYLOAD_LEN, FRAME_LEN, SYMBOL_MNEMONICS, BEEP_LEN, ATTACK_LEN, RELEASE_LEN};

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
    pub fn new(sample_rate: u32, on_received: js_sys::Function, on_transmit: js_sys::Function) -> Self {
        let on_received_wrapper = move |payload: [u8; PAYLOAD_LEN]| {
            let js_payload = js_sys::Array::new();
            for &c in payload.iter() {
                js_payload.push(&c.into());
            }
            on_received.call1(&JsValue::NULL, &js_payload).unwrap();
        };
        let on_transmit_wrapper = move |frequencies: [f32; FRAME_LEN]| {
            let js_frequencies = js_sys::Array::new();
            for &f in frequencies.iter() {
                js_frequencies.push(&f.into());
            }
            on_transmit.call1(&JsValue::NULL, &js_frequencies).unwrap();
        };
        
        Self(audio_barcode::Transceiver::new(
            sample_rate,
            Box::new(on_received_wrapper),
            Box::new(on_transmit_wrapper)))
    }

    pub fn send(&mut self, payload: &[u8]) {
        let mut payload_clone: [u8; PAYLOAD_LEN] = Default::default();
        payload_clone.clone_from_slice(payload);
        self.0.send(&payload_clone);
    }

    pub fn push_sample(&mut self, sample: f32) {
        self.0.push_sample(sample);
    }

    pub fn get_payload_len() -> usize {
        return PAYLOAD_LEN;
    }

    pub fn get_beep_len() -> f32 {
        BEEP_LEN
    }

    pub fn get_attack_len() -> f32 {
        ATTACK_LEN
    }

    pub fn get_release_len() -> f32 {
        RELEASE_LEN
    }

    pub fn get_symbol_mnemonics() -> String {
        SYMBOL_MNEMONICS.to_owned()
    }
}
