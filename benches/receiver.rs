#[macro_use]
extern crate bencher;

use bencher::Bencher;

use audio_barcode::test_utils::*;
use audio_barcode::*;

fn receiver(bench: &mut Bencher) {
    const SAMPLE_RATE: u32 = 44100;
    let mut transceiver = Transceiver::new(SAMPLE_RATE);
    let samples = {
        let payload = rand_payload(0);
        let mut samples = Vec::new();
        // prepend 0.5 seconds of silence
        samples.extend(vec![0.; (SAMPLE_RATE / 2) as usize]);
        for &frequency in transceiver.send(&payload).iter() {
            samples.extend(transceiver.generate_beep(frequency).iter());
        }
        // append 0.5 seconds of silence
        samples.extend(vec![0.; (SAMPLE_RATE / 2) as usize]);
        samples
    };
    bench.iter(|| {
        for &sample in samples.iter() {
            transceiver.push_sample(sample);
        }
    });
}

benchmark_group!(benches, receiver);
benchmark_main!(benches);
