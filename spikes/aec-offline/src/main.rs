use aec_offline::{generate_synthetic, process_files, DEFAULT_FILTER_LENGTH, DEFAULT_FRAME_SIZE};
use std::env;
use std::error::Error;
use std::path::PathBuf;

#[derive(Debug, Default, PartialEq)]
struct CliOptions {
    reference: Option<PathBuf>,
    recorded: Option<PathBuf>,
    output: Option<PathBuf>,
    generate_synthetic: bool,
    frame_size: usize,
    filter_length: i32,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error}");
        eprintln!("\n{}", usage());
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let options = parse_args(env::args().skip(1))?;

    if options.generate_synthetic {
        let reference = required_path(&options.reference, "--reference")?;
        let recorded = required_path(&options.recorded, "--recorded")?;
        generate_synthetic(reference, recorded)?;
        println!("generated synthetic WAVs:");
        println!("  reference: {}", reference.display());
        println!("  recorded:  {}", recorded.display());

        if options.output.is_none() {
            return Ok(());
        }
    }

    let reference = required_path(&options.reference, "--reference")?;
    let recorded = required_path(&options.recorded, "--recorded")?;
    let output = required_path(&options.output, "--output")?;

    let metrics = process_files(
        reference,
        recorded,
        output,
        options.frame_size,
        options.filter_length,
    )?;

    println!("wrote cleaned WAV: {}", output.display());
    println!("estimated ERLE: {:.2} dB", metrics.erle_db);
    println!("recorded RMS: {:.2}", metrics.recorded_rms);
    println!("output RMS: {:.2}", metrics.output_rms);
    println!(
        "reference correlation: recorded={:.4}, output={:.4}",
        metrics.recorded_reference_correlation, metrics.output_reference_correlation
    );
    println!(
        "pass criterion (>10 dB ERLE): {}",
        if metrics.erle_db > 10.0 {
            "PASS"
        } else {
            "FAIL"
        }
    );

    Ok(())
}

fn parse_args(args: impl IntoIterator<Item = String>) -> Result<CliOptions, Box<dyn Error>> {
    let mut options = CliOptions {
        frame_size: DEFAULT_FRAME_SIZE,
        filter_length: DEFAULT_FILTER_LENGTH,
        ..CliOptions::default()
    };
    let mut args = args.into_iter();

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--reference" => options.reference = Some(next_path(&mut args, "--reference")?),
            "--recorded" => options.recorded = Some(next_path(&mut args, "--recorded")?),
            "--output" => options.output = Some(next_path(&mut args, "--output")?),
            "--frame-size" => {
                options.frame_size = next_value(&mut args, "--frame-size")?.parse()?
            }
            "--filter-length" => {
                options.filter_length = next_value(&mut args, "--filter-length")?.parse()?
            }
            "--generate-synthetic" => options.generate_synthetic = true,
            "--help" | "-h" => return Err(usage().into()),
            unknown => return Err(format!("unknown argument: {unknown}").into()),
        }
    }

    if options.frame_size == 0 {
        return Err("--frame-size must be greater than zero".into());
    }
    if options.filter_length <= 0 {
        return Err("--filter-length must be greater than zero".into());
    }

    Ok(options)
}

fn next_path(
    args: &mut impl Iterator<Item = String>,
    flag: &'static str,
) -> Result<PathBuf, Box<dyn Error>> {
    Ok(PathBuf::from(next_value(args, flag)?))
}

fn next_value(
    args: &mut impl Iterator<Item = String>,
    flag: &'static str,
) -> Result<String, Box<dyn Error>> {
    args.next()
        .ok_or_else(|| format!("missing value for {flag}").into())
}

fn required_path<'a>(
    path: &'a Option<PathBuf>,
    flag: &'static str,
) -> Result<&'a PathBuf, Box<dyn Error>> {
    path.as_ref()
        .ok_or_else(|| format!("missing required argument {flag}").into())
}

fn usage() -> &'static str {
    "Usage:\n  aec-offline --reference ref.wav --recorded mic.wav --output clean.wav [--frame-size 480] [--filter-length 9600]\n  aec-offline --generate-synthetic --reference ref.wav --recorded mic.wav [--output clean.wav]\n\nInputs must be 48 kHz mono signed 16-bit PCM WAV."
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_processing_arguments() {
        let options = parse_args(
            [
                "--reference",
                "ref.wav",
                "--recorded",
                "mic.wav",
                "--output",
                "clean.wav",
            ]
            .map(String::from),
        )
        .expect("valid arguments");

        assert_eq!(options.reference, Some(PathBuf::from("ref.wav")));
        assert_eq!(options.recorded, Some(PathBuf::from("mic.wav")));
        assert_eq!(options.output, Some(PathBuf::from("clean.wav")));
        assert_eq!(options.frame_size, DEFAULT_FRAME_SIZE);
        assert_eq!(options.filter_length, DEFAULT_FILTER_LENGTH);
    }

    #[test]
    fn rejects_zero_frame_size() {
        let error = parse_args(["--frame-size", "0"].map(String::from)).expect_err("invalid");

        assert!(error.to_string().contains("--frame-size"));
    }
}
