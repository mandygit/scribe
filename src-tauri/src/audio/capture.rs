use std::{
    path::PathBuf,
    str::FromStr,
    sync::{
        atomic::{AtomicU32, AtomicU64, Ordering},
        mpsc::{sync_channel, SyncSender, TrySendError},
        Arc, Mutex, Weak,
    },
};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

use crate::domain::AppError;

use super::{
    audio_error,
    domain::AudioDevice,
    wav::{spawn_i16_mono_wav_writer, WavWriterHandle},
};

// Five seconds of 48 kHz mono i16 samples. This absorbs short disk scheduling
// hiccups without allowing unbounded memory growth in the audio callback path.
const AUDIO_SAMPLE_CHANNEL_CAPACITY: usize = 48_000 * 5;

pub trait AudioCaptureBackend: Send + Sync {
    fn list_input_devices(&self) -> Result<Vec<AudioDevice>, AppError>;

    fn start_recording(
        &self,
        file_path: PathBuf,
        device_id: Option<String>,
    ) -> Result<ActiveCapture, AppError>;
}

pub trait CaptureSessionHandle: Send {
    fn stop(self: Box<Self>) -> Result<CaptureSummary, AppError>;
}

pub struct ActiveCapture {
    pub file_path: PathBuf,
    pub sample_rate_hz: u32,
    pub started_at_ms: u64,
    pub level: LevelMeter,
    pub handle: Box<dyn CaptureSessionHandle>,
}

/// Live input level of an in-flight capture: the RMS of the most recent audio
/// callback buffer, normalised to 0..=1. Stored as f32 bits in an atomic so the
/// realtime audio callback can publish it without locking.
#[derive(Clone, Default)]
pub struct LevelMeter(Arc<AtomicU32>);

impl LevelMeter {
    fn store(&self, level: f32) {
        self.0.store(level.to_bits(), Ordering::Relaxed);
    }

    /// A weak view for readers that must not keep the capture alive: `read`
    /// returns `None` once the capture and its stream are gone, which is how
    /// the dictation level emitter thread knows to exit.
    pub fn observer(&self) -> LevelObserver {
        LevelObserver(Arc::downgrade(&self.0))
    }
}

pub struct LevelObserver(Weak<AtomicU32>);

impl LevelObserver {
    pub fn read(&self) -> Option<f32> {
        self.0
            .upgrade()
            .map(|bits| f32::from_bits(bits.load(Ordering::Relaxed)))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptureSummary {
    pub sample_count: u64,
    pub dropped_sample_count: u64,
    pub stream_error: Option<String>,
}

#[derive(Default)]
pub struct CpalCaptureBackend;

impl CpalCaptureBackend {
    pub fn new() -> Self {
        Self
    }
}

impl AudioCaptureBackend for CpalCaptureBackend {
    fn list_input_devices(&self) -> Result<Vec<AudioDevice>, AppError> {
        let host = cpal::default_host();
        let default_id = host
            .default_input_device()
            .and_then(|device| device.id().ok())
            .map(|id| id.to_string());
        let devices = host.input_devices().map_err(map_cpal_devices_error)?;

        devices
            .map(|device| {
                let id = device.id().map_err(map_cpal_device_id_error)?.to_string();
                let name = device
                    .description()
                    .map_err(map_cpal_device_name_error)?
                    .name()
                    .to_string();
                Ok(AudioDevice {
                    is_default_input: default_id.as_deref() == Some(id.as_str()),
                    id,
                    name,
                })
            })
            .collect()
    }

    fn start_recording(
        &self,
        file_path: PathBuf,
        device_id: Option<String>,
    ) -> Result<ActiveCapture, AppError> {
        let host = cpal::default_host();
        let device = select_input_device(&host, device_id.as_deref())?;
        let supported_config = device
            .default_input_config()
            .map_err(map_cpal_config_error)?;
        let sample_format = supported_config.sample_format();
        let config: cpal::StreamConfig = supported_config.config();
        let sample_rate_hz = config.sample_rate;
        if sample_rate_hz == 0 {
            return Err(audio_error(
                "audio_input_config_invalid",
                "Microphone input sample rate must be greater than zero.",
                None,
            ));
        }
        let channels = usize::from(config.channels);
        let (sample_sender, sample_receiver) = sync_channel(AUDIO_SAMPLE_CHANNEL_CAPACITY);
        let level = LevelMeter::default();
        let dropped_sample_count = Arc::new(AtomicU64::new(0));
        let stream_error = Arc::new(Mutex::new(None));
        let err_callback = {
            let stream_error = Arc::clone(&stream_error);
            move |error: cpal::StreamError| {
                if let Ok(mut slot) = stream_error.lock() {
                    *slot = Some(error.to_string());
                }
            }
        };

        let stream = match sample_format {
            cpal::SampleFormat::I16 => build_i16_stream(
                &device,
                &config,
                channels,
                sample_sender,
                level.clone(),
                Arc::clone(&dropped_sample_count),
                err_callback,
            ),
            cpal::SampleFormat::U16 => build_u16_stream(
                &device,
                &config,
                channels,
                sample_sender,
                level.clone(),
                Arc::clone(&dropped_sample_count),
                err_callback,
            ),
            cpal::SampleFormat::F32 => build_f32_stream(
                &device,
                &config,
                channels,
                sample_sender,
                level.clone(),
                Arc::clone(&dropped_sample_count),
                err_callback,
            ),
            unsupported => Err(audio_error(
                "audio_sample_format_unsupported",
                "Default microphone sample format is not supported yet.",
                Some(format!("sample_format={unsupported:?}")),
            )),
        }?;

        let writer = spawn_i16_mono_wav_writer(file_path.clone(), sample_rate_hz, sample_receiver);
        if let Err(error) = stream.play() {
            drop(stream);
            writer.join()?;
            return Err(map_cpal_play_error(error));
        }

        Ok(ActiveCapture {
            file_path,
            sample_rate_hz,
            started_at_ms: current_time_ms()?,
            level,
            handle: Box::new(CpalCaptureSession {
                stream,
                writer,
                dropped_sample_count,
                stream_error,
            }),
        })
    }
}

struct CpalCaptureSession {
    stream: cpal::Stream,
    writer: WavWriterHandle,
    dropped_sample_count: Arc<AtomicU64>,
    stream_error: Arc<Mutex<Option<String>>>,
}

impl CaptureSessionHandle for CpalCaptureSession {
    fn stop(self: Box<Self>) -> Result<CaptureSummary, AppError> {
        let Self {
            stream,
            writer,
            dropped_sample_count,
            stream_error,
        } = *self;
        drop(stream);

        let writer_summary = writer.join()?;
        let stream_error = stream_error.lock().map_err(|error| {
            audio_error(
                "audio_state_lock_failed",
                "Could not read microphone stream error state.",
                Some(error.to_string()),
            )
        })?;

        Ok(CaptureSummary {
            sample_count: writer_summary.sample_count,
            dropped_sample_count: dropped_sample_count.load(Ordering::Relaxed),
            stream_error: stream_error.clone(),
        })
    }
}

fn build_i16_stream(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    channels: usize,
    sender: SyncSender<i16>,
    level: LevelMeter,
    dropped_sample_count: Arc<AtomicU64>,
    err_callback: impl FnMut(cpal::StreamError) + Send + 'static,
) -> Result<cpal::Stream, AppError> {
    device
        .build_input_stream(
            config,
            move |data: &[i16], _| {
                level.store(rms_of_normalized(
                    data.iter()
                        .map(|sample| f64::from(*sample) / f64::from(i16::MAX)),
                ));
                enqueue_i16_samples(data, channels, &sender, &dropped_sample_count)
            },
            err_callback,
            None,
        )
        .map_err(map_cpal_build_error)
}

fn build_u16_stream(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    channels: usize,
    sender: SyncSender<i16>,
    level: LevelMeter,
    dropped_sample_count: Arc<AtomicU64>,
    err_callback: impl FnMut(cpal::StreamError) + Send + 'static,
) -> Result<cpal::Stream, AppError> {
    device
        .build_input_stream(
            config,
            move |data: &[u16], _| {
                level.store(rms_of_normalized(data.iter().map(|sample| {
                    (f64::from(*sample) - 32_768.0) / f64::from(i16::MAX)
                })));
                enqueue_u16_samples(data, channels, &sender, &dropped_sample_count)
            },
            err_callback,
            None,
        )
        .map_err(map_cpal_build_error)
}

fn build_f32_stream(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    channels: usize,
    sender: SyncSender<i16>,
    level: LevelMeter,
    dropped_sample_count: Arc<AtomicU64>,
    err_callback: impl FnMut(cpal::StreamError) + Send + 'static,
) -> Result<cpal::Stream, AppError> {
    device
        .build_input_stream(
            config,
            move |data: &[f32], _| {
                level.store(rms_of_normalized(
                    data.iter().map(|sample| f64::from(*sample)),
                ));
                enqueue_f32_samples(data, channels, &sender, &dropped_sample_count)
            },
            err_callback,
            None,
        )
        .map_err(map_cpal_build_error)
}

/// RMS of a buffer of samples already normalised to -1..=1, clamped to 0..=1.
/// An empty buffer reads as silence.
fn rms_of_normalized(samples: impl Iterator<Item = f64>) -> f32 {
    let mut sum_of_squares = 0.0_f64;
    let mut count = 0_u64;
    for sample in samples {
        sum_of_squares += sample * sample;
        count += 1;
    }
    if count == 0 {
        return 0.0;
    }
    ((sum_of_squares / count as f64).sqrt().clamp(0.0, 1.0)) as f32
}

fn enqueue_i16_samples(
    samples: &[i16],
    channels: usize,
    sender: &SyncSender<i16>,
    dropped_sample_count: &AtomicU64,
) {
    for frame in samples.chunks(channels.max(1)) {
        let sum = frame.iter().map(|sample| i64::from(*sample)).sum::<i64>();
        enqueue_sample(
            (sum / frame.len() as i64) as i16,
            sender,
            dropped_sample_count,
        );
    }
}

fn enqueue_u16_samples(
    samples: &[u16],
    channels: usize,
    sender: &SyncSender<i16>,
    dropped_sample_count: &AtomicU64,
) {
    for frame in samples.chunks(channels.max(1)) {
        let sum = frame
            .iter()
            .map(|sample| i64::from(*sample) - 32_768)
            .sum::<i64>();
        enqueue_sample(
            (sum / frame.len() as i64) as i16,
            sender,
            dropped_sample_count,
        );
    }
}

fn enqueue_f32_samples(
    samples: &[f32],
    channels: usize,
    sender: &SyncSender<i16>,
    dropped_sample_count: &AtomicU64,
) {
    for frame in samples.chunks(channels.max(1)) {
        let average = frame.iter().copied().sum::<f32>() / frame.len() as f32;
        let sample = (average.clamp(-1.0, 1.0) * f32::from(i16::MAX)).round() as i16;
        enqueue_sample(sample, sender, dropped_sample_count);
    }
}

fn enqueue_sample(sample: i16, sender: &SyncSender<i16>, dropped_sample_count: &AtomicU64) {
    match sender.try_send(sample) {
        Ok(()) => {}
        Err(TrySendError::Full(_)) | Err(TrySendError::Disconnected(_)) => {
            dropped_sample_count.fetch_add(1, Ordering::Relaxed);
        }
    }
}

fn select_input_device(
    host: &cpal::Host,
    device_id: Option<&str>,
) -> Result<cpal::Device, AppError> {
    match device_id {
        None => {
            #[cfg(target_os = "macos")]
            if let Some(device) = bluetooth_avoiding_input_device(host) {
                return Ok(device);
            }
            host.default_input_device().ok_or_else(|| {
                audio_error(
                    "audio_input_device_not_found",
                    "No default microphone input device is available.",
                    None,
                )
            })
        }
        Some(id) => cpal::DeviceId::from_str(id)
            .map_err(map_cpal_device_id_error)
            .and_then(|device_id| {
                host.device_by_id(&device_id).ok_or_else(|| {
                    audio_error(
                        "audio_input_device_not_found",
                        "Requested microphone input device was not found.",
                        Some(format!("device_id={id}")),
                    )
                })
            }),
    }
}

/// Picks the built-in microphone when the OS default input is a Bluetooth
/// hands-free microphone and no device was chosen explicitly. Bluetooth
/// hands-free input is 16 kHz telephone quality and drags the headset out of
/// its high-quality playback profile; the built-in microphone hears the same
/// voice from the room at full quality. An explicit device selection in
/// settings bypasses this entirely.
#[cfg(target_os = "macos")]
fn bluetooth_avoiding_input_device(host: &cpal::Host) -> Option<cpal::Device> {
    let uid = macos_transport::builtin_input_uid_when_default_is_bluetooth()?;
    let device = host.device_by_id(&cpal::DeviceId(cpal::HostId::CoreAudio, uid))?;
    if let Ok(description) = device.description() {
        eprintln!(
            "Default input is a Bluetooth hands-free microphone; recording from \"{}\" instead.",
            description.name()
        );
    }
    Some(device)
}

/// Minimal CoreAudio bindings to read device transport types, which cpal does
/// not expose. Everything degrades to `None` (keep the default device) when a
/// call fails.
#[cfg(target_os = "macos")]
mod macos_transport {
    use std::ffi::{c_char, c_void};

    #[repr(C)]
    struct AudioObjectPropertyAddress {
        selector: u32,
        scope: u32,
        element: u32,
    }

    const SYSTEM_OBJECT: u32 = 1;
    const SCOPE_GLOBAL: u32 = u32::from_be_bytes(*b"glob");
    const SCOPE_INPUT: u32 = u32::from_be_bytes(*b"inpt");
    const ELEMENT_MAIN: u32 = 0;
    const SELECTOR_DEFAULT_INPUT_DEVICE: u32 = u32::from_be_bytes(*b"dIn ");
    const SELECTOR_DEVICES: u32 = u32::from_be_bytes(*b"dev#");
    const SELECTOR_TRANSPORT_TYPE: u32 = u32::from_be_bytes(*b"tran");
    const SELECTOR_STREAMS: u32 = u32::from_be_bytes(*b"stm#");
    const SELECTOR_DEVICE_UID: u32 = u32::from_be_bytes(*b"uid ");
    const TRANSPORT_BUILT_IN: u32 = u32::from_be_bytes(*b"bltn");
    const TRANSPORT_BLUETOOTH: u32 = u32::from_be_bytes(*b"blue");
    const TRANSPORT_BLUETOOTH_LE: u32 = u32::from_be_bytes(*b"blea");
    const CF_STRING_ENCODING_UTF8: u32 = 0x0800_0100;
    const UID_BUFFER_LEN: usize = 512;

    // No #[link] attributes: CoreAudio and CoreFoundation are already linked
    // by cpal and the Tauri frameworks, so the symbols resolve without adding
    // them to the link line again.
    unsafe extern "C" {
        fn AudioObjectGetPropertyData(
            object_id: u32,
            address: *const AudioObjectPropertyAddress,
            qualifier_data_size: u32,
            qualifier_data: *const c_void,
            data_size: *mut u32,
            data: *mut c_void,
        ) -> i32;
        fn AudioObjectGetPropertyDataSize(
            object_id: u32,
            address: *const AudioObjectPropertyAddress,
            qualifier_data_size: u32,
            qualifier_data: *const c_void,
            data_size: *mut u32,
        ) -> i32;
    }

    unsafe extern "C" {
        fn CFStringGetCString(
            string: *const c_void,
            buffer: *mut c_char,
            buffer_size: isize,
            encoding: u32,
        ) -> u8;
        fn CFRelease(object: *const c_void);
    }

    /// UID of the built-in input device, but only when the current default
    /// input is a Bluetooth microphone. `None` means: keep the default.
    pub fn builtin_input_uid_when_default_is_bluetooth() -> Option<String> {
        let default_device = read_u32(SYSTEM_OBJECT, SELECTOR_DEFAULT_INPUT_DEVICE)?;
        if default_device == 0 {
            return None;
        }
        let transport = read_u32(default_device, SELECTOR_TRANSPORT_TYPE)?;
        if transport != TRANSPORT_BLUETOOTH && transport != TRANSPORT_BLUETOOTH_LE {
            return None;
        }

        all_device_ids()
            .into_iter()
            .filter(|device| has_input_streams(*device))
            .find(|device| read_u32(*device, SELECTOR_TRANSPORT_TYPE) == Some(TRANSPORT_BUILT_IN))
            .and_then(device_uid)
    }

    fn global_address(selector: u32) -> AudioObjectPropertyAddress {
        AudioObjectPropertyAddress {
            selector,
            scope: SCOPE_GLOBAL,
            element: ELEMENT_MAIN,
        }
    }

    fn read_u32(object_id: u32, selector: u32) -> Option<u32> {
        let address = global_address(selector);
        let mut value = 0_u32;
        let mut size = std::mem::size_of::<u32>() as u32;
        let status = unsafe {
            AudioObjectGetPropertyData(
                object_id,
                &address,
                0,
                std::ptr::null(),
                &mut size,
                (&mut value as *mut u32).cast::<c_void>(),
            )
        };
        (status == 0 && size == std::mem::size_of::<u32>() as u32).then_some(value)
    }

    fn all_device_ids() -> Vec<u32> {
        let address = global_address(SELECTOR_DEVICES);
        let mut size = 0_u32;
        let status = unsafe {
            AudioObjectGetPropertyDataSize(SYSTEM_OBJECT, &address, 0, std::ptr::null(), &mut size)
        };
        if status != 0 || size == 0 {
            return Vec::new();
        }

        let mut devices = vec![0_u32; size as usize / std::mem::size_of::<u32>()];
        let status = unsafe {
            AudioObjectGetPropertyData(
                SYSTEM_OBJECT,
                &address,
                0,
                std::ptr::null(),
                &mut size,
                devices.as_mut_ptr().cast::<c_void>(),
            )
        };
        if status != 0 {
            return Vec::new();
        }
        devices.truncate(size as usize / std::mem::size_of::<u32>());
        devices
    }

    fn has_input_streams(device_id: u32) -> bool {
        let address = AudioObjectPropertyAddress {
            selector: SELECTOR_STREAMS,
            scope: SCOPE_INPUT,
            element: ELEMENT_MAIN,
        };
        let mut size = 0_u32;
        let status = unsafe {
            AudioObjectGetPropertyDataSize(device_id, &address, 0, std::ptr::null(), &mut size)
        };
        status == 0 && size > 0
    }

    fn device_uid(device_id: u32) -> Option<String> {
        let address = global_address(SELECTOR_DEVICE_UID);
        let mut cf_string: *const c_void = std::ptr::null();
        let mut size = std::mem::size_of::<*const c_void>() as u32;
        let status = unsafe {
            AudioObjectGetPropertyData(
                device_id,
                &address,
                0,
                std::ptr::null(),
                &mut size,
                (&mut cf_string as *mut *const c_void).cast::<c_void>(),
            )
        };
        if status != 0 || cf_string.is_null() {
            return None;
        }

        let mut buffer = vec![0_u8; UID_BUFFER_LEN];
        let copied = unsafe {
            CFStringGetCString(
                cf_string,
                buffer.as_mut_ptr().cast::<c_char>(),
                buffer.len() as isize,
                CF_STRING_ENCODING_UTF8,
            )
        };
        unsafe { CFRelease(cf_string) };
        if copied == 0 {
            return None;
        }

        let terminator = buffer.iter().position(|byte| *byte == 0)?;
        buffer.truncate(terminator);
        String::from_utf8(buffer).ok()
    }
}

pub(crate) fn current_time_ms() -> Result<u64, AppError> {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .map_err(|error| {
            audio_error(
                "system_time_error",
                "System clock is before the Unix epoch.",
                Some(error.to_string()),
            )
        })
}

fn map_cpal_devices_error(error: cpal::DevicesError) -> AppError {
    audio_error(
        "audio_devices_unavailable",
        "Could not enumerate microphone input devices.",
        Some(error.to_string()),
    )
}

fn map_cpal_device_name_error(error: cpal::DeviceNameError) -> AppError {
    audio_error(
        "audio_device_name_unavailable",
        "Could not read microphone input device name.",
        Some(error.to_string()),
    )
}

fn map_cpal_device_id_error(error: cpal::DeviceIdError) -> AppError {
    audio_error(
        "invalid_audio_device_id",
        "Audio device id is not valid.",
        Some(error.to_string()),
    )
}

fn map_cpal_config_error(error: cpal::DefaultStreamConfigError) -> AppError {
    audio_error(
        "audio_input_config_unavailable",
        "Could not read the default microphone input configuration.",
        Some(error.to_string()),
    )
}

fn map_cpal_build_error(error: cpal::BuildStreamError) -> AppError {
    audio_error(
        "audio_stream_build_failed",
        "Could not build the microphone input stream.",
        Some(error.to_string()),
    )
}

fn map_cpal_play_error(error: cpal::PlayStreamError) -> AppError {
    audio_error(
        "audio_stream_start_failed",
        "Could not start the microphone input stream.",
        Some(error.to_string()),
    )
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc::sync_channel;

    use super::*;

    #[test]
    fn enqueue_samples_drops_when_bounded_channel_is_full() {
        let (sender, receiver) = sync_channel(1);
        let dropped_sample_count = AtomicU64::new(0);

        enqueue_i16_samples(&[10, 20, 30], 1, &sender, &dropped_sample_count);

        assert_eq!(receiver.try_recv().expect("first sample is retained"), 10);
        assert_eq!(dropped_sample_count.load(Ordering::Relaxed), 2);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn bluetooth_avoidance_returns_resolvable_builtin_device_or_none() {
        use cpal::traits::{DeviceTrait, HostTrait};

        // None is the correct answer whenever the default input is not a
        // Bluetooth microphone; when it is, the returned UID must resolve to
        // a real capture device so recording cannot fail on the swap.
        let Some(uid) = macos_transport::builtin_input_uid_when_default_is_bluetooth() else {
            return;
        };

        let host = cpal::default_host();
        let device = host
            .device_by_id(&cpal::DeviceId(cpal::HostId::CoreAudio, uid))
            .expect("built-in microphone UID resolves to a cpal device");
        let description = device.description().expect("device has a description");
        println!(
            "bluetooth default input detected; would record from: {}",
            description.name()
        );
        assert!(device.default_input_config().is_ok());
    }

    #[test]
    fn enqueue_samples_downmixes_stereo_to_mono() {
        let (sender, receiver) = sync_channel(2);
        let dropped_sample_count = AtomicU64::new(0);

        enqueue_i16_samples(&[10, 30, -20, 20], 2, &sender, &dropped_sample_count);

        assert_eq!(receiver.try_recv().expect("first mono sample"), 20);
        assert_eq!(receiver.try_recv().expect("second mono sample"), 0);
        assert_eq!(dropped_sample_count.load(Ordering::Relaxed), 0);
    }
}
