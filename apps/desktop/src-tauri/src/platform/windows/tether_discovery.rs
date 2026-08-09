//! Read-only Windows discovery for Android USB-tether gateways.
//!
//! A private default gateway alone is not evidence of USB tethering. This
//! adapter first proves that the Windows network device has a `USB` ancestor in
//! the Plug and Play tree, then intersects it with active IPv4 adapters and
//! their reported gateways. It never probes a port or sends network traffic.

use std::{
    collections::HashSet,
    ffi::CStr,
    mem::{MaybeUninit, size_of},
    net::{Ipv4Addr, SocketAddrV4},
};

use windows::{
    Win32::{
        Devices::DeviceAndDriverInstallation::{
            CM_Get_Device_ID_Size, CM_Get_Device_IDW, CM_Get_Parent, CR_SUCCESS, DICS_FLAG_GLOBAL,
            DIGCF_PRESENT, DIREG_DRV, GUID_DEVCLASS_NET, HDEVINFO, SP_DEVINFO_DATA,
            SetupDiDestroyDeviceInfoList, SetupDiEnumDeviceInfo, SetupDiGetClassDevsW,
            SetupDiOpenDevRegKey,
        },
        Foundation::{ERROR_BUFFER_OVERFLOW, ERROR_NO_MORE_ITEMS, ERROR_SUCCESS, GetLastError},
        NetworkManagement::{
            IpHelper::{
                GAA_FLAG_INCLUDE_GATEWAYS, GAA_FLAG_SKIP_ANYCAST, GAA_FLAG_SKIP_DNS_SERVER,
                GAA_FLAG_SKIP_MULTICAST, GAA_FLAG_SKIP_UNICAST, GetAdaptersAddresses,
                IP_ADAPTER_ADDRESSES_LH,
            },
            Ndis::IfOperStatusUp,
        },
        Networking::WinSock::{AF_INET, SOCKADDR_IN},
        System::Registry::{HKEY, KEY_READ, REG_SZ, REG_VALUE_TYPE, RegCloseKey, RegQueryValueExW},
    },
    core::{PCWSTR, PWSTR, w},
};

use crate::tether::{
    DEFAULT_TETHER_PORT, TetherDiscoveryReport, TetherEndpointCandidate, is_local_tether_address,
};

const INITIAL_ADAPTER_BUFFER_BYTES: usize = 15 * 1_024;
const MAX_ADAPTER_BUFFER_BYTES: usize = 1024 * 1_024;
const MAX_NETWORK_DEVICES: u32 = 1_024;
const MAX_DEVICE_ANCESTORS: usize = 16;
const MAX_ADAPTERS: usize = 256;
const MAX_GATEWAYS_PER_ADAPTER: usize = 8;
const MAX_CANDIDATES: usize = 16;
const MAX_WINDOWS_STRING_UNITS: usize = 1_024;

pub fn discover_tether_endpoints() -> Result<TetherDiscoveryReport, String> {
    let usb_network_guids = enumerate_usb_network_guids()?;
    let mut candidates = enumerate_gateway_candidates(&usb_network_guids)?;
    candidates.sort_by(|left, right| {
        left.adapter_name
            .cmp(&right.adapter_name)
            .then(left.gateway.cmp(&right.gateway))
    });
    candidates.dedup_by(|left, right| left.endpoint == right.endpoint);
    candidates.truncate(MAX_CANDIDATES);

    let detail = match candidates.len() {
        0 => "No active private IPv4 gateway was found on a network adapter whose Plug and Play ancestry is USB. Enable Android USB tethering or enter the address manually."
            .to_owned(),
        1 => "Found one active private gateway on a USB-backed Windows network adapter. Confirm it matches the address shown by Android before pairing."
            .to_owned(),
        count => format!(
            "Found {count} active private gateways on USB-backed Windows network adapters. Select the one that matches Android."
        ),
    };
    Ok(TetherDiscoveryReport { candidates, detail })
}

fn enumerate_usb_network_guids() -> Result<HashSet<String>, String> {
    let class_guid = GUID_DEVCLASS_NET;
    // SAFETY: the class GUID and flags are valid, and the returned handle is
    // owned by `DeviceInfoSet` for the remainder of this function.
    let handle = unsafe {
        SetupDiGetClassDevsW(
            Some(&raw const class_guid),
            PCWSTR::null(),
            None,
            DIGCF_PRESENT,
        )
    }
    .map_err(|error| format!("failed to enumerate present Windows network devices: {error}"))?;
    let devices = DeviceInfoSet(handle);
    let mut guids = HashSet::new();

    for index in 0..MAX_NETWORK_DEVICES {
        let mut info = SP_DEVINFO_DATA {
            cbSize: u32::try_from(size_of::<SP_DEVINFO_DATA>())
                .map_err(|_| "SP_DEVINFO_DATA size exceeds the Win32 range".to_owned())?,
            ..SP_DEVINFO_DATA::default()
        };
        // SAFETY: `devices` remains live and `info` advertises the exact
        // structure size required by SetupAPI.
        if let Err(error) = unsafe { SetupDiEnumDeviceInfo(devices.0, index, &raw mut info) } {
            // SAFETY: this reads the calling thread's last-error value
            // immediately after the failed SetupAPI call.
            if unsafe { GetLastError() } == ERROR_NO_MORE_ITEMS {
                break;
            }
            return Err(format!(
                "Windows network-device enumeration failed at index {index}: {error}"
            ));
        }
        if !has_usb_ancestor(info.DevInst) {
            continue;
        }
        if let Some(instance_guid) = read_netcfg_instance_id(devices.0, &info) {
            guids.insert(normalize_adapter_guid(&instance_guid));
        }
    }
    Ok(guids)
}

fn has_usb_ancestor(mut device: u32) -> bool {
    for _depth in 0..MAX_DEVICE_ANCESTORS {
        if device_instance_id(device)
            .is_some_and(|instance| instance.to_ascii_uppercase().starts_with("USB\\"))
        {
            return true;
        }
        let mut parent = 0_u32;
        // SAFETY: `parent` is writable and `device` came from SetupAPI or the
        // previous successful Config Manager parent lookup.
        if unsafe { CM_Get_Parent(&raw mut parent, device, 0) } != CR_SUCCESS {
            return false;
        }
        device = parent;
    }
    false
}

fn device_instance_id(device: u32) -> Option<String> {
    let mut length = 0_u32;
    // SAFETY: `length` is a valid out parameter and flags must be zero.
    if unsafe { CM_Get_Device_ID_Size(&raw mut length, device, 0) } != CR_SUCCESS {
        return None;
    }
    let units = usize::try_from(length).ok()?.checked_add(1)?;
    if units > MAX_WINDOWS_STRING_UNITS {
        return None;
    }
    let mut buffer = vec![0_u16; units];
    // SAFETY: the buffer has the size reported by Config Manager plus one unit
    // for its documented trailing NUL.
    if unsafe { CM_Get_Device_IDW(device, &mut buffer, 0) } != CR_SUCCESS {
        return None;
    }
    Some(utf16_until_nul(&buffer))
}

fn read_netcfg_instance_id(devices: HDEVINFO, info: &SP_DEVINFO_DATA) -> Option<String> {
    // SAFETY: `devices` and `info` identify a currently enumerated network
    // device. Only the read-only driver registry key is requested.
    let key = unsafe {
        SetupDiOpenDevRegKey(devices, info, DICS_FLAG_GLOBAL.0, 0, DIREG_DRV, KEY_READ.0)
    }
    .ok()?;
    let key = RegistryKey(key);
    let mut value_type = REG_VALUE_TYPE::default();
    let mut byte_count = 0_u32;
    // SAFETY: this first call supplies no data pointer and requests the value
    // type and required byte count only.
    let first = unsafe {
        RegQueryValueExW(
            key.0,
            w!("NetCfgInstanceId"),
            None,
            Some(&raw mut value_type),
            None,
            Some(&raw mut byte_count),
        )
    };
    if first != ERROR_SUCCESS || value_type != REG_SZ || byte_count < 2 {
        return None;
    }
    let unit_count = usize::try_from(byte_count).ok()?.div_ceil(size_of::<u16>());
    if unit_count > MAX_WINDOWS_STRING_UNITS {
        return None;
    }
    let mut buffer = vec![0_u16; unit_count];
    // SAFETY: the byte buffer is exactly the size returned by the first query;
    // the key is read-only and remains open through this call.
    let second = unsafe {
        RegQueryValueExW(
            key.0,
            w!("NetCfgInstanceId"),
            None,
            Some(&raw mut value_type),
            Some(buffer.as_mut_ptr().cast()),
            Some(&raw mut byte_count),
        )
    };
    (second == ERROR_SUCCESS && value_type == REG_SZ).then(|| utf16_until_nul(&buffer))
}

fn enumerate_gateway_candidates(
    usb_network_guids: &HashSet<String>,
) -> Result<Vec<TetherEndpointCandidate>, String> {
    let flags = GAA_FLAG_INCLUDE_GATEWAYS
        | GAA_FLAG_SKIP_UNICAST
        | GAA_FLAG_SKIP_ANYCAST
        | GAA_FLAG_SKIP_MULTICAST
        | GAA_FLAG_SKIP_DNS_SERVER;
    let mut requested_bytes = INITIAL_ADAPTER_BUFFER_BYTES;

    for _attempt in 0..3 {
        if requested_bytes > MAX_ADAPTER_BUFFER_BYTES {
            return Err(format!(
                "Windows requested an unexpectedly large adapter buffer ({requested_bytes} bytes)"
            ));
        }
        let words = requested_bytes.div_ceil(size_of::<usize>());
        let mut storage = vec![0_usize; words];
        let mut available_bytes =
            u32::try_from(storage.len().saturating_mul(size_of::<usize>()))
                .map_err(|_| "adapter buffer exceeds the Win32 range".to_owned())?;
        // SAFETY: `storage` is aligned for the returned pointer-rich structs,
        // remains live while the list is traversed, and its byte length is
        // provided accurately to IP Helper.
        let result = unsafe {
            GetAdaptersAddresses(
                u32::from(AF_INET.0),
                flags,
                None,
                Some(storage.as_mut_ptr().cast()),
                &raw mut available_bytes,
            )
        };
        if result == ERROR_BUFFER_OVERFLOW.0 {
            requested_bytes = usize::try_from(available_bytes)
                .map_err(|_| "adapter buffer size is not representable".to_owned())?;
            continue;
        }
        if result != ERROR_SUCCESS.0 {
            return Err(format!(
                "GetAdaptersAddresses failed with Windows error {result}"
            ));
        }
        // SAFETY: the successful call initialized a linked list wholly inside
        // `storage`, which remains borrowed until collection finishes.
        return Ok(unsafe {
            collect_gateway_candidates(storage.as_mut_ptr().cast(), usb_network_guids)
        });
    }
    Err("Windows adapter addresses changed repeatedly during discovery; try again".to_owned())
}

unsafe fn collect_gateway_candidates(
    mut adapter: *mut IP_ADAPTER_ADDRESSES_LH,
    usb_network_guids: &HashSet<String>,
) -> Vec<TetherEndpointCandidate> {
    let mut candidates = Vec::new();
    let mut visited = 0_usize;
    while !adapter.is_null() && visited < MAX_ADAPTERS {
        // SAFETY: `adapter` is the current node in the buffer-owned linked list.
        let current = unsafe { &*adapter };
        visited += 1;
        if current.OperStatus == IfOperStatusUp {
            // SAFETY: AdapterName is a NUL-terminated ANSI string owned by the
            // GetAdaptersAddresses buffer for the duration of this traversal.
            let adapter_guid = unsafe { adapter_name(current.AdapterName.0) };
            if usb_network_guids.contains(&normalize_adapter_guid(&adapter_guid)) {
                // SAFETY: these optional UTF-16 pointers share the adapter
                // buffer lifetime and are bounded by `wide_string`.
                let friendly = unsafe { wide_string(current.FriendlyName) };
                // SAFETY: same lifetime and bounds as FriendlyName.
                let description = unsafe { wide_string(current.Description) };
                let adapter_name = if friendly.is_empty() {
                    if description.is_empty() {
                        "USB network adapter".to_owned()
                    } else {
                        description
                    }
                } else {
                    friendly
                };
                // SAFETY: the gateway list belongs to the same successful IP
                // Helper buffer and is traversed with a strict node bound.
                unsafe {
                    collect_adapter_gateways(
                        current.FirstGatewayAddress,
                        &adapter_name,
                        &mut candidates,
                    );
                }
            }
        }
        adapter = current.Next;
    }
    candidates
}

unsafe fn collect_adapter_gateways(
    mut gateway: *mut windows::Win32::NetworkManagement::IpHelper::IP_ADAPTER_GATEWAY_ADDRESS_LH,
    adapter_name: &str,
    candidates: &mut Vec<TetherEndpointCandidate>,
) {
    for _index in 0..MAX_GATEWAYS_PER_ADAPTER {
        if gateway.is_null() {
            break;
        }
        // SAFETY: `gateway` is the current node in the bounded buffer-owned
        // linked list.
        let current = unsafe { &*gateway };
        let socket = current.Address;
        if !socket.lpSockaddr.is_null()
            && usize::try_from(socket.iSockaddrLength).is_ok_and(|length| {
                length >= size_of::<SOCKADDR_IN>()
            })
            // SAFETY: the non-null socket pointer is valid for at least the
            // checked SOCKADDR_IN length.
            && unsafe { (*socket.lpSockaddr).sa_family == AF_INET }
        {
            let mut ipv4 = MaybeUninit::<SOCKADDR_IN>::uninit();
            // SAFETY: family and byte length were validated above. A byte copy
            // avoids assuming SOCKADDR's weaker pointer alignment is sufficient
            // for a direct SOCKADDR_IN reference.
            unsafe {
                std::ptr::copy_nonoverlapping(
                    socket.lpSockaddr.cast::<u8>(),
                    ipv4.as_mut_ptr().cast::<u8>(),
                    size_of::<SOCKADDR_IN>(),
                );
            }
            // SAFETY: every byte of the plain C structure was initialized by
            // the copy above.
            let ipv4 = unsafe { ipv4.assume_init() };
            // SAFETY: reading the byte view of IN_ADDR is valid for IPv4.
            let octets = unsafe { ipv4.sin_addr.S_un.S_un_b };
            let address = Ipv4Addr::new(octets.s_b1, octets.s_b2, octets.s_b3, octets.s_b4);
            if is_local_tether_address(address) && !address.is_loopback() {
                candidates.push(TetherEndpointCandidate {
                    endpoint: SocketAddrV4::new(address, DEFAULT_TETHER_PORT).to_string(),
                    adapter_name: adapter_name.to_owned(),
                    gateway: address.to_string(),
                    evidence: "USB device-tree parent and active private IPv4 gateway".to_owned(),
                });
            }
        }
        gateway = current.Next;
    }
}

unsafe fn adapter_name(pointer: *const u8) -> String {
    if pointer.is_null() {
        return String::new();
    }
    // SAFETY: the caller guarantees a system-provided NUL-terminated string.
    unsafe { CStr::from_ptr(pointer.cast()) }
        .to_string_lossy()
        .into_owned()
}

unsafe fn wide_string(pointer: PWSTR) -> String {
    if pointer.is_null() {
        return String::new();
    }
    let mut length = 0_usize;
    // SAFETY: the caller guarantees a valid system string pointer. The strict
    // upper bound prevents unbounded scanning if the data is malformed.
    while length < MAX_WINDOWS_STRING_UNITS && unsafe { *pointer.0.add(length) } != 0 {
        length += 1;
    }
    // SAFETY: each unit up to `length` was just proven readable.
    String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(pointer.0, length) })
}

fn normalize_adapter_guid(value: &str) -> String {
    value
        .trim()
        .trim_start_matches('{')
        .trim_end_matches('}')
        .to_ascii_lowercase()
}

fn utf16_until_nul(value: &[u16]) -> String {
    let length = value
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(value.len());
    String::from_utf16_lossy(&value[..length])
}

struct DeviceInfoSet(HDEVINFO);

impl Drop for DeviceInfoSet {
    fn drop(&mut self) {
        // SAFETY: this guard uniquely owns the valid SetupAPI list handle.
        let _result = unsafe { SetupDiDestroyDeviceInfoList(self.0) };
    }
}

struct RegistryKey(HKEY);

impl Drop for RegistryKey {
    fn drop(&mut self) {
        // SAFETY: this guard uniquely owns the device registry key handle.
        let _result = unsafe { RegCloseKey(self.0) };
    }
}

#[cfg(test)]
mod tests {
    use std::net::SocketAddr;

    use super::{MAX_CANDIDATES, discover_tether_endpoints, normalize_adapter_guid};
    use crate::tether::is_local_tether_address;

    #[test]
    fn adapter_guids_are_normalized_without_localized_names() {
        assert_eq!(
            normalize_adapter_guid(" {A1B2C3D4-E5F6-47A8-9012-3456789ABCDE} "),
            "a1b2c3d4-e5f6-47a8-9012-3456789abcde"
        );
    }

    #[test]
    fn discovery_is_bounded_and_never_returns_a_public_gateway() {
        let report = discover_tether_endpoints().expect("read-only discovery succeeds");
        assert!(report.candidates.len() <= MAX_CANDIDATES);
        for candidate in report.candidates {
            let endpoint = candidate
                .endpoint
                .parse::<SocketAddr>()
                .expect("candidate endpoint");
            let std::net::IpAddr::V4(address) = endpoint.ip() else {
                panic!("discovery must return IPv4 endpoints");
            };
            assert!(is_local_tether_address(address) && !address.is_loopback());
        }
    }
}
