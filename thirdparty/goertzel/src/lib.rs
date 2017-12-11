// This (implicitly/naively) uses a rectangular window.  Some way to set up a window function
// will be needed probably -- if mutating your samples before calling this isn't sufficient.
use std::f32;
use std::f32::consts::PI;

/// Set up parameters (and some precomputed values for those).
#[derive(Clone, Copy)]
pub struct Parameters {
    // Parameters we're working with
    window_size: usize,
    // Precomputed value:
    sine: f32,
    cosine: f32,
    term_coefficient: f32,
}

pub struct Partial {
    params: Parameters,
    count: usize,
    prev: f32,
    prevprev: f32,
}

impl Parameters {
    pub fn new(target_freq: f32, sample_rate: u32, window_size: usize) -> Self {
        let k = target_freq * (window_size as f32) / (sample_rate as f32);
        let omega = (PI * 2. * k) / (window_size as f32);
        let cosine = omega.cos();
        Parameters {
            window_size: window_size,
            sine: omega.sin(),
            cosine: cosine,
            term_coefficient: 2. * cosine,
        }
    }

    pub fn start(self) -> Partial {
        Partial{ params: self, count: 0, prev: 0., prevprev: 0. }
    }

    pub fn mag(self, samples: &[f32]) -> f32 {
        self.start().add(samples).finish_mag()
    }
}

impl Partial {
    pub fn add(mut self, samples: &[f32]) -> Self {
        for &sample in samples {
            let this = self.params.term_coefficient * self.prev - self.prevprev + sample;
            self.prevprev = self.prev;
            self.prev = this;
        }
        self.count += samples.len();
        self
    }
    pub fn finish(self) -> (f32, f32) {
        assert_eq!(self.count, self.params.window_size);
        let real = self.prev - self.prevprev * self.params.cosine;
        let imag = self.prevprev * self.params.sine;
        (real, imag)
    }

    pub fn finish_mag(self) -> f32 {
        let (real, imag) = self.finish();
        (real*real + imag*imag).sqrt()
    }
}

#[test]
fn zero_data() {
    let p = Parameters::new(1800., 8000, 256);
    assert!(p.start().add(&[0.; 256]).finish_mag() == 0.);
    assert!(p.start().add(&[0.; 128]).add(&[0.; 128]).finish_mag() == 0.);
}

#[test]
fn sine() {
    let mut buf = [0.; 8000];
    for &freq in [697., 1200., 1800., 1633.].iter() {
        // Generate a 1 second sine wave at freq hz
        let step = 1. / 8000.;
        for sample in 0..8000 {
            let time = sample as f32 * step;
            buf[sample] = (time * freq * PI * 2.).sin();
        }

        let p = Parameters::new(freq, 8000, 8000);
        let mag = p.start().add(&buf[..]).finish_mag();
        for testfreq in (0 .. 30).map(|x| (x * 100) as f32) {
            let p = Parameters::new(testfreq, 8000, 8000);
            let testmag = p.mag(&buf[..]);
            println!("{:4}: {:12.3}", testfreq, testmag);
            if (freq-testfreq).abs() > 100. {
                println!("{} > 10*{}", mag, testmag);
                assert!(mag > 10.*testmag);
            }
        }
    }
}

