//! Media Foundation lifetime, hardware H.264 discovery, and encode probing.

#![allow(unsafe_code)]

use std::{
    ffi::c_void,
    mem::ManuallyDrop,
    ptr::NonNull,
    thread,
    time::{Duration, Instant},
};

use ::windows::{
    Win32::{
        Graphics::{Direct3D::D3D_DRIVER_TYPE_HARDWARE, Direct3D11::ID3D11Device},
        Media::MediaFoundation::{
            IMFActivate, IMFDXGIDeviceManager, IMFMediaBuffer, IMFMediaEventGenerator, IMFSample,
            IMFTransform, METransformDrainComplete, METransformHaveOutput, METransformNeedInput,
            MF_E_NO_EVENTS_AVAILABLE, MF_E_TRANSFORM_NEED_MORE_INPUT, MF_E_TRANSFORM_STREAM_CHANGE,
            MF_EVENT_FLAG_NO_WAIT, MF_MT_ALL_SAMPLES_INDEPENDENT, MF_MT_AVG_BITRATE,
            MF_MT_FIXED_SIZE_SAMPLES, MF_MT_FRAME_RATE, MF_MT_FRAME_SIZE, MF_MT_INTERLACE_MODE,
            MF_MT_MAJOR_TYPE, MF_MT_MPEG2_PROFILE, MF_MT_PIXEL_ASPECT_RATIO, MF_MT_SAMPLE_SIZE,
            MF_MT_SUBTYPE, MF_SA_D3D11_AWARE, MF_TRANSFORM_ASYNC, MF_TRANSFORM_ASYNC_UNLOCK,
            MF_VERSION, MFCreateDXGIDeviceManager, MFCreateMediaType, MFCreateMemoryBuffer,
            MFCreateSample, MFMediaType_Video, MFSTARTUP_FULL, MFSampleExtension_CleanPoint,
            MFShutdown, MFStartup, MFT_CATEGORY_VIDEO_ENCODER, MFT_ENUM_FLAG_HARDWARE,
            MFT_ENUM_FLAG_SORTANDFILTER, MFT_ENUM_HARDWARE_URL_Attribute,
            MFT_FRIENDLY_NAME_Attribute, MFT_MESSAGE_COMMAND_DRAIN, MFT_MESSAGE_COMMAND_FLUSH,
            MFT_MESSAGE_NOTIFY_BEGIN_STREAMING, MFT_MESSAGE_NOTIFY_END_OF_STREAM,
            MFT_MESSAGE_NOTIFY_END_STREAMING, MFT_MESSAGE_NOTIFY_START_OF_STREAM,
            MFT_MESSAGE_SET_D3D_MANAGER, MFT_OUTPUT_DATA_BUFFER,
            MFT_OUTPUT_STREAM_CAN_PROVIDE_SAMPLES, MFT_OUTPUT_STREAM_INFO,
            MFT_OUTPUT_STREAM_PROVIDES_SAMPLES, MFT_REGISTER_TYPE_INFO,
            MFT_TRANSFORM_CLSID_Attribute, MFTEnumEx, MFVideoFormat_H264, MFVideoFormat_NV12,
            MFVideoInterlace_Progressive, eAVEncH264VProfile_Main,
        },
        System::Com::CoTaskMemFree,
    },
    core::{GUID, Interface, PWSTR},
};

const PROBE_WIDTH: u32 = 640;
const PROBE_HEIGHT: u32 = 360;
const PROBE_FPS: u32 = 30;
const PROBE_BITRATE: u32 = 2_000_000;
const PROBE_FRAME_COUNT: u32 = 8;
const PROBE_TIMEOUT: Duration = Duration::from_secs(5);
const HUNDRED_NANOSECONDS_PER_SECOND: i64 = 10_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct H264EncoderConfig {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub bitrate: u32,
}

impl H264EncoderConfig {
    pub const fn new(width: u32, height: u32, fps: u32, bitrate: u32) -> Self {
        Self {
            width,
            height,
            fps,
            bitrate,
        }
    }

    fn validate(self) -> Result<Self, String> {
        nv12_sample_size(self.width, self.height)?;
        if self.fps == 0 {
            return Err("H.264 frame rate must be non-zero".to_owned());
        }
        if self.bitrate == 0 {
            return Err("H.264 bitrate must be non-zero".to_owned());
        }
        Ok(self)
    }

    fn frame_duration_100ns(self) -> i64 {
        HUNDRED_NANOSECONDS_PER_SECOND / i64::from(self.fps)
    }
}

const PROBE_CONFIG: H264EncoderConfig =
    H264EncoderConfig::new(PROBE_WIDTH, PROBE_HEIGHT, PROBE_FPS, PROBE_BITRATE);

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct EncodedAccessUnit {
    pub bytes: Vec<u8>,
    pub timestamp_100ns: Option<i64>,
    pub duration_100ns: Option<i64>,
    pub keyframe: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct H264EncodeBatch {
    pub encoder_name: String,
    pub access_units: Vec<EncodedAccessUnit>,
    pub frames_submitted: u32,
    pub elapsed_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct HardwareEncoder {
    pub name: String,
    pub clsid: String,
    pub hardware_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct HardwareEncodeProbe {
    pub encoder_name: String,
    pub width: u32,
    pub height: u32,
    pub frames_submitted: u32,
    pub access_units: usize,
    pub timestamped_access_units: usize,
    pub keyframes: usize,
    pub encoded_bytes: usize,
    pub nal_units: usize,
    pub elapsed_ms: u64,
}

pub(super) struct MediaFoundationRuntime;

impl MediaFoundationRuntime {
    pub fn startup() -> Result<Self, String> {
        // SAFETY: startup happens once on the dedicated Windows media worker
        // and is balanced by `Drop` on that same thread.
        unsafe { MFStartup(MF_VERSION, MFSTARTUP_FULL) }
            .map_err(|error| format!("failed to start Media Foundation: {error}"))?;
        Ok(Self)
    }

    pub fn hardware_h264_encoders() -> Result<Vec<HardwareEncoder>, String> {
        let activations = hardware_h264_activations()?;
        let mut encoders = activations
            .iter()
            .map(hardware_encoder_metadata)
            .collect::<Vec<_>>();
        encoders.sort_by(|left, right| left.name.cmp(&right.name));
        encoders.dedup();
        Ok(encoders)
    }

    pub fn probe_hardware_h264_encode() -> Result<HardwareEncodeProbe, String> {
        let batch = Self::encode_synthetic_h264(PROBE_CONFIG, PROBE_FRAME_COUNT)?;
        let encoded_bytes = batch
            .access_units
            .iter()
            .map(|unit| unit.bytes.len())
            .sum::<usize>();
        let nal_units = batch
            .access_units
            .iter()
            .map(|unit| count_annex_b_nal_units(&unit.bytes))
            .sum::<usize>();
        let timestamped_access_units = batch
            .access_units
            .iter()
            .filter(|unit| {
                unit.timestamp_100ns.is_some()
                    && unit.duration_100ns.is_some_and(|duration| duration > 0)
            })
            .count();
        let keyframes = batch
            .access_units
            .iter()
            .filter(|unit| unit.keyframe)
            .count();
        if encoded_bytes == 0 || nal_units == 0 {
            return Err(format!(
                "encoder returned {encoded_bytes} bytes and {nal_units} Annex B H.264 NAL units"
            ));
        }
        if timestamped_access_units != batch.access_units.len() {
            return Err(format!(
                "encoder timestamped only {timestamped_access_units} of {} H.264 access units",
                batch.access_units.len()
            ));
        }
        if keyframes == 0 {
            return Err("encoder produced no independently decodable H.264 keyframe".to_owned());
        }
        Ok(HardwareEncodeProbe {
            encoder_name: batch.encoder_name,
            width: PROBE_CONFIG.width,
            height: PROBE_CONFIG.height,
            frames_submitted: batch.frames_submitted,
            access_units: batch.access_units.len(),
            timestamped_access_units,
            keyframes,
            encoded_bytes,
            nal_units,
            elapsed_ms: batch.elapsed_ms,
        })
    }

    pub fn encode_synthetic_h264(
        config: H264EncoderConfig,
        frame_count: u32,
    ) -> Result<H264EncodeBatch, String> {
        let config = config.validate()?;
        if frame_count == 0 {
            return Err("H.264 encode batch must contain at least one frame".to_owned());
        }
        let mut activations = hardware_h264_activations()?;
        activations.sort_by_key(|activation| hardware_encoder_metadata(activation).name);
        if activations.is_empty() {
            return Err(
                "Media Foundation reported no hardware H.264 encoder for NV12 input".to_owned(),
            );
        }

        let mut failures = Vec::with_capacity(activations.len());
        for activation in activations {
            let encoder = hardware_encoder_metadata(&activation);
            let result = encode_activation(&activation, &encoder.name, config, frame_count);
            // SAFETY: this balances `ActivateObject` when activation succeeded;
            // calling it after an activation failure is also documented as safe.
            let _ = unsafe { activation.ShutdownObject() };
            match result {
                Ok(report) => return Ok(report),
                Err(error) => failures.push(format!("{}: {error}", encoder.name)),
            }
        }

        Err(format!(
            "all hardware H.264 encode probes failed ({})",
            failures.join("; ")
        ))
    }
}

fn hardware_h264_activations() -> Result<Vec<IMFActivate>, String> {
    let input = MFT_REGISTER_TYPE_INFO {
        guidMajorType: MFMediaType_Video,
        guidSubtype: MFVideoFormat_NV12,
    };
    let output = MFT_REGISTER_TYPE_INFO {
        guidMajorType: MFMediaType_Video,
        guidSubtype: MFVideoFormat_H264,
    };
    let mut pointer = std::ptr::null_mut();
    let mut count = 0_u32;

    // SAFETY: both type descriptors live for the call. Media Foundation
    // initializes the returned CoTaskMem array and count on success.
    unsafe {
        MFTEnumEx(
            MFT_CATEGORY_VIDEO_ENCODER,
            MFT_ENUM_FLAG_HARDWARE | MFT_ENUM_FLAG_SORTANDFILTER,
            Some(&raw const input),
            Some(&raw const output),
            &raw mut pointer,
            &raw mut count,
        )
    }
    .map_err(|error| format!("failed to enumerate hardware H.264 encoders: {error}"))?;

    let mut activations = ActivationArray::new(pointer, count)?;
    Ok(activations.take_all())
}

fn hardware_encoder_metadata(activation: &IMFActivate) -> HardwareEncoder {
    let name = allocated_string(activation, &MFT_FRIENDLY_NAME_Attribute)
        .unwrap_or_else(|_error| "Unnamed Media Foundation encoder".to_owned());
    let clsid_key = MFT_TRANSFORM_CLSID_Attribute;
    let clsid = unsafe { activation.GetGUID(&raw const clsid_key) }
        .map_or_else(|_error| "unknown".to_owned(), format_guid);
    let hardware_url = allocated_string(activation, &MFT_ENUM_HARDWARE_URL_Attribute).ok();
    HardwareEncoder {
        name,
        clsid,
        hardware_url,
    }
}

fn encode_activation(
    activation: &IMFActivate,
    encoder_name: &str,
    config: H264EncoderConfig,
    frame_count: u32,
) -> Result<H264EncodeBatch, String> {
    let started = Instant::now();
    // SAFETY: the activation owns the returned COM object until the matching
    // `ShutdownObject` call in the caller.
    let transform = unsafe { activation.ActivateObject::<IMFTransform>() }
        .map_err(|error| format!("failed to activate encoder: {error}"))?;
    let asynchronous = unlock_if_asynchronous(&transform)?;
    let _d3d_manager = bind_d3d11_manager_if_supported(&transform)?;
    configure_transform(&transform, config)?;

    // SAFETY: these messages follow successful media-type negotiation and are
    // paired with end-of-stream/flush cleanup below.
    unsafe { transform.ProcessMessage(MFT_MESSAGE_NOTIFY_BEGIN_STREAMING, 0) }
        .map_err(|error| format!("failed to begin encoder streaming: {error}"))?;
    unsafe { transform.ProcessMessage(MFT_MESSAGE_NOTIFY_START_OF_STREAM, 0) }
        .map_err(|error| format!("failed to start encoder stream: {error}"))?;

    let encode_result = if asynchronous {
        encode_async(&transform, started, config, frame_count)
    } else {
        encode_sync(&transform, started, config, frame_count)
    };

    // SAFETY: cleanup messages are idempotent for a configured transform. Any
    // individual cleanup failure must not hide the primary encode result.
    let _ = unsafe { transform.ProcessMessage(MFT_MESSAGE_NOTIFY_END_OF_STREAM, 0) };
    let _ = unsafe { transform.ProcessMessage(MFT_MESSAGE_COMMAND_FLUSH, 0) };
    let _ = unsafe { transform.ProcessMessage(MFT_MESSAGE_NOTIFY_END_STREAMING, 0) };

    let encoded = encode_result?;
    if encoded.access_units.is_empty() {
        return Err("encoder completed without producing H.264 bytes".to_owned());
    }
    let encoded_bytes = encoded
        .access_units
        .iter()
        .map(|unit| unit.bytes.len())
        .sum::<usize>();
    let nal_units = encoded
        .access_units
        .iter()
        .map(|unit| count_annex_b_nal_units(&unit.bytes))
        .sum::<usize>();
    if nal_units == 0 {
        return Err(format!(
            "encoder returned {encoded_bytes} bytes without an Annex B H.264 start code"
        ));
    }

    Ok(H264EncodeBatch {
        encoder_name: encoder_name.to_owned(),
        access_units: encoded.access_units,
        frames_submitted: encoded.frames_submitted,
        elapsed_ms: millis_u64(started.elapsed()),
    })
}

struct D3dManagerBinding<'a> {
    transform: &'a IMFTransform,
    _device: ID3D11Device,
    _manager: IMFDXGIDeviceManager,
}

impl Drop for D3dManagerBinding<'_> {
    fn drop(&mut self) {
        // SAFETY: zero clears the manager previously attached to this MFT.
        let _ = unsafe {
            self.transform
                .ProcessMessage(MFT_MESSAGE_SET_D3D_MANAGER, 0)
        };
    }
}

fn bind_d3d11_manager_if_supported(
    transform: &IMFTransform,
) -> Result<Option<D3dManagerBinding<'_>>, String> {
    // SAFETY: the transform owns the returned attribute store.
    let attributes = unsafe { transform.GetAttributes() }
        .map_err(|error| format!("failed to query encoder D3D attributes: {error}"))?;
    let aware_key = MF_SA_D3D11_AWARE;
    let aware = unsafe { attributes.GetUINT32(&raw const aware_key) }.is_ok_and(|value| value != 0);
    if !aware {
        return Ok(None);
    }

    let device = super::create_native_d3d11_device(D3D_DRIVER_TYPE_HARDWARE)
        .map_err(|error| format!("failed to create encoder D3D11 device: {error}"))?;
    let mut reset_token = 0_u32;
    let mut manager = None;
    // SAFETY: both output pointers reference initialized local storage.
    unsafe { MFCreateDXGIDeviceManager(&raw mut reset_token, &raw mut manager) }
        .map_err(|error| format!("failed to create encoder DXGI device manager: {error}"))?;
    let manager = manager.ok_or_else(|| {
        "Media Foundation created no DXGI device manager despite succeeding".to_owned()
    })?;
    // SAFETY: the manager and D3D11 device remain alive in the returned binding.
    unsafe { manager.ResetDevice(&device, reset_token) }
        .map_err(|error| format!("failed to bind encoder D3D11 device: {error}"))?;
    let manager_pointer = Interface::as_raw(&manager) as usize;
    unsafe { transform.ProcessMessage(MFT_MESSAGE_SET_D3D_MANAGER, manager_pointer) }
        .map_err(|error| format!("encoder rejected its DXGI device manager: {error}"))?;

    Ok(Some(D3dManagerBinding {
        transform,
        _device: device,
        _manager: manager,
    }))
}

fn unlock_if_asynchronous(transform: &IMFTransform) -> Result<bool, String> {
    // SAFETY: the transform owns the returned attribute store. Reading and
    // setting scalar attributes does not outlive that store.
    let attributes = unsafe { transform.GetAttributes() }
        .map_err(|error| format!("failed to query encoder attributes: {error}"))?;
    let asynchronous_key = MF_TRANSFORM_ASYNC;
    let asynchronous_unlock_key = MF_TRANSFORM_ASYNC_UNLOCK;
    let is_asynchronous =
        unsafe { attributes.GetUINT32(&raw const asynchronous_key) }.is_ok_and(|value| value != 0);
    if is_asynchronous {
        unsafe { attributes.SetUINT32(&raw const asynchronous_unlock_key, 1) }
            .map_err(|error| format!("failed to unlock asynchronous encoder: {error}"))?;
    }
    Ok(is_asynchronous)
}

fn configure_transform(transform: &IMFTransform, config: H264EncoderConfig) -> Result<(), String> {
    // Media Foundation's H.264 encoder contract requires output negotiation
    // before input negotiation.
    let output = create_video_type(MFVideoFormat_H264, false, config)?;
    set_u32(&output, &MF_MT_AVG_BITRATE, config.bitrate)?;
    set_u32(
        &output,
        &MF_MT_MPEG2_PROFILE,
        u32::try_from(eAVEncH264VProfile_Main.0)
            .map_err(|_error| "invalid H.264 main profile value".to_owned())?,
    )?;
    // SAFETY: stream zero is the single input/output stream reported by video
    // encoder MFTs returned for this category and type pair.
    unsafe { transform.SetOutputType(0, &output, 0) }
        .map_err(|error| format!("failed to set H.264 output media type: {error}"))?;

    let input = create_video_type(MFVideoFormat_NV12, true, config)?;
    unsafe { transform.SetInputType(0, &input, 0) }
        .map_err(|error| format!("failed to set NV12 input media type: {error}"))?;
    Ok(())
}

fn create_video_type(
    subtype: GUID,
    uncompressed: bool,
    config: H264EncoderConfig,
) -> Result<::windows::Win32::Media::MediaFoundation::IMFMediaType, String> {
    // SAFETY: the returned COM object owns its attribute storage.
    let media_type = unsafe { MFCreateMediaType() }
        .map_err(|error| format!("failed to create video media type: {error}"))?;
    set_guid(&media_type, &MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
    set_guid(&media_type, &MF_MT_SUBTYPE, &subtype)?;
    set_u64(
        &media_type,
        &MF_MT_FRAME_SIZE,
        pack_u32_pair(config.width, config.height),
    )?;
    set_u64(&media_type, &MF_MT_FRAME_RATE, pack_u32_pair(config.fps, 1))?;
    set_u64(&media_type, &MF_MT_PIXEL_ASPECT_RATIO, pack_u32_pair(1, 1))?;
    set_u32(
        &media_type,
        &MF_MT_INTERLACE_MODE,
        u32::try_from(MFVideoInterlace_Progressive.0)
            .map_err(|_error| "invalid progressive interlace value".to_owned())?,
    )?;
    if uncompressed {
        let sample_size = nv12_sample_size(config.width, config.height)?;
        set_u32(&media_type, &MF_MT_FIXED_SIZE_SAMPLES, 1)?;
        set_u32(&media_type, &MF_MT_ALL_SAMPLES_INDEPENDENT, 1)?;
        set_u32(
            &media_type,
            &MF_MT_SAMPLE_SIZE,
            u32::try_from(sample_size)
                .map_err(|_error| "NV12 probe frame is too large".to_owned())?,
        )?;
    }
    Ok(media_type)
}

fn set_guid(
    attributes: &::windows::Win32::Media::MediaFoundation::IMFAttributes,
    key: &GUID,
    value: &GUID,
) -> Result<(), String> {
    // SAFETY: both GUID pointers remain valid for the duration of the call.
    unsafe { attributes.SetGUID(key, value) }.map_err(|error| error.to_string())
}

fn set_u32(
    attributes: &::windows::Win32::Media::MediaFoundation::IMFAttributes,
    key: &GUID,
    value: u32,
) -> Result<(), String> {
    // SAFETY: the GUID pointer remains valid for the duration of the call.
    unsafe { attributes.SetUINT32(key, value) }.map_err(|error| error.to_string())
}

fn set_u64(
    attributes: &::windows::Win32::Media::MediaFoundation::IMFAttributes,
    key: &GUID,
    value: u64,
) -> Result<(), String> {
    // SAFETY: the GUID pointer remains valid for the duration of the call.
    unsafe { attributes.SetUINT64(key, value) }.map_err(|error| error.to_string())
}

struct EncodedProbe {
    access_units: Vec<EncodedAccessUnit>,
    frames_submitted: u32,
}

fn encode_async(
    transform: &IMFTransform,
    started: Instant,
    config: H264EncoderConfig,
    frame_count: u32,
) -> Result<EncodedProbe, String> {
    let events = transform
        .cast::<IMFMediaEventGenerator>()
        .map_err(|error| format!("asynchronous encoder has no event generator: {error}"))?;
    let mut access_units = Vec::new();
    let mut frames_submitted = 0_u32;
    let mut draining = false;
    let mut drain_complete = false;

    while started.elapsed() < PROBE_TIMEOUT {
        // SAFETY: non-blocking event retrieval returns either a fully owned
        // event or MF_E_NO_EVENTS_AVAILABLE.
        let event = match unsafe { events.GetEvent(MF_EVENT_FLAG_NO_WAIT) } {
            Ok(event) => event,
            Err(error) if error.code() == MF_E_NO_EVENTS_AVAILABLE => {
                thread::sleep(Duration::from_millis(1));
                continue;
            }
            Err(error) => return Err(format!("failed to poll encoder event: {error}")),
        };
        // SAFETY: the event object remains alive while its scalar fields are read.
        let status = unsafe { event.GetStatus() }
            .map_err(|error| format!("failed to read encoder event status: {error}"))?;
        status
            .ok()
            .map_err(|error| format!("encoder event failed: {error}"))?;
        let event_type = unsafe { event.GetType() }
            .map_err(|error| format!("failed to read encoder event type: {error}"))?;

        if event_type == u32::try_from(METransformNeedInput.0).unwrap_or_default() {
            if frames_submitted < frame_count {
                let sample = create_nv12_sample(config, frames_submitted)?;
                // SAFETY: the sample owns its buffer and remains alive for the call.
                unsafe { transform.ProcessInput(0, &sample, 0) }
                    .map_err(|error| format!("encoder rejected NV12 frame: {error}"))?;
                frames_submitted += 1;
            } else if !draining {
                // SAFETY: no more input is submitted after these messages.
                unsafe { transform.ProcessMessage(MFT_MESSAGE_NOTIFY_END_OF_STREAM, 0) }
                    .map_err(|error| format!("failed to end encoder input stream: {error}"))?;
                unsafe { transform.ProcessMessage(MFT_MESSAGE_COMMAND_DRAIN, 0) }
                    .map_err(|error| format!("failed to drain encoder: {error}"))?;
                draining = true;
            }
        } else if event_type == u32::try_from(METransformHaveOutput.0).unwrap_or_default() {
            if let Some(output) = take_output(transform)? {
                if !output.bytes.is_empty() {
                    access_units.push(output);
                }
            }
        } else if event_type == u32::try_from(METransformDrainComplete.0).unwrap_or_default() {
            drain_complete = true;
            break;
        }
    }

    if !drain_complete {
        return Err(format!(
            "hardware encoder did not finish the H.264 batch within {} ms after {frames_submitted} frames",
            PROBE_TIMEOUT.as_millis()
        ));
    }
    Ok(EncodedProbe {
        access_units,
        frames_submitted,
    })
}

fn encode_sync(
    transform: &IMFTransform,
    started: Instant,
    config: H264EncoderConfig,
    frame_count: u32,
) -> Result<EncodedProbe, String> {
    let mut access_units = Vec::new();
    let mut frames_submitted = 0_u32;

    while frames_submitted < frame_count && started.elapsed() < PROBE_TIMEOUT {
        let sample = create_nv12_sample(config, frames_submitted)?;
        // SAFETY: the sample owns its buffer and remains alive for the call.
        unsafe { transform.ProcessInput(0, &sample, 0) }
            .map_err(|error| format!("encoder rejected NV12 frame: {error}"))?;
        frames_submitted += 1;

        loop {
            match take_output(transform) {
                Ok(Some(output)) => {
                    if !output.bytes.is_empty() {
                        access_units.push(output);
                    }
                }
                Ok(None) => break,
                Err(error) if error.contains("needs more input") => break,
                Err(error) => return Err(error),
            }
        }
    }

    if frames_submitted < frame_count {
        return Err(format!(
            "hardware encoder accepted only {frames_submitted} of {frame_count} frames within {} ms",
            PROBE_TIMEOUT.as_millis()
        ));
    }

    // SAFETY: all requested input has been submitted before draining.
    unsafe { transform.ProcessMessage(MFT_MESSAGE_NOTIFY_END_OF_STREAM, 0) }
        .map_err(|error| format!("failed to end encoder input stream: {error}"))?;
    unsafe { transform.ProcessMessage(MFT_MESSAGE_COMMAND_DRAIN, 0) }
        .map_err(|error| format!("failed to drain encoder: {error}"))?;
    while started.elapsed() < PROBE_TIMEOUT {
        match take_output(transform) {
            Ok(Some(output)) => {
                if !output.bytes.is_empty() {
                    access_units.push(output);
                }
            }
            Ok(None) => {}
            Err(error) if error.contains("needs more input") => break,
            Err(error) => return Err(error),
        }
    }

    Ok(EncodedProbe {
        access_units,
        frames_submitted,
    })
}

fn output_stream_info(transform: &IMFTransform) -> Result<MFT_OUTPUT_STREAM_INFO, String> {
    // SAFETY: stream zero is the configured H.264 output stream.
    unsafe { transform.GetOutputStreamInfo(0) }
        .map_err(|error| format!("failed to query encoder output buffer requirements: {error}"))
}

fn take_output(transform: &IMFTransform) -> Result<Option<EncodedAccessUnit>, String> {
    let info = output_stream_info(transform)?;
    let provides_samples =
        info.dwFlags & u32::try_from(MFT_OUTPUT_STREAM_PROVIDES_SAMPLES.0).unwrap_or_default() != 0;
    let can_provide_samples = info.dwFlags
        & u32::try_from(MFT_OUTPUT_STREAM_CAN_PROVIDE_SAMPLES.0).unwrap_or_default()
        != 0;
    let caller_sample = if provides_samples || can_provide_samples {
        None
    } else {
        Some(create_output_sample(info.cbSize.max(1))?)
    };
    let mut output = MFT_OUTPUT_DATA_BUFFER {
        dwStreamID: 0,
        pSample: ManuallyDrop::new(caller_sample),
        dwStatus: 0,
        pEvents: ManuallyDrop::new(None),
    };
    let mut process_status = 0_u32;
    // SAFETY: the one-element output slice and status pointer remain valid for
    // the call. ManuallyDrop fields are reclaimed immediately below.
    let result = unsafe {
        transform.ProcessOutput(
            0,
            std::slice::from_mut(&mut output),
            &raw mut process_status,
        )
    };
    // SAFETY: these ManuallyDrop fields were initialized above and are each
    // moved exactly once, including on ProcessOutput failure.
    let sample = unsafe { ManuallyDrop::take(&mut output.pSample) };
    let events = unsafe { ManuallyDrop::take(&mut output.pEvents) };
    drop(events);

    if let Err(error) = result {
        if error.code() == MF_E_TRANSFORM_NEED_MORE_INPUT {
            return Err("encoder needs more input".to_owned());
        }
        if error.code() == MF_E_TRANSFORM_STREAM_CHANGE {
            renegotiate_output_type(transform)?;
            // Asynchronous MFTs issue a fresh METransformHaveOutput event after
            // renegotiation; calling ProcessOutput again for the consumed event
            // violates their event contract.
            return Ok(None);
        }
        return Err(format!(
            "failed to receive encoder output: {error} (stream flags {:#x}, size {}, alignment {}, output status {process_status:#x}, buffer status {:#x})",
            info.dwFlags, info.cbSize, info.cbAlignment, output.dwStatus
        ));
    }
    let Some(sample) = sample else {
        return Ok(None);
    };
    read_access_unit(&sample).map(Some)
}

fn renegotiate_output_type(transform: &IMFTransform) -> Result<(), String> {
    let mut failures = Vec::new();
    for type_index in 0..32 {
        // SAFETY: stream zero is the configured output stream; every returned
        // media type owns its COM lifetime.
        let media_type = match unsafe { transform.GetOutputAvailableType(0, type_index) } {
            Ok(media_type) => media_type,
            Err(error) => {
                failures.push(format!("type {type_index}: {error}"));
                break;
            }
        };
        // SAFETY: the candidate media type remains alive for the call.
        match unsafe { transform.SetOutputType(0, &media_type, 0) } {
            Ok(()) => return Ok(()),
            Err(error) => failures.push(format!("type {type_index}: {error}")),
        }
    }
    Err(format!(
        "encoder requested an output stream change but exposed no usable H.264 type ({})",
        failures.join("; ")
    ))
}

fn create_output_sample(size: u32) -> Result<IMFSample, String> {
    // SAFETY: both returned objects own their allocations; the buffer is then
    // transferred to the sample through a COM reference.
    let sample = unsafe { MFCreateSample() }
        .map_err(|error| format!("failed to create encoder output sample: {error}"))?;
    let buffer = unsafe { MFCreateMemoryBuffer(size) }
        .map_err(|error| format!("failed to create encoder output buffer: {error}"))?;
    unsafe { sample.AddBuffer(&buffer) }
        .map_err(|error| format!("failed to attach encoder output buffer: {error}"))?;
    Ok(sample)
}

fn create_nv12_sample(config: H264EncoderConfig, frame_index: u32) -> Result<IMFSample, String> {
    let frame = synthetic_nv12_frame(config.width, config.height, frame_index)?;
    let size = u32::try_from(frame.len()).map_err(|_error| "NV12 frame is too large".to_owned())?;
    // SAFETY: the returned objects own their allocations; the buffer is locked
    // only while the source slice is copied.
    let sample = unsafe { MFCreateSample() }
        .map_err(|error| format!("failed to create NV12 input sample: {error}"))?;
    let buffer = unsafe { MFCreateMemoryBuffer(size) }
        .map_err(|error| format!("failed to allocate NV12 input buffer: {error}"))?;
    write_buffer(&buffer, &frame)?;
    unsafe { sample.AddBuffer(&buffer) }
        .map_err(|error| format!("failed to attach NV12 input buffer: {error}"))?;
    let duration = config.frame_duration_100ns();
    unsafe { sample.SetSampleTime(i64::from(frame_index) * duration) }
        .map_err(|error| format!("failed to timestamp NV12 input sample: {error}"))?;
    unsafe { sample.SetSampleDuration(duration) }
        .map_err(|error| format!("failed to set NV12 input duration: {error}"))?;
    Ok(sample)
}

fn read_access_unit(sample: &IMFSample) -> Result<EncodedAccessUnit, String> {
    let bytes = read_sample_bytes(sample)?;
    let clean_point_key = MFSampleExtension_CleanPoint;
    let clean_point =
        unsafe { sample.GetUINT32(&raw const clean_point_key) }.is_ok_and(|value| value != 0);
    let timestamp_100ns = unsafe { sample.GetSampleTime() }.ok();
    let duration_100ns = unsafe { sample.GetSampleDuration() }.ok();
    let keyframe = clean_point || annex_b_contains_idr(&bytes);
    Ok(EncodedAccessUnit {
        bytes,
        timestamp_100ns,
        duration_100ns,
        keyframe,
    })
}

fn write_buffer(buffer: &IMFMediaBuffer, source: &[u8]) -> Result<(), String> {
    let mut pointer = std::ptr::null_mut();
    let mut capacity = 0_u32;
    // SAFETY: the pointer is valid until the matching Unlock call below.
    unsafe { buffer.Lock(&raw mut pointer, Some(&raw mut capacity), None) }
        .map_err(|error| format!("failed to lock Media Foundation buffer: {error}"))?;
    let result = (|| {
        let capacity =
            usize::try_from(capacity).map_err(|_error| "buffer is too large".to_owned())?;
        if source.len() > capacity {
            return Err(format!(
                "Media Foundation buffer capacity {capacity} is smaller than {}",
                source.len()
            ));
        }
        if pointer.is_null() {
            return Err("Media Foundation returned a null buffer pointer".to_owned());
        }
        // SAFETY: Lock returned at least `capacity` writable bytes and the
        // source/destination do not overlap.
        unsafe { std::ptr::copy_nonoverlapping(source.as_ptr(), pointer, source.len()) };
        Ok(())
    })();
    // SAFETY: balances the successful Lock call above.
    let unlock = unsafe { buffer.Unlock() };
    result?;
    unlock.map_err(|error| format!("failed to unlock Media Foundation buffer: {error}"))?;
    unsafe { buffer.SetCurrentLength(u32::try_from(source.len()).unwrap_or(u32::MAX)) }
        .map_err(|error| format!("failed to set Media Foundation buffer length: {error}"))
}

fn read_sample_bytes(sample: &IMFSample) -> Result<Vec<u8>, String> {
    // SAFETY: the returned contiguous buffer owns the sample data.
    let buffer = unsafe { sample.ConvertToContiguousBuffer() }
        .map_err(|error| format!("failed to coalesce encoder output sample: {error}"))?;
    let length = unsafe { buffer.GetCurrentLength() }
        .map_err(|error| format!("failed to query encoder output length: {error}"))?;
    if length == 0 {
        return Ok(Vec::new());
    }
    let mut pointer = std::ptr::null_mut();
    let mut current_length = 0_u32;
    // SAFETY: the pointer remains valid until the matching Unlock below.
    unsafe { buffer.Lock(&raw mut pointer, None, Some(&raw mut current_length)) }
        .map_err(|error| format!("failed to lock encoder output buffer: {error}"))?;
    let result = if pointer.is_null() {
        Err("Media Foundation returned a null encoder output pointer".to_owned())
    } else {
        let len = usize::try_from(current_length)
            .map_err(|_error| "encoder output is too large".to_owned())?;
        // SAFETY: Lock returned `current_length` readable bytes.
        Ok(unsafe { std::slice::from_raw_parts(pointer, len) }.to_vec())
    };
    // SAFETY: balances the successful Lock call above.
    let unlock = unsafe { buffer.Unlock() };
    let bytes = result?;
    unlock.map_err(|error| format!("failed to unlock encoder output buffer: {error}"))?;
    Ok(bytes)
}

fn synthetic_nv12_frame(width: u32, height: u32, frame_index: u32) -> Result<Vec<u8>, String> {
    let len = nv12_sample_size(width, height)?;
    let luma_len = usize::try_from(u64::from(width) * u64::from(height))
        .map_err(|_error| "NV12 luma plane is too large".to_owned())?;
    let width_usize = usize::try_from(width).map_err(|_error| "width is too large".to_owned())?;
    let mut frame = vec![128_u8; len];
    for (index, value) in frame[..luma_len].iter_mut().enumerate() {
        let x = index % width_usize;
        let y = index / width_usize;
        let motion = usize::try_from(frame_index).unwrap_or_default() * 7;
        *value = u8::try_from((x + y + motion) % 220 + 16).unwrap_or(16);
    }
    Ok(frame)
}

fn nv12_sample_size(width: u32, height: u32) -> Result<usize, String> {
    if width == 0 || height == 0 || width % 2 != 0 || height % 2 != 0 {
        return Err("NV12 dimensions must be non-zero and even".to_owned());
    }
    usize::try_from(u64::from(width) * u64::from(height) * 3 / 2)
        .map_err(|_error| "NV12 frame is too large".to_owned())
}

const fn pack_u32_pair(high: u32, low: u32) -> u64 {
    (high as u64) << 32 | low as u64
}

fn count_annex_b_nal_units(bytes: &[u8]) -> usize {
    let mut count = 0;
    let mut index = 0;
    while index + 3 <= bytes.len() {
        if index + 4 <= bytes.len() && bytes[index..index + 4] == [0, 0, 0, 1] {
            count += 1;
            index += 4;
        } else if bytes[index..index + 3] == [0, 0, 1] {
            count += 1;
            index += 3;
        } else {
            index += 1;
        }
    }
    count
}

fn annex_b_contains_idr(bytes: &[u8]) -> bool {
    annex_b_nal_types(bytes).any(|nal_type| nal_type == 5)
}

fn annex_b_nal_types(bytes: &[u8]) -> impl Iterator<Item = u8> + '_ {
    (0..bytes.len()).filter_map(move |index| {
        let payload_index = if bytes.get(index..index.saturating_add(4)) == Some(&[0, 0, 0, 1]) {
            index.checked_add(4)?
        } else if bytes.get(index..index.saturating_add(3)) == Some(&[0, 0, 1]) {
            index.checked_add(3)?
        } else {
            return None;
        };
        bytes.get(payload_index).map(|header| header & 0x1f)
    })
}

fn millis_u64(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

impl Drop for MediaFoundationRuntime {
    fn drop(&mut self) {
        // SAFETY: balances the successful startup on the same dedicated thread.
        let _ = unsafe { MFShutdown() };
    }
}

struct ActivationArray {
    pointer: NonNull<Option<IMFActivate>>,
    len: usize,
}

impl ActivationArray {
    fn new(pointer: *mut Option<IMFActivate>, count: u32) -> Result<Self, String> {
        let len = usize::try_from(count).map_err(|_error| "encoder count is too large")?;
        let pointer = match NonNull::new(pointer) {
            Some(pointer) => pointer,
            None if len == 0 => NonNull::dangling(),
            None => return Err("Media Foundation returned a null encoder array".to_owned()),
        };
        Ok(Self { pointer, len })
    }

    fn take_all(&mut self) -> Vec<IMFActivate> {
        // SAFETY: `pointer` and `len` are the initialized array returned by
        // `MFTEnumEx`; every element is moved out at most once with `take`.
        unsafe { std::slice::from_raw_parts_mut(self.pointer.as_ptr(), self.len) }
            .iter_mut()
            .filter_map(Option::take)
            .collect()
    }
}

impl Drop for ActivationArray {
    fn drop(&mut self) {
        if self.len == 0 {
            return;
        }
        // SAFETY: any elements not already moved out are released before the
        // exact CoTaskMem allocation returned by `MFTEnumEx` is freed.
        unsafe {
            for activation in std::slice::from_raw_parts_mut(self.pointer.as_ptr(), self.len) {
                let _ = activation.take();
            }
            CoTaskMemFree(Some(self.pointer.as_ptr().cast::<c_void>().cast_const()));
        }
    }
}

fn allocated_string(activation: &IMFActivate, key: &GUID) -> Result<String, String> {
    let mut pointer = PWSTR::null();
    let mut length = 0_u32;
    // SAFETY: Media Foundation initializes an allocated UTF-16 pointer and its
    // character count. The allocation is freed below on every success path.
    unsafe { activation.GetAllocatedString(key, &raw mut pointer, &raw mut length) }
        .map_err(|error| error.to_string())?;
    let len = usize::try_from(length).map_err(|_error| "encoder name is too long")?;
    let value = if pointer.is_null() {
        String::new()
    } else {
        // SAFETY: `GetAllocatedString` returned `length` initialized UTF-16
        // code units at this non-null pointer.
        String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(pointer.0, len) })
    };
    // SAFETY: this is the allocation returned by `GetAllocatedString`.
    unsafe { CoTaskMemFree(Some(pointer.0.cast::<c_void>().cast_const())) };
    Ok(value)
}

fn format_guid(guid: GUID) -> String {
    format!("{guid:?}").to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::{
        H264EncoderConfig, annex_b_contains_idr, count_annex_b_nal_units, format_guid,
        nv12_sample_size, synthetic_nv12_frame,
    };
    use windows::core::GUID;

    #[test]
    fn guid_diagnostics_are_stable() {
        assert_eq!(
            format_guid(GUID::from_u128(0x6ca50344_051a_4ded_9779_a43305165e35)),
            "6ca50344-051a-4ded-9779-a43305165e35"
        );
    }

    #[test]
    fn annex_b_parser_counts_three_and_four_byte_start_codes() {
        let stream = [0, 0, 0, 1, 0x67, 1, 2, 0, 0, 1, 0x68, 3, 0, 0, 0, 1, 0x65];
        assert_eq!(count_annex_b_nal_units(&stream), 3);
        assert!(annex_b_contains_idr(&stream));
        assert!(!annex_b_contains_idr(&[0, 0, 0, 1, 0x41, 2, 3]));
        assert_eq!(count_annex_b_nal_units(&[1, 2, 3]), 0);
    }

    #[test]
    fn encoder_configuration_rejects_invalid_video_geometry() {
        assert!(
            H264EncoderConfig::new(640, 360, 60, 4_000_000)
                .validate()
                .is_ok()
        );
        assert!(
            H264EncoderConfig::new(641, 360, 60, 4_000_000)
                .validate()
                .is_err()
        );
        assert!(
            H264EncoderConfig::new(640, 360, 0, 4_000_000)
                .validate()
                .is_err()
        );
        assert!(H264EncoderConfig::new(640, 360, 60, 0).validate().is_err());
    }

    #[test]
    fn synthetic_nv12_frames_have_valid_plane_lengths() {
        let frame = synthetic_nv12_frame(640, 360, 2).expect("valid even dimensions");
        assert_eq!(frame.len(), 640 * 360 * 3 / 2);
        assert!(frame[..640 * 360].iter().any(|value| *value != 128));
        assert!(frame[640 * 360..].iter().all(|value| *value == 128));
        assert!(nv12_sample_size(641, 360).is_err());
        assert!(nv12_sample_size(640, 0).is_err());
    }
}
