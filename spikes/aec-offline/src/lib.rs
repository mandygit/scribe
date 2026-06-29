use hound::{SampleFormat, WavReader, WavSpec, WavWriter};
use std::error::Error;
use std::f32::consts::PI;
use std::ffi::{c_int, c_void};
use std::path::Path;

const SPEEX_ECHO_SET_SAMPLING_RATE: c_int = 24;

#[repr(C)]
struct SpeexEchoState {
    _private: [u8; 0],
}

#[link(name = "speexdsp")]
extern "C" {
    fn speex_echo_state_init(frame_size: c_int, filter_length: c_int) -> *mut SpeexEchoState;
    fn speex_echo_state_destroy(state: *mut SpeexEchoState);
    fn speex_echo_cancellation(
        state: *mut SpeexEchoState,
        rec: *const i16,
        play: *const i16,
        out: *mut i16,
    );
    fn speex_echo_ctl(state: *mut SpeexEchoState, request: c_int, ptr: *mut c_void) -> c_int;
}

struct SpeexAec {
    state: *mut SpeexEchoState,
}

impl SpeexAec {
    fn new(
        frame_size: usize,
        filter_length: i32,
        sample_rate: u32,
    ) -> Result<Self, Box<dyn Error>> {
        let frame_size = c_int::try_from(frame_size)?;
        let filter_length = c_int::try_from(filter_length)?;
        let state = unsafe { speex_echo_state_init(frame_size, filter_length) };
        if state.is_null() {
            return Err("failed to initialize SpeexDSP echo state".into());
        }

        let mut sample_rate = c_int::try_from(sample_rate)?;
        let control_result = unsafe {
            speex_echo_ctl(
                state,
                SPEEX_ECHO_SET_SAMPLING_RATE,
                (&mut sample_rate as *mut c_int).cast::<c_void>(),
            )
        };
        if control_result != 0 {
            unsafe { speex_echo_state_destroy(state) };
            return Err("failed to configure SpeexDSP sample rate".into());
        }

        Ok(Self { state })
    }

    fn cancel_echo(&mut self, recorded: &[i16], reference: &[i16], output: &mut [i16]) {
        unsafe {
            speex_echo_cancellation(
                self.state,
                recorded.as_ptr(),
                reference.as_ptr(),
                output.as_mut_ptr(),
            )
        };
    }
}

impl Drop for SpeexAec {
    fn drop(&mut self) {
        unsafe { speex_echo_state_destroy(self.state) };
    }
}

pub const SAMPLE_RATE: u32 = 48_000;
pub const CHANNELS: u16 = 1;
pub const BITS_PER_SAMPLE: u16 = 16;
pub const DEFAULT_FRAME_SIZE: usize = 480;
pub const DEFAULT_FILTER_LENGTH: i32 = 9_600;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Metrics {
    pub erle_db: f64,
    pub recorded_rms: f64,
    pub output_rms: f64,
    pub recorded_reference_correlation: f64,
    pub output_reference_correlation: f64,
}

pub fn process_files(
    reference_path: &Path,
    recorded_path: &Path,
    output_path: &Path,
    frame_size: usize,
    filter_length: i32,
) -> Result<Metrics, Box<dyn Error>> {
    let reference = read_mono_i16_48k(reference_path)?;
    let recorded = read_mono_i16_48k(recorded_path)?;

    if reference.len() != recorded.len() {
        return Err(format!(
            "reference and recorded WAVs must have equal sample counts ({} != {})",
            reference.len(),
            recorded.len()
        )
        .into());
    }

    let output = cancel_echo(&reference, &recorded, frame_size, filter_length)?;
    write_mono_i16_48k(output_path, &output)?;
    Ok(compute_metrics(&reference, &recorded, &output))
}

pub fn cancel_echo(
    reference: &[i16],
    recorded: &[i16],
    frame_size: usize,
    filter_length: i32,
) -> Result<Vec<i16>, Box<dyn Error>> {
    let mut aec = SpeexAec::new(frame_size, filter_length, SAMPLE_RATE)?;
    let mut output = Vec::with_capacity(recorded.len());

    for (reference_frame, recorded_frame) in reference
        .chunks(frame_size)
        .zip(recorded.chunks(frame_size))
    {
        let mut padded_reference = vec![0_i16; frame_size];
        let mut padded_recorded = vec![0_i16; frame_size];
        padded_reference[..reference_frame.len()].copy_from_slice(reference_frame);
        padded_recorded[..recorded_frame.len()].copy_from_slice(recorded_frame);

        let mut cleaned_frame = vec![0_i16; frame_size];
        aec.cancel_echo(&padded_recorded, &padded_reference, &mut cleaned_frame);
        output.extend_from_slice(&cleaned_frame[..recorded_frame.len()]);
    }

    Ok(output)
}

pub fn generate_synthetic(
    reference_path: &Path,
    recorded_path: &Path,
) -> Result<(), Box<dyn Error>> {
    let sample_count = SAMPLE_RATE as usize * 6;
    let reference: Vec<i16> = (0..sample_count)
        .map(|index| synthetic_reference_sample(index, SAMPLE_RATE as f32))
        .map(float_to_i16)
        .collect();

    let recorded: Vec<i16> = (0..sample_count)
        .map(|index| synthetic_echo_sample(index, &reference))
        .collect();

    write_mono_i16_48k(reference_path, &reference)?;
    write_mono_i16_48k(recorded_path, &recorded)?;
    Ok(())
}

pub fn compute_metrics(reference: &[i16], recorded: &[i16], output: &[i16]) -> Metrics {
    let recorded_power = mean_square(recorded);
    let output_power = mean_square(output).max(1.0);
    Metrics {
        erle_db: 10.0 * (recorded_power / output_power).log10(),
        recorded_rms: recorded_power.sqrt(),
        output_rms: output_power.sqrt(),
        recorded_reference_correlation: absolute_correlation(reference, recorded),
        output_reference_correlation: absolute_correlation(reference, output),
    }
}

fn read_mono_i16_48k(path: &Path) -> Result<Vec<i16>, Box<dyn Error>> {
    let mut reader = WavReader::open(path)?;
    let spec = reader.spec();
    if spec.channels != CHANNELS
        || spec.sample_rate != SAMPLE_RATE
        || spec.bits_per_sample != BITS_PER_SAMPLE
        || spec.sample_format != SampleFormat::Int
    {
        return Err(format!(
            "{} must be 48 kHz mono signed 16-bit PCM WAV; got channels={}, sample_rate={}, bits_per_sample={}, format={:?}",
            path.display(),
            spec.channels,
            spec.sample_rate,
            spec.bits_per_sample,
            spec.sample_format
        )
        .into());
    }

    reader
        .samples::<i16>()
        .collect::<Result<Vec<_>, _>>()
        .map_err(Into::into)
}

fn write_mono_i16_48k(path: &Path, samples: &[i16]) -> Result<(), Box<dyn Error>> {
    let spec = WavSpec {
        channels: CHANNELS,
        sample_rate: SAMPLE_RATE,
        bits_per_sample: BITS_PER_SAMPLE,
        sample_format: SampleFormat::Int,
    };
    let mut writer = WavWriter::create(path, spec)?;
    for sample in samples {
        writer.write_sample(*sample)?;
    }
    writer.finalize()?;
    Ok(())
}

fn mean_square(samples: &[i16]) -> f64 {
    if samples.is_empty() {
        return 0.0;
    }

    let sum = samples
        .iter()
        .map(|sample| f64::from(*sample))
        .map(|sample| sample * sample)
        .sum::<f64>();
    sum / samples.len() as f64
}

fn absolute_correlation(left: &[i16], right: &[i16]) -> f64 {
    let sample_count = left.len().min(right.len());
    if sample_count == 0 {
        return 0.0;
    }

    let (dot, left_energy, right_energy) = left.iter().zip(right.iter()).take(sample_count).fold(
        (0.0, 0.0, 0.0),
        |(dot, left_energy, right_energy), (left_sample, right_sample)| {
            let left_value = f64::from(*left_sample);
            let right_value = f64::from(*right_sample);
            (
                dot + left_value * right_value,
                left_energy + left_value * left_value,
                right_energy + right_value * right_value,
            )
        },
    );

    if left_energy == 0.0 || right_energy == 0.0 {
        0.0
    } else {
        (dot / (left_energy.sqrt() * right_energy.sqrt())).abs()
    }
}

fn synthetic_reference_sample(index: usize, sample_rate: f32) -> f32 {
    let t = index as f32 / sample_rate;
    let sweep_frequency = 300.0 + 1_700.0 * (t / 6.0);
    let multitone = (2.0 * PI * 440.0 * t).sin() * 0.35
        + (2.0 * PI * 1_130.0 * t).sin() * 0.25
        + (2.0 * PI * sweep_frequency * t).sin() * 0.20;
    multitone * 12_000.0
}

fn synthetic_echo_sample(index: usize, reference: &[i16]) -> i16 {
    let echo_taps = [(240_usize, 0.62_f32), (640, 0.28), (1_180, 0.13)];
    let echo = echo_taps
        .iter()
        .filter_map(|(delay, gain)| index.checked_sub(*delay).map(|position| (position, gain)))
        .map(|(position, gain)| f32::from(reference[position]) * gain)
        .sum::<f32>();
    float_to_i16(echo)
}

fn float_to_i16(sample: f32) -> i16 {
    sample.clamp(f32::from(i16::MIN), f32::from(i16::MAX)) as i16
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metrics_report_positive_erle_when_output_is_quieter() {
        let reference = vec![1_000_i16, -1_000, 1_000, -1_000];
        let recorded = vec![2_000_i16, -2_000, 2_000, -2_000];
        let output = vec![200_i16, -200, 200, -200];

        let metrics = compute_metrics(&reference, &recorded, &output);

        assert!(metrics.erle_db > 19.0);
        assert!(metrics.recorded_rms > metrics.output_rms);
    }

    #[test]
    fn echo_cancellation_preserves_sample_count_for_partial_frame() {
        let reference = vec![0_i16; DEFAULT_FRAME_SIZE + 17];
        let recorded = vec![0_i16; DEFAULT_FRAME_SIZE + 17];

        let output = cancel_echo(
            &reference,
            &recorded,
            DEFAULT_FRAME_SIZE,
            DEFAULT_FILTER_LENGTH,
        )
        .expect("echo cancellation succeeds");

        assert_eq!(output.len(), recorded.len());
    }
}
