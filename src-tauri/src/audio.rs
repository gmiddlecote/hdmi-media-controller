//! Audio output device management.
//!
//! Playback ("render") endpoints are enumerated from the Windows registry
//! because the lean offline dependency set has no WASAPI bindings. Choosing a
//! device works by making it the *system default* playback endpoint through
//! the undocumented `IPolicyConfig::SetDefaultEndpoint` interface — every
//! output window plays through whatever endpoint is the default, so picking a
//! device here is all the app needs to route its audio. The choice persists
//! system-wide until changed, exactly like picking a speaker in the Windows
//! sound settings.

use serde::Serialize;

/// A single playback (render) audio endpoint.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioDevice {
    /// Endpoint identifier, e.g. `{0.0.0.00000000}.{guid}`.
    pub id: String,
    /// Friendly name shown by Windows audio settings.
    pub name: String,
}

/// Lists active playback devices, sorted by name. Empty off Windows.
#[cfg(windows)]
pub fn list_devices() -> Vec<AudioDevice> {
    imp::list_devices()
}

/// Lists active playback devices, sorted by name. Empty off Windows.
#[cfg(not(windows))]
pub fn list_devices() -> Vec<AudioDevice> {
    Vec::new()
}

/// Makes `id` the system default playback endpoint. Falls back to the
/// existing default and clears the stored choice when the endpoint vanished.
#[cfg(windows)]
pub fn set_default_device(id: &str) -> Result<(), String> {
    imp::set_default_device(id)
}

/// Makes `id` the system default playback endpoint.
#[cfg(not(windows))]
pub fn set_default_device(_id: &str) -> Result<(), String> {
    Err("Audio device selection is only supported on Windows.".into())
}

#[cfg(windows)]
mod imp {
    use super::AudioDevice;
    use windows_sys::core::GUID;
    use windows_sys::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_ALL,
    };
    use winreg::enums::{HKEY_LOCAL_MACHINE, KEY_READ};
    use winreg::RegKey;

    /// Where Windows keeps the audio endpoint list.
    const RENDER_ROOT: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\MMDevices\Audio\Render";
    /// REG_SZ value holding the endpoint's friendly name.
    const FRIENDLY_NAME_VALUE: &str = r"{a45c254e-df1c-4efd-8020-67d146a850e0},2";
    /// DEVICE_STATE_ACTIVE = 1; disabled/unplugged endpoints are skipped.
    const DEVICE_STATE_ACTIVE: u32 = 1;

    /// The `PolicyConfigClient` COM class (undocumented but stable since
    /// Vista) that owns the default playback endpoint.
    const CLSID_POLICY_CONFIG_CLIENT: GUID = GUID {
        data1: 0x870A_F99C,
        data2: 0x171D,
        data3: 0x4F9E,
        data4: [0xAF, 0x0D, 0xE6, 0x3D, 0xF4, 0x0C, 0x2B, 0xC9],
    };
    const IID_IPOLICY_CONFIG: GUID = GUID {
        data1: 0xF867_9F50,
        data2: 0x850A,
        data3: 0x41CF,
        data4: [0x9C, 0x72, 0x43, 0x0F, 0x29, 0x02, 0x90, 0xC8],
    };

    /// `IPolicyConfig` vtable laid out as raw slots. Only Release (2) and
    /// SetDefaultEndpoint (13) are called; the rest keep slot offsets intact.
    #[repr(C)]
    struct PolicyConfigVtbl([usize; 15]);

    type ReleaseFn = unsafe extern "system" fn(*mut core::ffi::c_void) -> u32;
    type SetDefaultEndpointFn =
        unsafe extern "system" fn(*mut core::ffi::c_void, *const u16, u32) -> i32;

    pub fn list_devices() -> Vec<AudioDevice> {
        let Ok(root) =
            RegKey::predef(HKEY_LOCAL_MACHINE).open_subkey_with_flags(RENDER_ROOT, KEY_READ)
        else {
            return Vec::new();
        };
        let mut devices = Vec::new();
        for entry in root.enum_keys() {
            let Ok(guid) = entry else { continue };
            let Ok(key) = root.open_subkey_with_flags(&guid, KEY_READ) else {
                continue;
            };
            if key.get_value::<u32, _>("DeviceState").unwrap_or(0) != DEVICE_STATE_ACTIVE {
                continue;
            }
            let name = key
                .open_subkey("Properties")
                .ok()
                .and_then(|props| props.get_value::<String, _>(FRIENDLY_NAME_VALUE).ok())
                .unwrap_or_else(|| "Unknown audio device".to_string());
            let id = format!("{{0.0.0.00000000}}.{{{}}}", guid.to_lowercase());
            devices.push(AudioDevice { id, name });
        }
        devices.sort_by_key(|a| a.name.to_lowercase());
        devices
    }

    pub fn set_default_device(id: &str) -> Result<(), String> {
        const COINIT_APARTMENTTHREADED: u32 = 0x2;
        // S_OK (0) means we initialised and must balance it with CoUninitialize;
        // S_FALSE (1) means this thread already had COM. Other failures still
        // leave a STA/MTA running, so only treat a negative HRESULT as fatal.
        let init = unsafe { CoInitializeEx(std::ptr::null(), COINIT_APARTMENTTHREADED) };
        if init < 0 {
            return Err(format!("Could not initialise COM (0x{:08X}).", init as u32));
        }

        let wide: Vec<u16> = id.encode_utf16().chain(std::iter::once(0)).collect();
        let applied = unsafe {
            let mut policy: *mut core::ffi::c_void = std::ptr::null_mut();
            let created = CoCreateInstance(
                &CLSID_POLICY_CONFIG_CLIENT,
                std::ptr::null_mut(),
                CLSCTX_ALL,
                &IID_IPOLICY_CONFIG,
                &mut policy,
            );
            if created < 0 {
                return Err(format!(
                    "Could not create the audio policy service (0x{:08X}).",
                    created as u32
                ));
            }
            // The slot offsets must be exact: 0..2 are the IUnknown trio.
            let vtbl = &**(policy as *const *const PolicyConfigVtbl);
            let release = std::mem::transmute::<usize, ReleaseFn>(vtbl.0[2]);
            let set_default = std::mem::transmute::<usize, SetDefaultEndpointFn>(vtbl.0[13]);
            // 0 = ERole::eConsole — the default speaker/sink for ordinary audio.
            let result = set_default(policy, wide.as_ptr(), 0);
            release(policy);
            result
        };

        if init == 0 {
            unsafe { CoUninitialize() };
        }
        if applied < 0 {
            Err(format!(
                "The device could not be set as the default (0x{:08X}).",
                applied as u32
            ))
        } else {
            Ok(())
        }
    }
}
