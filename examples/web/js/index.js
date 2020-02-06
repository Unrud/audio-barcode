import * as wasm from "../pkg/index.js";

wasm.init();

const OUTPUT_GAIN = 0.5;
const PERF_MEASUREMENT_DURATION = 2000;
const PERF_MIN = 0.98;
const BEEP_TIME = wasm.Transceiver.get_beep_time();
const ATTACK_TIME = wasm.Transceiver.get_attack_time();
const RELEASE_TIME = wasm.Transceiver.get_release_time();
const TIME_BETWEEN_PACKETS = wasm.Transceiver.get_time_between_packets();
const PAYLOAD_LEN = wasm.Transceiver.get_payload_len();
const SYMBOL_MNEMONICS = wasm.Transceiver.get_symbol_mnemonics();
const MAX_MESSAGE_LEN = wasm.Transceiver.get_max_message_len();

let message_btn = document.querySelector("#message-btn");
let message_form = document.querySelector("#message-form");
let message_input = message_form.querySelector("#message-form input");
let message_submit = message_form.querySelector("#message-form button");
let packet_btn = document.querySelector("#packet-btn");
let packet_form = document.querySelector("#packet-form");
let packet_input = packet_form.querySelector("#packet-form input");
let packet_submit = packet_form.querySelector("#packet-form button");
let log_container = document.querySelector("#log");

message_btn.addEventListener("click", function(event) {
    event.preventDefault();
    document.documentElement.classList.add("message");
    document.documentElement.classList.remove("packet");
});

packet_btn.addEventListener("click", function(event) {
    event.preventDefault();
    document.documentElement.classList.remove("message");
    document.documentElement.classList.add("packet");
});

function log(msg, classes) {
    let e = document.createElement("p");
    for (let class_ of classes) {
        e.classList.add(class_);
    }
    e.textContent = msg;
    log_container.prepend(e);
}

function log_packet(payload, direction) {
    let msg = "";
    for (let i = 0; i < payload.length; i++) {
        msg += SYMBOL_MNEMONICS.charAt(payload[i]);
    }
    log(msg, ["packet", direction]);
}

function log_message(text, direction) {
    log(text, ["message", direction]);
}

let audioCtx = new AudioContext();

function on_received(payload) {
    log_packet(payload, "rx");
}

function on_received_message(text) {
    let data = (new TextDecoder()).decode(new Uint8Array(text));
    log_message(data, "rx");
}

let pending_transmissions = [];
let transmission_in_progress = false;

function on_transmit(frequencies) {
    if (frequencies) {
        pending_transmissions.push(frequencies);
    }
    if (transmission_in_progress) {
        return;
    }
    frequencies = pending_transmissions.shift();
    if (!frequencies) {
        message_submit.removeAttribute("disabled");
        packet_submit.removeAttribute("disabled");
        return;
    }
    transmission_in_progress = true;
    // Disable UI while transmitting
    message_submit.setAttribute("disabled", "");
    packet_submit.setAttribute("disabled", "");
    let oscillator = audioCtx.createOscillator();
    let gainNode = audioCtx.createGain();
    oscillator.connect(gainNode);
    gainNode.connect(audioCtx.destination);
    oscillator.type = "sine";
    let now = audioCtx.currentTime;
    gainNode.gain.setValueAtTime(0, now);
    // Pre-program the oscillator
    for (let i = 0; i < frequencies.length; i++) {
        let beep_start = now + BEEP_TIME * i;
        gainNode.gain.linearRampToValueAtTime(OUTPUT_GAIN, beep_start + ATTACK_TIME);
        gainNode.gain.setValueAtTime(OUTPUT_GAIN, beep_start + BEEP_TIME - RELEASE_TIME);
        gainNode.gain.linearRampToValueAtTime(0, beep_start + BEEP_TIME);
        oscillator.frequency.setValueAtTime(frequencies[i], beep_start);
    }
    oscillator.start(now);
    oscillator.stop(now + BEEP_TIME * frequencies.length + TIME_BETWEEN_PACKETS);
    oscillator.onended = function() {
        transmission_in_progress = false;
        on_transmit();
    }
}

let transceiver = wasm.Transceiver.new(audioCtx.sampleRate, on_received, on_received_message, on_transmit);

function on_gum_err(err) {
    console.log("The following gUM error occured: " + err);
    log("Microphone access denied: Unable to receive", ["err"]);
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
        let perf_container = document.createElement("p");
        perf_container.textContent = "Performance: ";
        let perf_display = document.createTextNode("…");
        perf_container.append(perf_display);
        log_container.prepend(perf_container);
        let perf_start = performance.now();  // timestamp is low-resolution!
        let perf_sample_count = 0;
        scriptNode.onaudioprocess = function(audioProcessingEvent) {
            let inputBuffer = audioProcessingEvent.inputBuffer;
            let inputData = inputBuffer.getChannelData(0);
            for (let i = 0; i < inputBuffer.length; i++) {
                transceiver.push_sample(transmission_in_progress ? 0 : inputData[i]);
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
    let text = message_input.value;
    let data = (new TextEncoder()).encode(text);
    transceiver.send_message(data);
    log_message(text, "tx");
});

message_input.setAttribute("maxlength", MAX_MESSAGE_LEN);
function validate_message_input() {
    let text = message_input.value;
    let data = (new TextEncoder()).encode(text);
    if (data.length > MAX_MESSAGE_LEN) {
        message_input.setCustomValidity("Message too long: " + data.length);
        return;
    }
    message_input.setCustomValidity("");
}
validate_message_input();
message_input.addEventListener("input", validate_message_input);

packet_form.addEventListener("submit", function(event) {
    event.preventDefault();
    let text = packet_input.value;
    let payload = new Uint8Array(PAYLOAD_LEN);
    for (let i = 0, j = 0; i < text.length && j < payload.length; i++) {
        let symbol = SYMBOL_MNEMONICS.indexOf(text[i]);
        if (symbol !== -1) {
            payload[j] = symbol;
            j++;
        }
    }
    transceiver.send(payload);
    log_packet(payload, "tx");
});

packet_input.setAttribute("maxlength", PAYLOAD_LEN);
function validate_packet_input() {
    let text = packet_input.value;
    for (let symbol of text) {
        if (SYMBOL_MNEMONICS.indexOf(symbol) == -1) {
            packet_input.setCustomValidity("Allowed symbols: " + SYMBOL_MNEMONICS);
            return;
        }
    }
    packet_input.setCustomValidity("");
}
validate_packet_input();
packet_input.addEventListener("input", validate_packet_input);
