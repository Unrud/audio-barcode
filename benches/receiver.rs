#[macro_use]
extern crate bencher;

use bencher::Bencher;

use audio_barcode::*;

fn receiver(bench: &mut Bencher) {
    let mut wav_reader = hound::WavReader::open("testsamples/packet.wav").unwrap();
    let mut samples = vec![0f32; 0];
    for sample in wav_reader.samples::<f32>() {
        samples.push(sample.unwrap());
    }
    let samples = samples;
    let mut transceiver = Transceiver::new(
        wav_reader.spec().sample_rate,
        Box::new(|_| ()),
        Box::new(|_| ()),
        Box::new(|_| ()));
    bench.iter(|| {
        for &sample in samples.iter() {
            transceiver.push_sample(sample);
        }
    });
}

benchmark_group!(benches, receiver);
benchmark_main!(benches);
