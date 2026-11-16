//! System and microphone audio through WASAPI, delivered to FFmpeg as raw PCM
//! over a named pipe. Every stream is converted to 48 kHz stereo float by the
//! Windows audio engine so FFmpeg always receives the same format.
//!
//! Loopback capture only yields packets while something is playing. The
//! capture loop therefore pads silence by the performance counter so that the
//! sample position always follows wall-clock time and stays aligned with video.
use crate::capture::AudioInput;
use std::{
    sync::{
        atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering},
        mpsc, Arc,
    },
    thread::JoinHandle,
    time::Duration,
};

pub const SAMPLE_RATE: u32 = 48_000;
pub const CHANNELS: u16 = 2;

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct AudioDevice {
    pub id: String,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AudioSource {
    /// Whatever currently plays on the default output device.
    Loopback,
    /// Includes only the selected process and its children (Windows build 20348+).
    Process { pid: u32 },
    /// A capture endpoint; an empty id means the default microphone.
    Microphone { device_id: String },
}

impl AudioSource {
    pub fn label(&self) -> &'static str {
        match self {
            AudioSource::Loopback => "Systemton",
            AudioSource::Process { .. } => "Spielton",
            AudioSource::Microphone { .. } => "Mikrofon",
        }
    }
}

#[derive(Clone, Default)]
pub struct Meter(Arc<AtomicU32>);
impl Meter {
    fn feed(&self, bytes: &[u8], gain: f32) {
        let peak = bytes
            .chunks_exact(4)
            .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
            .filter(|v| v.is_finite())
            .map(|v| (v.abs() * gain).min(2.0))
            .fold(0.0_f32, f32::max);
        self.0.fetch_max(peak.to_bits(), Ordering::Relaxed);
    }
    pub fn take(&self) -> f32 {
        f32::from_bits(self.0.swap(0, Ordering::Relaxed))
    }
}

static PIPE_SEQUENCE: AtomicUsize = AtomicUsize::new(0);

/// One running capture thread feeding one named pipe.
pub struct AudioPipe {
    pub input: AudioInput,
    pub source: AudioSource,
    pub meter: Meter,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl AudioPipe {
    /// Opens the device and the pipe. Returns once the device is initialized so
    /// FFmpeg is only started for inputs that actually work.
    pub fn start(source: AudioSource, gain: f32) -> Result<AudioPipe, String> {
        let sequence = PIPE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = format!("\\\\.\\pipe\\stillclip-{}-{sequence}", std::process::id());
        let stop = Arc::new(AtomicBool::new(false));
        let (ready_sender, ready_receiver) = mpsc::channel::<Result<(), String>>();
        let meter = Meter::default();
        let thread_meter = meter.clone();
        let thread_source = source.clone();
        let thread_stop = stop.clone();
        let thread_path = path.clone();
        let thread = std::thread::Builder::new()
            .name(format!("audio-{sequence}"))
            .spawn(move || {
                platform::run(
                    thread_source,
                    thread_path,
                    thread_stop,
                    ready_sender,
                    thread_meter,
                    gain,
                );
            })
            .map_err(|error| format!("Der Audio-Thread konnte nicht gestartet werden: {error}"))?;
        let mut pipe = AudioPipe {
            input: AudioInput {
                path,
                sample_format: "f32le",
                rate: SAMPLE_RATE,
                channels: CHANNELS,
                gain,
            },
            source,
            meter,
            stop,
            thread: Some(thread),
        };
        match ready_receiver.recv_timeout(Duration::from_secs(10)) {
            Ok(Ok(())) => Ok(pipe),
            Ok(Err(error)) => {
                pipe.stop();
                Err(error)
            }
            Err(_) => {
                pipe.stop();
                Err("Das Audiogerät hat nicht rechtzeitig geantwortet.".into())
            }
        }
    }

    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        platform::unblock(&self.input.path);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for AudioPipe {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(windows)]
pub fn capture_devices() -> Vec<AudioDevice> {
    platform::capture_devices()
}

#[cfg(not(windows))]
pub fn capture_devices() -> Vec<AudioDevice> {
    Vec::new()
}

#[cfg(windows)]
pub fn default_output_name() -> Option<String> {
    platform::default_output_name()
}

#[cfg(not(windows))]
pub fn default_output_name() -> Option<String> {
    None
}

/// Shared sample bookkeeping: pads or trims so that `written` frames always
/// correspond to the elapsed wall-clock time.
#[derive(Debug, Default)]
pub struct Clock {
    pub written: u64,
}

impl Clock {
    /// Frames of silence to insert before a packet captured at `desired`.
    pub fn padding_before(&self, desired: u64, rate: u64) -> u64 {
        if desired > self.written + rate / 20 {
            desired - self.written
        } else {
            0
        }
    }
    /// Frames to drop from the front of a packet when the stream ran ahead.
    pub fn excess_before(&self, desired: u64, rate: u64) -> u64 {
        if self.written > desired + rate / 10 {
            self.written - desired
        } else {
            0
        }
    }
    /// Silence to append while the device delivers nothing.
    pub fn idle_padding(&self, expected: u64, rate: u64) -> u64 {
        if expected > self.written + rate / 10 {
            expected - self.written - rate / 40
        } else {
            0
        }
    }
}

#[cfg(windows)]
mod platform {
    use super::{AudioDevice, AudioSource, Clock, CHANNELS, SAMPLE_RATE};
    use std::{
        fs::File,
        io::Write,
        os::windows::io::FromRawHandle,
        sync::{atomic::AtomicBool, atomic::Ordering, mpsc::Sender, Arc},
        time::Duration,
    };
    use windows::{
        core::{GUID, PCWSTR, PWSTR},
        Win32::{
            Devices::FunctionDiscovery::PKEY_Device_FriendlyName,
            Media::Audio::{
                eCapture, eConsole, eRender, IAudioCaptureClient, IAudioClient, IMMDevice,
                IMMDeviceEnumerator, MMDeviceEnumerator, AUDCLNT_BUFFERFLAGS_SILENT,
                AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM,
                AUDCLNT_STREAMFLAGS_LOOPBACK, AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY,
                DEVICE_STATE_ACTIVE, WAVEFORMATEX, WAVEFORMATEXTENSIBLE, WAVEFORMATEXTENSIBLE_0,
            },
            System::{
                Com::{
                    CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_ALL,
                    COINIT_MULTITHREADED, STGM_READ,
                },
                Performance::{QueryPerformanceCounter, QueryPerformanceFrequency},
                Variant::VT_LPWSTR,
            },
        },
    };
    use windows_sys::Win32::{
        Foundation::{CloseHandle, ERROR_PIPE_CONNECTED, GENERIC_READ, INVALID_HANDLE_VALUE},
        Storage::FileSystem::{CreateFileW, FILE_SHARE_READ, OPEN_EXISTING, PIPE_ACCESS_OUTBOUND},
        System::Pipes::{
            ConnectNamedPipe, CreateNamedPipeW, PIPE_READMODE_BYTE, PIPE_TYPE_BYTE, PIPE_WAIT,
        },
    };

    const KSDATAFORMAT_SUBTYPE_IEEE_FLOAT: GUID =
        GUID::from_u128(0x00000003_0000_0010_8000_00aa00389b71);
    const WAVE_FORMAT_EXTENSIBLE: u16 = 0xFFFE;

    fn wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(Some(0)).collect()
    }

    struct Com;
    impl Com {
        fn init() -> Self {
            unsafe {
                let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            }
            Com
        }
    }
    impl Drop for Com {
        fn drop(&mut self) {
            unsafe { CoUninitialize() };
        }
    }

    fn pwstr_to_string(value: PWSTR) -> String {
        if value.is_null() {
            return String::new();
        }
        let text = unsafe { value.to_string() }.unwrap_or_default();
        text
    }

    fn device_name(device: &IMMDevice) -> String {
        unsafe {
            let store = match device.OpenPropertyStore(STGM_READ) {
                Ok(store) => store,
                Err(_) => return String::new(),
            };
            let value = match store.GetValue(&PKEY_Device_FriendlyName) {
                Ok(value) => value,
                Err(_) => return String::new(),
            };
            let inner = &value.Anonymous.Anonymous;
            if inner.vt == VT_LPWSTR {
                inner.Anonymous.pwszVal.to_string().unwrap_or_default()
            } else {
                String::new()
            }
        }
    }

    fn device_id(device: &IMMDevice) -> String {
        unsafe {
            match device.GetId() {
                Ok(id) => {
                    let text = pwstr_to_string(id);
                    CoTaskMemFree(Some(id.0 as *const _));
                    text
                }
                Err(_) => String::new(),
            }
        }
    }

    fn enumerator() -> Result<IMMDeviceEnumerator, String> {
        unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) }
            .map_err(|error| format!("Die Audiogeräte sind nicht verfügbar: {error}"))
    }

    pub fn capture_devices() -> Vec<AudioDevice> {
        let _com = Com::init();
        let mut devices = Vec::new();
        let Ok(enumerator) = enumerator() else {
            return devices;
        };
        unsafe {
            if let Ok(collection) = enumerator.EnumAudioEndpoints(eCapture, DEVICE_STATE_ACTIVE) {
                let count = collection.GetCount().unwrap_or(0);
                for index in 0..count {
                    if let Ok(device) = collection.Item(index) {
                        let id = device_id(&device);
                        let name = device_name(&device);
                        if !id.is_empty() {
                            devices.push(AudioDevice {
                                id,
                                name: if name.is_empty() {
                                    "Mikrofon".into()
                                } else {
                                    name
                                },
                            });
                        }
                    }
                }
            }
        }
        devices
    }

    pub fn default_output_name() -> Option<String> {
        let _com = Com::init();
        let enumerator = enumerator().ok()?;
        let device = unsafe { enumerator.GetDefaultAudioEndpoint(eRender, eConsole) }.ok()?;
        let name = device_name(&device);
        (!name.is_empty()).then_some(name)
    }

    fn requested_format() -> WAVEFORMATEXTENSIBLE {
        let block_align = CHANNELS * 4;
        WAVEFORMATEXTENSIBLE {
            Format: WAVEFORMATEX {
                wFormatTag: WAVE_FORMAT_EXTENSIBLE,
                nChannels: CHANNELS,
                nSamplesPerSec: SAMPLE_RATE,
                nAvgBytesPerSec: SAMPLE_RATE * u32::from(block_align),
                nBlockAlign: block_align,
                wBitsPerSample: 32,
                cbSize: 22,
            },
            Samples: WAVEFORMATEXTENSIBLE_0 {
                wValidBitsPerSample: 32,
            },
            dwChannelMask: 0x3,
            SubFormat: KSDATAFORMAT_SUBTYPE_IEEE_FLOAT,
        }
    }

    fn qpc_100ns(frequency: i64) -> i64 {
        let mut counter = 0_i64;
        unsafe {
            let _ = QueryPerformanceCounter(&mut counter);
        }
        ((counter as i128) * 10_000_000 / (frequency.max(1) as i128)) as i64
    }

    /// Opens the pipe server end. FFmpeg connects as the client.
    fn create_pipe(path: &str) -> Result<File, String> {
        let name = wide(path);
        let handle = unsafe {
            CreateNamedPipeW(
                name.as_ptr(),
                PIPE_ACCESS_OUTBOUND,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
                1,
                1 << 20,
                0,
                0,
                std::ptr::null(),
            )
        };
        if handle == INVALID_HANDLE_VALUE || handle.is_null() {
            return Err(format!(
                "Die Audio-Pipe konnte nicht erstellt werden (Windows-Fehler {}).",
                std::io::Error::last_os_error()
            ));
        }
        Ok(unsafe { File::from_raw_handle(handle as _) })
    }

    /// Connects as a client so a waiting `ConnectNamedPipe` returns.
    pub fn unblock(path: &str) {
        let name = wide(path);
        let handle = unsafe {
            CreateFileW(
                name.as_ptr(),
                GENERIC_READ,
                FILE_SHARE_READ,
                std::ptr::null(),
                OPEN_EXISTING,
                0,
                std::ptr::null_mut(),
            )
        };
        if handle != INVALID_HANDLE_VALUE && !handle.is_null() {
            unsafe {
                CloseHandle(handle);
            }
        }
    }

    struct Stream {
        client: IAudioClient,
        capture: IAudioCaptureClient,
        device_id: String,
    }

    #[windows::core::implement(
        windows::Win32::Media::Audio::IActivateAudioInterfaceCompletionHandler
    )]
    struct ActivationComplete {
        ready: std::sync::mpsc::Sender<()>,
        _params: Box<windows::Win32::Media::Audio::AUDIOCLIENT_ACTIVATION_PARAMS>,
    }

    impl windows::Win32::Media::Audio::IActivateAudioInterfaceCompletionHandler_Impl
        for ActivationComplete_Impl
    {
        fn ActivateCompleted(
            &self,
            _: windows::core::Ref<
                '_,
                windows::Win32::Media::Audio::IActivateAudioInterfaceAsyncOperation,
            >,
        ) -> windows::core::Result<()> {
            let _ = self.ready.send(());
            Ok(())
        }
    }

    fn process_client(pid: u32) -> Result<IAudioClient, String> {
        use windows::{
            core::{Interface, HRESULT},
            Win32::{
                Media::Audio::*,
                System::{
                    Com::{StructuredStorage::PROPVARIANT, BLOB},
                    Variant::VT_BLOB,
                },
            },
        };
        let params = Box::new(AUDIOCLIENT_ACTIVATION_PARAMS {
            ActivationType: AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK,
            Anonymous: AUDIOCLIENT_ACTIVATION_PARAMS_0 {
                ProcessLoopbackParams: AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS {
                    TargetProcessId: pid,
                    ProcessLoopbackMode: PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE,
                },
            },
        });
        // This BLOB borrows the callback's Box. PROPVARIANT normally frees BLOB
        // data with CoTaskMemFree, so its destructor must not own this pointer.
        let mut variant = std::mem::ManuallyDrop::new(PROPVARIANT::default());
        unsafe {
            let value = &mut *variant.Anonymous.Anonymous;
            value.vt = VT_BLOB;
            value.Anonymous.blob = BLOB {
                cbSize: std::mem::size_of::<AUDIOCLIENT_ACTIVATION_PARAMS>() as u32,
                pBlobData: (&*params as *const AUDIOCLIENT_ACTIVATION_PARAMS)
                    .cast_mut()
                    .cast(),
            };
        }
        let (tx, rx) = std::sync::mpsc::channel();
        let callback: IActivateAudioInterfaceCompletionHandler = ActivationComplete {
            ready: tx,
            _params: params,
        }
        .into();
        let operation = unsafe {
            ActivateAudioInterfaceAsync(
                VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK,
                &IAudioClient::IID,
                Some(&*variant),
                &callback,
            )
        }
        .map_err(|e| format!("Spielton benötigt Windows 11 bzw. Build 20348 oder neuer: {e}"))?;
        rx.recv_timeout(Duration::from_secs(8)).map_err(|_| {
            "Windows hat die Spielton-Aufnahme nicht rechtzeitig aktiviert.".to_string()
        })?;
        let mut status = HRESULT::default();
        let mut result = None;
        unsafe { operation.GetActivateResult(&mut status, &mut result) }
            .map_err(|e| e.to_string())?;
        status.ok().map_err(|e| format!("Spielton ist für diesen Prozess nicht verfügbar (Windows Build 20348+ erforderlich): {e}"))?;
        result
            .ok_or("Windows hat keine Spielton-Schnittstelle geliefert.")?
            .cast()
            .map_err(|e| e.to_string())
    }

    /// The endpoint id a source should currently use, when it follows the default.
    fn current_default_id(
        enumerator: &IMMDeviceEnumerator,
        source: &AudioSource,
    ) -> Option<String> {
        let device = unsafe {
            match source {
                AudioSource::Loopback => {
                    enumerator.GetDefaultAudioEndpoint(eRender, eConsole).ok()?
                }
                AudioSource::Microphone { device_id } if device_id.is_empty() => enumerator
                    .GetDefaultAudioEndpoint(eCapture, eConsole)
                    .ok()?,
                AudioSource::Microphone { .. } => return None,
                AudioSource::Process { .. } => return None,
            }
        };
        Some(device_id(&device))
    }

    fn open_stream(
        enumerator: &IMMDeviceEnumerator,
        source: &AudioSource,
    ) -> Result<Stream, String> {
        if let AudioSource::Process { pid } = source {
            let client = process_client(*pid)?;
            return initialize_stream(client, source, format!("process:{pid}"));
        }
        let device = unsafe {
            match source {
                AudioSource::Loopback => enumerator.GetDefaultAudioEndpoint(eRender, eConsole),
                AudioSource::Process { .. } => unreachable!(),
                AudioSource::Microphone { device_id } if device_id.is_empty() => {
                    enumerator.GetDefaultAudioEndpoint(eCapture, eConsole)
                }
                AudioSource::Microphone { device_id } => {
                    let id = wide(device_id);
                    enumerator.GetDevice(PCWSTR(id.as_ptr()))
                }
            }
        }
        .map_err(|error| match source {
            AudioSource::Loopback => {
                format!("Kein Wiedergabegerät für den Systemton gefunden: {error}")
            }
            AudioSource::Microphone { .. } => format!("Das Mikrofon wurde nicht gefunden: {error}"),
            AudioSource::Process { .. } => format!("Spielton nicht verfügbar: {error}"),
        })?;
        let client: IAudioClient = unsafe { device.Activate(CLSCTX_ALL, None) }
            .map_err(|error| format!("Das Audiogerät konnte nicht geöffnet werden: {error}"))?;
        initialize_stream(client, source, device_id(&device))
    }

    fn initialize_stream(
        client: IAudioClient,
        source: &AudioSource,
        device_id: String,
    ) -> Result<Stream, String> {
        let format = requested_format();
        let mut flags =
            AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY;
        if matches!(source, AudioSource::Loopback | AudioSource::Process { .. }) {
            flags |= AUDCLNT_STREAMFLAGS_LOOPBACK;
        }
        unsafe {
            client.Initialize(
                AUDCLNT_SHAREMODE_SHARED,
                flags,
                2_000_000,
                0,
                &format.Format as *const WAVEFORMATEX,
                None,
            )
        }
        .map_err(|error| format!("Das Audiogerät unterstützt die Aufnahme nicht: {error}"))?;
        let capture: IAudioCaptureClient = unsafe { client.GetService() }
            .map_err(|error| format!("Die Audioaufnahme ist nicht verfügbar: {error}"))?;
        Ok(Stream {
            client,
            capture,
            device_id,
        })
    }

    pub fn run(
        source: AudioSource,
        path: String,
        stop: Arc<AtomicBool>,
        ready: Sender<Result<(), String>>,
        meter: super::Meter,
        gain: f32,
    ) {
        let _com = Com::init();
        let enumerator = match enumerator() {
            Ok(enumerator) => enumerator,
            Err(error) => {
                let _ = ready.send(Err(error));
                return;
            }
        };
        let mut stream = match open_stream(&enumerator, &source) {
            Ok(stream) => stream,
            Err(error) => {
                let _ = ready.send(Err(error));
                return;
            }
        };
        let mut pipe = match create_pipe(&path) {
            Ok(pipe) => pipe,
            Err(error) => {
                let _ = ready.send(Err(error));
                return;
            }
        };
        let _ = ready.send(Ok(()));
        // Wait for FFmpeg; `unblock` connects a dummy client when stopping early.
        let connected = unsafe { ConnectNamedPipe(pipe_handle(&pipe), std::ptr::null_mut()) };
        if connected == 0
            && std::io::Error::last_os_error().raw_os_error() != Some(ERROR_PIPE_CONNECTED as i32)
        {
            return;
        }
        if stop.load(Ordering::SeqCst) {
            return;
        }
        let mut frequency = 0_i64;
        unsafe {
            let _ = QueryPerformanceFrequency(&mut frequency);
        }
        if unsafe { stream.client.Start() }.is_err() {
            return;
        }
        let start = qpc_100ns(frequency);
        let rate = u64::from(SAMPLE_RATE);
        let frame_bytes = usize::from(CHANNELS) * 4;
        let mut clock = Clock::default();
        let silence = vec![0_u8; frame_bytes * (SAMPLE_RATE as usize / 10)];
        let write_silence = |pipe: &mut File, frames: u64, clock: &mut Clock| -> bool {
            let mut remaining = frames;
            while remaining > 0 {
                let chunk = remaining.min((silence.len() / frame_bytes) as u64);
                if pipe
                    .write_all(&silence[..chunk as usize * frame_bytes])
                    .is_err()
                {
                    return false;
                }
                remaining -= chunk;
                clock.written += chunk;
            }
            true
        };
        let mut ticks: u32 = 0;
        'capture: while !stop.load(Ordering::SeqCst) {
            ticks = ticks.wrapping_add(1);
            // Follow the default device: a headset plugged in mid-session takes over.
            if ticks % 150 == 0 {
                if let Some(default_id) = current_default_id(&enumerator, &source) {
                    if default_id != stream.device_id {
                        if let Ok(next) = open_stream(&enumerator, &source) {
                            if unsafe { next.client.Start() }.is_ok() {
                                let _ = unsafe { stream.client.Stop() };
                                stream = next;
                            }
                        }
                    }
                }
            }
            let mut device_failed = false;
            loop {
                let packet = match unsafe { stream.capture.GetNextPacketSize() } {
                    Ok(size) => size,
                    Err(_) => {
                        device_failed = true;
                        break;
                    }
                };
                if packet == 0 {
                    break;
                }
                let mut data: *mut u8 = std::ptr::null_mut();
                let mut frames = 0_u32;
                let mut flags = 0_u32;
                let mut position = 0_u64;
                if unsafe {
                    stream.capture.GetBuffer(
                        &mut data,
                        &mut frames,
                        &mut flags,
                        None,
                        Some(&mut position),
                    )
                }
                .is_err()
                {
                    device_failed = true;
                    break;
                }
                let desired = ((position as i64 - start).max(0) as u64) * rate / 10_000_000;
                let padding = clock.padding_before(desired, rate);
                if padding > 0 && !write_silence(&mut pipe, padding, &mut clock) {
                    break 'capture;
                }
                let skip = clock.excess_before(desired, rate).min(u64::from(frames)) as usize;
                let keep = frames as usize - skip;
                let written = if flags & (AUDCLNT_BUFFERFLAGS_SILENT.0 as u32) != 0
                    || data.is_null()
                {
                    write_silence(&mut pipe, keep as u64, &mut clock)
                } else {
                    let bytes = unsafe {
                        std::slice::from_raw_parts(data.add(skip * frame_bytes), keep * frame_bytes)
                    };
                    meter.feed(bytes, gain);
                    let ok = pipe.write_all(bytes).is_ok();
                    clock.written += keep as u64;
                    ok
                };
                let _ = unsafe { stream.capture.ReleaseBuffer(frames) };
                if !written {
                    break 'capture;
                }
            }
            if device_failed {
                // The device vanished (unplugged or invalidated): continue on the current default.
                let _ = unsafe { stream.client.Stop() };
                let mut reopened = None;
                for _ in 0..10 {
                    if stop.load(Ordering::SeqCst) {
                        break 'capture;
                    }
                    if let Ok(next) = open_stream(&enumerator, &source) {
                        if unsafe { next.client.Start() }.is_ok() {
                            reopened = Some(next);
                            break;
                        }
                    }
                    let expected =
                        ((qpc_100ns(frequency) - start).max(0) as u64) * rate / 10_000_000;
                    let idle = clock.idle_padding(expected, rate);
                    if idle > 0 && !write_silence(&mut pipe, idle, &mut clock) {
                        break 'capture;
                    }
                    std::thread::sleep(Duration::from_millis(300));
                }
                match reopened {
                    Some(next) => stream = next,
                    None => break 'capture,
                }
            }
            let expected = ((qpc_100ns(frequency) - start).max(0) as u64) * rate / 10_000_000;
            let idle = clock.idle_padding(expected, rate);
            if idle > 0 && !write_silence(&mut pipe, idle, &mut clock) {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        let _ = unsafe { stream.client.Stop() };
    }

    fn pipe_handle(file: &File) -> windows_sys::Win32::Foundation::HANDLE {
        use std::os::windows::io::AsRawHandle;
        file.as_raw_handle() as _
    }
}

#[cfg(not(windows))]
mod platform {
    use super::AudioSource;
    use std::sync::{atomic::AtomicBool, mpsc::Sender, Arc};

    pub fn run(
        _: AudioSource,
        _: String,
        _: Arc<AtomicBool>,
        ready: Sender<Result<(), String>>,
        _: super::Meter,
        _: f32,
    ) {
        let _ = ready.send(Err("Audioaufnahme ist nur unter Windows verfügbar.".into()));
    }
    pub fn unblock(_: &str) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meter_applies_gain_ignores_invalid_samples_and_clears_idle_peak() {
        let meter = Meter::default();
        let bytes: Vec<u8> = [-0.25_f32, 0.75, f32::NAN, f32::INFINITY]
            .into_iter()
            .flat_map(f32::to_le_bytes)
            .collect();
        meter.feed(&bytes, 2.0);
        assert_eq!(meter.take(), 1.5);
        assert_eq!(meter.take(), 0.0);
        meter.feed(&bytes, 0.0);
        assert_eq!(meter.take(), 0.0);
    }

    #[test]
    fn clock_pads_gaps_and_trims_runaway_streams() {
        let rate = 48_000;
        let mut clock = Clock::default();
        assert_eq!(clock.padding_before(1_000, rate), 0);
        assert_eq!(clock.padding_before(10_000, rate), 10_000);
        clock.written = 10_000;
        assert_eq!(clock.excess_before(9_000, rate), 0);
        assert_eq!(clock.excess_before(4_000, rate), 6_000);
        assert_eq!(clock.idle_padding(12_000, rate), 0);
        assert_eq!(clock.idle_padding(20_000, rate), 10_000 - rate / 40);
    }
}
