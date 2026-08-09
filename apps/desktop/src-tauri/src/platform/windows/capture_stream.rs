//! Long-running Windows.Graphics.Capture to GPU H.264 stream.

#![allow(unsafe_code)]

use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{SyncSender, TrySendError, sync_channel},
    },
    time::{Duration, Instant},
};

use windows::{
    Foundation::TypedEventHandler,
    Graphics::{
        Capture::{
            Direct3D11CaptureFrame, Direct3D11CaptureFramePool, GraphicsCaptureItem,
            GraphicsCaptureSession,
        },
        DirectX::Direct3D11::IDirect3DSurface,
    },
    Win32::{
        Graphics::Direct3D11::ID3D11Texture2D,
        System::WinRT::Direct3D11::IDirect3DDxgiInterfaceAccess,
    },
    core::{IInspectable, Interface},
};

use super::{
    CAPTURE_BUFFER_COUNT, CAPTURE_PIXEL_FORMAT, H264StreamBatch, H264StreamConfig,
    MediaFoundationRuntime, WinRtApartment, close_capture_session, create_capture_device,
    create_capture_item, enumerate_monitors, positive_dimension, select_monitor,
};
use super::{
    convert_h264_access_units,
    media_foundation::{H264EncoderConfig, HardwareH264Encoder},
    send_encoder_result,
    video_processor::BgraToNv12Processor,
};

const CAPTURE_FRAME_QUEUE: usize = 3;
const CAPTURE_WAIT: Duration = Duration::from_millis(10);

pub(super) fn run(
    config: H264StreamConfig,
    display_id: Option<&str>,
    cancel: &Arc<AtomicBool>,
    sender: &SyncSender<Result<H264StreamBatch, String>>,
) -> Result<(), String> {
    let _apartment = WinRtApartment::initialize()?;
    let _media_foundation = MediaFoundationRuntime::startup()?;
    if !GraphicsCaptureSession::IsSupported()
        .map_err(|error| format!("failed to query Windows.Graphics.Capture: {error}"))?
    {
        return Err("Windows.Graphics.Capture is not supported by this OS build".to_owned());
    }

    let mut capture = PreparedCapture::new(config, display_id)?;
    run_prepared_capture(&mut capture, cancel, sender)
}

struct PreparedCapture {
    item: GraphicsCaptureItem,
    frame_pool: Direct3D11CaptureFramePool,
    capture_device: super::CaptureDevice,
    config: H264StreamConfig,
    processor: BgraToNv12Processor,
    encoder: HardwareH264Encoder,
}

impl PreparedCapture {
    fn new(config: H264StreamConfig, display_id: Option<&str>) -> Result<Self, String> {
        let monitors = enumerate_monitors()?;
        let selected = select_monitor(&monitors, display_id)?;
        let item = create_capture_item(selected.handle)?;
        let item_size = item
            .Size()
            .map_err(|error| format!("failed to query capture item size: {error}"))?;
        let input_width = positive_dimension(item_size.Width, "capture width")?;
        let input_height = positive_dimension(item_size.Height, "capture height")?;
        let capture_device = create_capture_device()?;
        let frame_pool = Direct3D11CaptureFramePool::CreateFreeThreaded(
            &capture_device.runtime,
            CAPTURE_PIXEL_FORMAT,
            CAPTURE_BUFFER_COUNT,
            item_size,
        )
        .map_err(|error| format!("failed to create streaming capture frame pool: {error}"))?;
        let encoder_config = H264EncoderConfig::new(
            u32::from(config.width),
            u32::from(config.height),
            u32::from(config.fps),
            config
                .bitrate_kbps
                .checked_mul(1_000)
                .ok_or_else(|| "H.264 bitrate exceeds the Windows encoder range".to_owned())?,
        );
        let processor = BgraToNv12Processor::new(
            &capture_device.native,
            input_width,
            input_height,
            u32::from(config.width),
            u32::from(config.height),
            u32::from(config.fps),
        )?;
        let encoder = HardwareH264Encoder::start(encoder_config, &capture_device.native)?;
        Ok(Self {
            item,
            frame_pool,
            capture_device,
            config,
            processor,
            encoder,
        })
    }
}

fn run_prepared_capture(
    capture: &mut PreparedCapture,
    cancel: &AtomicBool,
    sender: &SyncSender<Result<H264StreamBatch, String>>,
) -> Result<(), String> {
    let (frame_sender, frame_receiver) = sync_channel(CAPTURE_FRAME_QUEUE);
    let source_closed = Arc::new(AtomicBool::new(false));
    let closed_handler = capture_closed_handler(Arc::clone(&source_closed));
    let closed_token = capture
        .item
        .Closed(&closed_handler)
        .map_err(|error| format!("failed to subscribe to capture-source closure: {error}"))?;
    let handler = frame_handler(frame_sender);
    let handler_token = match capture.frame_pool.FrameArrived(&handler) {
        Ok(token) => token,
        Err(error) => {
            let _removed = capture.item.RemoveClosed(closed_token);
            let _closed = capture.frame_pool.Close();
            return Err(format!(
                "failed to subscribe to streaming capture frames: {error}"
            ));
        }
    };
    let session = match capture.frame_pool.CreateCaptureSession(&capture.item) {
        Ok(session) => session,
        Err(error) => {
            let _removed = capture.frame_pool.RemoveFrameArrived(handler_token);
            let _removed_closed = capture.item.RemoveClosed(closed_token);
            let _closed = capture.frame_pool.Close();
            return Err(format!(
                "failed to create streaming capture session: {error}"
            ));
        }
    };
    let _cursor = session.SetIsCursorCaptureEnabled(true);

    if let Err(error) = session.StartCapture() {
        let mut cleanup_errors =
            close_capture_session(&capture.frame_pool, handler_token, &session);
        if let Err(remove_error) = capture.item.RemoveClosed(closed_token) {
            cleanup_errors.push(format!("remove source-closed handler: {remove_error}"));
        }
        return Err(if cleanup_errors.is_empty() {
            format!("failed to start streaming capture: {error}")
        } else {
            format!(
                "failed to start streaming capture: {error}; cleanup failed: {}",
                cleanup_errors.join("; ")
            )
        });
    }

    let capture_started = Instant::now();
    let frame_interval = Duration::from_nanos(1_000_000_000 / u64::from(capture.config.fps));
    let result = CapturePipeline {
        receiver: &frame_receiver,
        cancel,
        source_closed: &source_closed,
        sender,
        capture_started,
        frame_interval,
        frame_pool: &capture.frame_pool,
        capture_device: &capture.capture_device,
        config: capture.config,
        processor: &mut capture.processor,
        encoder: &mut capture.encoder,
    }
    .run();
    let mut cleanup_errors = close_capture_session(&capture.frame_pool, handler_token, &session);
    if let Err(error) = capture.item.RemoveClosed(closed_token) {
        cleanup_errors.push(format!("remove source-closed handler: {error}"));
    }
    if let Err(error) = result {
        return if cleanup_errors.is_empty() {
            Err(error)
        } else {
            Err(format!(
                "{error}; capture cleanup also failed: {}",
                cleanup_errors.join("; ")
            ))
        };
    }
    if !cleanup_errors.is_empty() {
        return Err(format!(
            "streaming capture could not shut down cleanly: {}",
            cleanup_errors.join("; ")
        ));
    }
    Ok(())
}

fn frame_handler(
    sender: SyncSender<Result<Direct3D11CaptureFrame, String>>,
) -> TypedEventHandler<Direct3D11CaptureFramePool, IInspectable> {
    TypedEventHandler::<Direct3D11CaptureFramePool, IInspectable>::new(move |pool, _args| {
        let frame = match pool.ok() {
            Ok(pool) => pool
                .TryGetNextFrame()
                .map_err(|error| format!("failed to dequeue streaming capture frame: {error}")),
            Err(error) => Err(format!(
                "streaming capture callback had no frame pool: {error}"
            )),
        };
        match sender.try_send(frame) {
            Ok(()) | Err(TrySendError::Disconnected(_)) => {}
            Err(TrySendError::Full(Ok(frame))) => {
                let _closed = frame.Close();
            }
            Err(TrySendError::Full(Err(_error))) => {}
        }
        Ok(())
    })
}

fn capture_closed_handler(
    source_closed: Arc<AtomicBool>,
) -> TypedEventHandler<GraphicsCaptureItem, IInspectable> {
    TypedEventHandler::<GraphicsCaptureItem, IInspectable>::new(move |_item, _args| {
        source_closed.store(true, Ordering::Release);
        Ok(())
    })
}

struct CapturePipeline<'a> {
    receiver: &'a std::sync::mpsc::Receiver<Result<Direct3D11CaptureFrame, String>>,
    cancel: &'a AtomicBool,
    source_closed: &'a AtomicBool,
    sender: &'a SyncSender<Result<H264StreamBatch, String>>,
    capture_started: Instant,
    frame_interval: Duration,
    frame_pool: &'a Direct3D11CaptureFramePool,
    capture_device: &'a super::CaptureDevice,
    config: H264StreamConfig,
    processor: &'a mut BgraToNv12Processor,
    encoder: &'a mut HardwareH264Encoder,
}

impl CapturePipeline<'_> {
    fn run(&mut self) -> Result<(), String> {
        let mut next_frame_at = Duration::ZERO;
        while !self.cancel.load(Ordering::Acquire) {
            if self.source_closed.load(Ordering::Acquire) {
                return Err("selected Windows capture source was closed or removed".to_owned());
            }
            self.process_next_frame(&mut next_frame_at)?;
        }
        Ok(())
    }

    fn process_next_frame(&mut self, next_frame_at: &mut Duration) -> Result<(), String> {
        let frame = match self.receiver.recv_timeout(CAPTURE_WAIT) {
            Ok(frame) => frame?,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => return Ok(()),
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                return Err("Windows capture callback stopped unexpectedly".to_owned());
            }
        };
        let captured_at = self.capture_started.elapsed();
        if captured_at < *next_frame_at {
            let _closed = frame.Close();
            return Ok(());
        }
        *next_frame_at = next_frame_at
            .checked_add(self.frame_interval)
            .unwrap_or(captured_at);
        if captured_at.saturating_sub(*next_frame_at) > self.frame_interval {
            *next_frame_at = captured_at;
        }

        let dimensions = (|| {
            let content_size = frame
                .ContentSize()
                .map_err(|error| format!("failed to query streaming frame size: {error}"))?;
            let width = positive_dimension(content_size.Width, "streaming frame width")?;
            let height = positive_dimension(content_size.Height, "streaming frame height")?;
            Ok::<_, String>((content_size, width, height))
        })();
        let (content_size, frame_width, frame_height) = match dimensions {
            Ok(dimensions) => dimensions,
            Err(error) => {
                let _closed = frame.Close();
                return Err(error);
            }
        };
        if self.processor.input_dimensions() != (frame_width, frame_height) {
            frame
                .Close()
                .map_err(|error| format!("failed to close resized capture frame: {error}"))?;
            self.frame_pool
                .Recreate(
                    &self.capture_device.runtime,
                    CAPTURE_PIXEL_FORMAT,
                    CAPTURE_BUFFER_COUNT,
                    content_size,
                )
                .map_err(|error| format!("failed to resize capture frame pool: {error}"))?;
            *self.processor = BgraToNv12Processor::new(
                &self.capture_device.native,
                frame_width,
                frame_height,
                u32::from(self.config.width),
                u32::from(self.config.height),
                u32::from(self.config.fps),
            )?;
            *next_frame_at = captured_at;
            return Ok(());
        }

        let access_unit_result = (|| {
            let texture = capture_texture(&frame)?;
            let nv12 = self.processor.convert(&texture)?;
            let timestamp_100ns = duration_to_100ns(captured_at)?;
            self.encoder.encode_texture(&nv12, timestamp_100ns)
        })();
        let close_result = frame.Close();
        let access_units = access_unit_result?;
        close_result.map_err(|error| format!("failed to close captured frame: {error}"))?;
        if access_units.is_empty() {
            return Ok(());
        }
        let batch =
            convert_h264_access_units(self.encoder.encoder_name().to_owned(), access_units)?;
        if !send_encoder_result(self.sender, Ok(batch), self.cancel) {
            return Ok(());
        }
        Ok(())
    }
}

fn capture_texture(frame: &Direct3D11CaptureFrame) -> Result<ID3D11Texture2D, String> {
    let surface: IDirect3DSurface = frame
        .Surface()
        .map_err(|error| format!("captured frame has no D3D surface: {error}"))?;
    let access = surface
        .cast::<IDirect3DDxgiInterfaceAccess>()
        .map_err(|error| format!("captured surface has no DXGI interop: {error}"))?;
    unsafe { access.GetInterface::<ID3D11Texture2D>() }
        .map_err(|error| format!("failed to query captured D3D11 texture: {error}"))
}

fn duration_to_100ns(duration: Duration) -> Result<i64, String> {
    let ticks = duration.as_nanos() / 100;
    i64::try_from(ticks).map_err(|_| "capture timestamp exceeds Media Foundation range".to_owned())
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::duration_to_100ns;

    #[test]
    fn capture_timestamp_uses_media_foundation_units() {
        assert_eq!(
            duration_to_100ns(Duration::from_micros(16_667)),
            Ok(166_670)
        );
    }
}
