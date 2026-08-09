//! Media Foundation lifetime and hardware H.264 encoder discovery.

#![allow(unsafe_code)]

use std::{ffi::c_void, ptr::NonNull};

use ::windows::{
    Win32::{
        Media::MediaFoundation::{
            IMFActivate, MF_VERSION, MFMediaType_Video, MFSTARTUP_FULL, MFShutdown, MFStartup,
            MFT_CATEGORY_VIDEO_ENCODER, MFT_ENUM_FLAG_HARDWARE, MFT_ENUM_FLAG_SORTANDFILTER,
            MFT_ENUM_HARDWARE_URL_Attribute, MFT_FRIENDLY_NAME_Attribute, MFT_REGISTER_TYPE_INFO,
            MFT_TRANSFORM_CLSID_Attribute, MFTEnumEx, MFVideoFormat_H264, MFVideoFormat_NV12,
        },
        System::Com::CoTaskMemFree,
    },
    core::{GUID, PWSTR},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct HardwareEncoder {
    pub name: String,
    pub clsid: String,
    pub hardware_url: Option<String>,
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
        let mut encoders = Vec::with_capacity(activations.len());
        for activation in activations.take_all() {
            let name = allocated_string(&activation, &MFT_FRIENDLY_NAME_Attribute)
                .unwrap_or_else(|_error| "Unnamed Media Foundation encoder".to_owned());
            let clsid_key = MFT_TRANSFORM_CLSID_Attribute;
            let clsid = unsafe { activation.GetGUID(&raw const clsid_key) }
                .map_or_else(|_error| "unknown".to_owned(), format_guid);
            let hardware_url = allocated_string(&activation, &MFT_ENUM_HARDWARE_URL_Attribute).ok();
            encoders.push(HardwareEncoder {
                name,
                clsid,
                hardware_url,
            });
        }
        encoders.sort_by(|left, right| left.name.cmp(&right.name));
        encoders.dedup();
        Ok(encoders)
    }
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

    fn len(&self) -> usize {
        self.len
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
    use super::format_guid;
    use windows::core::GUID;

    #[test]
    fn guid_diagnostics_are_stable() {
        assert_eq!(
            format_guid(GUID::from_u128(0x6ca50344_051a_4ded_9779_a43305165e35)),
            "6ca50344-051a-4ded-9779-a43305165e35"
        );
    }
}
