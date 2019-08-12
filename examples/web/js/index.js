import * as wasm from "../pkg/index.js";

wasm.init();

const OUTPUT_GAIN = 0.5;
const PERF_MEASUREMENT_DURATION = 2000;
const PERF_MIN = 0.98;
const BEEP_LEN = wasm.Transceiver.get_beep_len();
const ATTACK_LEN = wasm.Transceiver.get_attack_len();
const RELEASE_LEN = wasm.Transceiver.get_release_len();
const PAYLOAD_LEN = wasm.Transceiver.get_payload_len();
const SYMBOL_MNEMONICS = wasm.Transceiver.get_symbol_mnemonics();

let message_form = document.querySelector("form");
let message_input = message_form.querySelector("#message");
let message_submit = message_form.querySelector("button[type=submit]");
let log_container = document.querySelector("#log");

function log(msg, class_) {
    let e = document.createElement("div");
    e.classList.add(class_);
    e.textContent = msg;
    log_container.prepend(e);
}

function log_message(payload, class_) {
    let msg = "";
    for (let i = 0; i < payload.length; i++) {
        msg += SYMBOL_MNEMONICS.charAt(payload[i]);
    }
    log(msg, class_);
}

let audioCtx = new AudioContext();
let skip_recording_samples = 0;

function on_received(payload) {
    log_message(payload, "rx");
}

function on_transmit(frequencies) {
    let oscillator = audioCtx.createOscillator();
    let gainNode = audioCtx.createGain();
    oscillator.connect(gainNode);
    gainNode.connect(audioCtx.destination);
    oscillator.type = "sine";
    let now = audioCtx.currentTime;
    gainNode.gain.setValueAtTime(0, now);
    // Pre-program the oscillator
    for (let i = 0; i < frequencies.length; i++) {
        let beep_start = now + BEEP_LEN * i;
        gainNode.gain.linearRampToValueAtTime(OUTPUT_GAIN, beep_start + ATTACK_LEN);
        gainNode.gain.setValueAtTime(OUTPUT_GAIN, beep_start + BEEP_LEN - RELEASE_LEN);
        gainNode.gain.linearRampToValueAtTime(0, beep_start + BEEP_LEN);
        oscillator.frequency.setValueAtTime(frequencies[i], beep_start);
    }
    oscillator.start(now);
    oscillator.stop(now + (BEEP_LEN * frequencies.length));
    // Disable UI while transmitting
    message_submit.setAttribute("disabled", "");
    oscillator.onended = function() {
        message_submit.removeAttribute("disabled");
    }
    // Mute recording while transmitting
    skip_recording_samples = BEEP_LEN * frequencies.length * audioCtx.sampleRate;
}

let transceiver = wasm.Transceiver.new(audioCtx.sampleRate, on_received, on_transmit);

function on_gum_err(err) {
    console.log("The following gUM error occured: " + err);
    log("Microphone access denied: Unable to receive messages", "err");
}

if (navigator.mediaDevices) {
    navigator.mediaDevices.getUserMedia({audio: {
        echoCancellation: false, autoGainControl: false, noiseSuppression: false, channelCount: 1}})
    .then(function(stream) {
        let source = audioCtx.createMediaStreamSource(stream);
        let scriptNode = audioCtx.createScriptProcessor(0, 1, 1);
        source.connect(scriptNode);
        // HACK: chromium doesn't record audio, unless it's connected to a destination
        scriptNode.connect(audioCtx.destination);
        // Resume AudioContext after user interaction
        audioCtx.resume().catch(on_gum_err);
        let perf_container = document.createElement("div");
        perf_container.textContent = "Performance: ";
        let perf_display = document.createTextNode("...");
        perf_container.append(perf_display);
        log_container.prepend(perf_container);
        let perf_start = performance.now();  // timestamp is low-resolution!
        let perf_sample_count = 0;
        scriptNode.onaudioprocess = function(audioProcessingEvent) {
            let inputBuffer = audioProcessingEvent.inputBuffer;
            let inputData = inputBuffer.getChannelData(0);
            for (let i = 0; i < inputBuffer.length; i++) {
                skip_recording_samples -= 1;
                transceiver.push_sample(skip_recording_samples < 0 ? inputData[i] : 0);
            }
            perf_sample_count += inputBuffer.length;
            let perf_diff = performance.now() - perf_start;
            if (perf_diff >= PERF_MEASUREMENT_DURATION) {
                let perf = perf_sample_count / (inputBuffer.sampleRate * perf_diff / 1000);
                perf_start += perf_diff;
                perf_sample_count = 0;
                perf_display.textContent = Math.round(perf * 100) + "%";
                if (perf < PERF_MIN) {
                    perf_container.classList.add("err");
                } else {
                    perf_container.classList.remove("err");
                }
            }
        }
    }).catch(on_gum_err);
} else {
    on_gum_err("Not supported on your browser!");
}

message_form.addEventListener("submit", function(event) {
    event.preventDefault();
    let message = message_input.value;
    let payload = new Uint8Array(PAYLOAD_LEN);
    for (let i = 0, j = 0; i < message.length && j < payload.length; i++) {
        let symbol = SYMBOL_MNEMONICS.indexOf(message[i]);
        if (symbol !== -1) {
            payload[j] = symbol;
            j++;
        }
    }
    transceiver.send(payload);
    log_message(payload, "tx");
});

message_input.setAttribute("maxlength", PAYLOAD_LEN);
function validate_message_input() {
    let message = message_input.value;
    for (let i = 0; i < message.length; i++) {
        if (SYMBOL_MNEMONICS.indexOf(message[i]) == -1) {
            message_input.setCustomValidity("Allowed symbols: " + SYMBOL_MNEMONICS);
            return;
        }
    }
    message_input.setCustomValidity("");
}
validate_message_input();
message_input.addEventListener("input", validate_message_input);
