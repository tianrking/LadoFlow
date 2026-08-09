//! D3D11 video-processor conversion from capture BGRA surfaces to encoder NV12.

#![allow(unsafe_code)]

use std::mem::ManuallyDrop;

use windows::{
    Win32::{
        Foundation::RECT,
        Graphics::{
            Direct3D11::{
                D3D11_BIND_RENDER_TARGET, D3D11_TEX2D_VPIV, D3D11_TEX2D_VPOV, D3D11_TEXTURE2D_DESC,
                D3D11_USAGE_DEFAULT, D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE,
                D3D11_VIDEO_PROCESSOR_CONTENT_DESC, D3D11_VIDEO_PROCESSOR_FORMAT_SUPPORT_INPUT,
                D3D11_VIDEO_PROCESSOR_FORMAT_SUPPORT_OUTPUT, D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC,
                D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC_0, D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC,
                D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC_0, D3D11_VIDEO_PROCESSOR_STREAM,
                D3D11_VIDEO_USAGE_OPTIMAL_SPEED, D3D11_VPIV_DIMENSION_TEXTURE2D,
                D3D11_VPOV_DIMENSION_TEXTURE2D, ID3D11Device, ID3D11Texture2D, ID3D11VideoContext,
                ID3D11VideoDevice, ID3D11VideoProcessor, ID3D11VideoProcessorEnumerator,
                ID3D11VideoProcessorOutputView,
            },
            Dxgi::Common::{
                DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_NV12, DXGI_RATIONAL, DXGI_SAMPLE_DESC,
            },
        },
    },
    core::{BOOL, Interface},
};

pub(super) struct BgraToNv12Processor {
    video_device: ID3D11VideoDevice,
    video_context: ID3D11VideoContext,
    enumerator: ID3D11VideoProcessorEnumerator,
    processor: ID3D11VideoProcessor,
    output_texture: ID3D11Texture2D,
    output_view: ID3D11VideoProcessorOutputView,
    input_width: u32,
    input_height: u32,
    frame_index: u32,
}

impl BgraToNv12Processor {
    #[allow(clippy::too_many_lines)]
    pub fn new(
        device: &ID3D11Device,
        input_width: u32,
        input_height: u32,
        output_width: u32,
        output_height: u32,
        fps: u32,
    ) -> Result<Self, String> {
        validate_dimensions(input_width, input_height, output_width, output_height, fps)?;
        let video_device = device
            .cast::<ID3D11VideoDevice>()
            .map_err(|error| format!("D3D11 device has no video interface: {error}"))?;
        // SAFETY: the immediate context belongs to `device` and the cast keeps
        // its own COM reference for this processor's lifetime.
        let context = unsafe { device.GetImmediateContext() }
            .map_err(|error| format!("failed to get D3D11 immediate context: {error}"))?;
        let video_context = context
            .cast::<ID3D11VideoContext>()
            .map_err(|error| format!("D3D11 context has no video interface: {error}"))?;
        let frame_rate = DXGI_RATIONAL {
            Numerator: fps,
            Denominator: 1,
        };
        let processor_description = D3D11_VIDEO_PROCESSOR_CONTENT_DESC {
            InputFrameFormat: D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE,
            InputFrameRate: frame_rate,
            InputWidth: input_width,
            InputHeight: input_height,
            OutputFrameRate: frame_rate,
            OutputWidth: output_width,
            OutputHeight: output_height,
            Usage: D3D11_VIDEO_USAGE_OPTIMAL_SPEED,
        };
        // SAFETY: `content` remains valid for the call and the returned COM
        // interfaces own their native lifetimes.
        let enumerator = unsafe {
            video_device.CreateVideoProcessorEnumerator(&raw const processor_description)
        }
        .map_err(|error| format!("failed to create D3D11 video enumerator: {error}"))?;
        require_format_support(
            &enumerator,
            DXGI_FORMAT_B8G8R8A8_UNORM,
            D3D11_VIDEO_PROCESSOR_FORMAT_SUPPORT_INPUT.0,
            "BGRA input",
        )?;
        require_format_support(
            &enumerator,
            DXGI_FORMAT_NV12,
            D3D11_VIDEO_PROCESSOR_FORMAT_SUPPORT_OUTPUT.0,
            "NV12 output",
        )?;
        let processor = unsafe { video_device.CreateVideoProcessor(&enumerator, 0) }
            .map_err(|error| format!("failed to create D3D11 video processor: {error}"))?;
        let output_texture = create_nv12_texture(
            device,
            output_width,
            output_height,
            u32::try_from(D3D11_BIND_RENDER_TARGET.0)
                .map_err(|_| "invalid D3D11 render-target flag".to_owned())?,
        )?;
        let output_description = D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC {
            ViewDimension: D3D11_VPOV_DIMENSION_TEXTURE2D,
            Anonymous: D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC_0 {
                Texture2D: D3D11_TEX2D_VPOV { MipSlice: 0 },
            },
        };
        let mut output_view = None;
        unsafe {
            video_device.CreateVideoProcessorOutputView(
                &output_texture,
                &enumerator,
                &raw const output_description,
                Some(&raw mut output_view),
            )
        }
        .map_err(|error| format!("failed to create NV12 processor output view: {error}"))?;
        let output_view =
            output_view.ok_or_else(|| "D3D11 created no NV12 processor output view".to_owned())?;

        let source_rect = checked_rect(input_width, input_height)?;
        let destination_rect = checked_rect(output_width, output_height)?;
        // SAFETY: the processor and rects remain valid for these immediate
        // configuration calls.
        unsafe {
            video_context.VideoProcessorSetOutputTargetRect(
                &processor,
                true,
                Some(&raw const destination_rect),
            );
            video_context.VideoProcessorSetStreamFrameFormat(
                &processor,
                0,
                D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE,
            );
            video_context.VideoProcessorSetStreamSourceRect(
                &processor,
                0,
                true,
                Some(&raw const source_rect),
            );
            video_context.VideoProcessorSetStreamDestRect(
                &processor,
                0,
                true,
                Some(&raw const destination_rect),
            );
        }

        Ok(Self {
            video_device,
            video_context,
            enumerator,
            processor,
            output_texture,
            output_view,
            input_width,
            input_height,
            frame_index: 0,
        })
    }

    pub fn convert(&mut self, input: &ID3D11Texture2D) -> Result<ID3D11Texture2D, String> {
        let description = texture_description(input);
        if description.Width != self.input_width
            || description.Height != self.input_height
            || description.Format != DXGI_FORMAT_B8G8R8A8_UNORM
        {
            return Err(format!(
                "captured texture changed from {}x{} BGRA to {}x{} format {}",
                self.input_width,
                self.input_height,
                description.Width,
                description.Height,
                description.Format.0
            ));
        }
        let input_description = D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC {
            FourCC: 0,
            ViewDimension: D3D11_VPIV_DIMENSION_TEXTURE2D,
            Anonymous: D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC_0 {
                Texture2D: D3D11_TEX2D_VPIV {
                    MipSlice: 0,
                    ArraySlice: 0,
                },
            },
        };
        let mut input_view = None;
        // SAFETY: input and descriptors stay alive for the view-creation call.
        unsafe {
            self.video_device.CreateVideoProcessorInputView(
                input,
                &self.enumerator,
                &raw const input_description,
                Some(&raw mut input_view),
            )
        }
        .map_err(|error| format!("failed to create BGRA processor input view: {error}"))?;
        let input_view =
            input_view.ok_or_else(|| "D3D11 created no BGRA processor input view".to_owned())?;
        let mut stream = D3D11_VIDEO_PROCESSOR_STREAM {
            Enable: BOOL(1),
            pInputSurface: ManuallyDrop::new(Some(input_view)),
            ..Default::default()
        };
        // SAFETY: all views and the stream descriptor remain alive for this
        // immediate blit. The output texture belongs to the same D3D11 device.
        let result = unsafe {
            self.video_context.VideoProcessorBlt(
                &self.processor,
                &self.output_view,
                self.frame_index,
                std::slice::from_ref(&stream),
            )
        };
        // SAFETY: this reclaims the one COM reference placed in ManuallyDrop.
        unsafe { ManuallyDrop::drop(&mut stream.pInputSurface) };
        result.map_err(|error| format!("D3D11 BGRA-to-NV12 blit failed: {error}"))?;
        self.frame_index = self.frame_index.wrapping_add(1);
        Ok(self.output_texture.clone())
    }

    pub const fn input_dimensions(&self) -> (u32, u32) {
        (self.input_width, self.input_height)
    }
}

fn validate_dimensions(
    input_width: u32,
    input_height: u32,
    output_width: u32,
    output_height: u32,
    fps: u32,
) -> Result<(), String> {
    if input_width == 0 || input_height == 0 {
        return Err("D3D11 video processor input dimensions must be non-zero".to_owned());
    }
    if output_width == 0 || output_height == 0 || output_width % 2 != 0 || output_height % 2 != 0 {
        return Err("D3D11 NV12 output dimensions must be non-zero and even".to_owned());
    }
    if fps == 0 {
        return Err("D3D11 video processor frame rate must be non-zero".to_owned());
    }
    Ok(())
}

fn require_format_support(
    enumerator: &ID3D11VideoProcessorEnumerator,
    format: windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT,
    required: i32,
    label: &str,
) -> Result<(), String> {
    let flags = unsafe { enumerator.CheckVideoProcessorFormat(format) }
        .map_err(|error| format!("failed to query D3D11 {label} support: {error}"))?;
    let required =
        u32::try_from(required).map_err(|_| format!("invalid D3D11 {label} support flag"))?;
    if flags & required == 0 {
        return Err(format!("D3D11 video processor does not support {label}"));
    }
    Ok(())
}

fn create_nv12_texture(
    device: &ID3D11Device,
    width: u32,
    height: u32,
    bind_flags: u32,
) -> Result<ID3D11Texture2D, String> {
    let description = D3D11_TEXTURE2D_DESC {
        Width: width,
        Height: height,
        MipLevels: 1,
        ArraySize: 1,
        Format: DXGI_FORMAT_NV12,
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        Usage: D3D11_USAGE_DEFAULT,
        BindFlags: bind_flags,
        CPUAccessFlags: 0,
        MiscFlags: 0,
    };
    let mut texture = None;
    unsafe { device.CreateTexture2D(&raw const description, None, Some(&raw mut texture)) }
        .map_err(|error| format!("failed to create D3D11 NV12 texture: {error}"))?;
    texture.ok_or_else(|| "D3D11 created no NV12 texture".to_owned())
}

fn texture_description(texture: &ID3D11Texture2D) -> D3D11_TEXTURE2D_DESC {
    let mut description = D3D11_TEXTURE2D_DESC::default();
    unsafe { texture.GetDesc(&raw mut description) };
    description
}

fn checked_rect(width: u32, height: u32) -> Result<RECT, String> {
    Ok(RECT {
        left: 0,
        top: 0,
        right: i32::try_from(width)
            .map_err(|_| "D3D11 video width exceeds RECT range".to_owned())?,
        bottom: i32::try_from(height)
            .map_err(|_| "D3D11 video height exceeds RECT range".to_owned())?,
    })
}

#[cfg(test)]
mod tests {
    use super::validate_dimensions;

    #[test]
    fn video_processor_requires_even_nv12_output() {
        assert!(validate_dimensions(1_920, 1_080, 1_280, 720, 60).is_ok());
        assert!(validate_dimensions(0, 1_080, 1_280, 720, 60).is_err());
        assert!(validate_dimensions(1_920, 1_080, 1_281, 720, 60).is_err());
        assert!(validate_dimensions(1_920, 1_080, 1_280, 720, 0).is_err());
    }
}
