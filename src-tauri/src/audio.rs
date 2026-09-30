//! Audio output device management.
//!
//! Playback ("render") endpoints are enumerated from the Windows registry
//! because the lean offline dependency set has no WASAPI bindings. Choosing a
//! device works by making it the *system default* playback endpoint through
//! the undocumented `IPolicyConfig::SetDefaultEndpoint` interface — every
//! output window plays through whatever endpoint is the default, so picking a
//! device here is all the app needs to route its audio. That makes the change
//! system-wide, exactly like picking a speaker in the Windows sound settings,
//! so [`Routing`] puts the user's own device back as soon as the app stops
//! holding it.

use serde::Serialize;

/// A single playback (render) audio endpoint.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioDevice {
    /// Endpoint identifier, e.g. `{0.0.0.00000000}.{guid}`.
    pub id: String,
    /// Friendly name shown by Windows audio settings, e.g. `Speakers`.
    pub name: String,
    /// Hardware or driver behind the endpoint, e.g. `Realtek(R) Audio`. The UI
    /// shows it alongside the name so virtual buses are told apart from real
    /// jacks, which Windows names generically.
    pub description: String,
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

/// Tracks the system default endpoint so the app can put it back.
///
/// The app cannot read the current default (the offline dependency set has no
/// WASAPI bindings), so this remembers only what the app has itself changed.
/// `home` is the device the user nominated as their normal output; it is
/// replayed whenever an item has no device of its own, and on stop and exit.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Routing {
    /// Device this app last made default; empty when it has set none.
    applied: String,
}

impl Routing {
    /// Device to make default now that `want` is playing, or `None` to leave
    /// the endpoint alone. `home` is the nominated restore target.
    pub fn apply(&mut self, want: &str, home: &str) -> Option<String> {
        if want.is_empty() {
            return self.restore(home);
        }
        if !home.is_empty() && want == home {
            // Already what we would restore, so there is nothing to do now and
            // nothing to undo later.
            self.applied.clear();
            return None;
        }
        if self.applied == want {
            // Already the default; setting it again would click audibly.
            return None;
        }
        self.applied = want.to_string();
        Some(want.to_string())
    }

    /// Device to put back so the system is left the way the user had it.
    pub fn restore(&mut self, home: &str) -> Option<String> {
        if self.applied.is_empty() {
            return None;
        }
        self.applied.clear();
        if home.is_empty() {
            return None;
        }
        Some(home.to_string())
    }
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
    /// REG_SZ value holding the device behind the endpoint, e.g. the chipset
    /// for an onboard jack or the driver for a virtual bus.
    const DESCRIPTION_VALUE: &str = r"{b3f8fa53-0004-438e-9003-51a46e139bfc},6";
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
            let props = key.open_subkey("Properties").ok();
            let read = |value: &str| -> String {
                props
                    .as_ref()
                    .and_then(|props| props.get_value::<String, _>(value).ok())
                    .unwrap_or_default()
            };
            let name = {
                let name = read(FRIENDLY_NAME_VALUE);
                if name.is_empty() {
                    "Unknown audio device".to_string()
                } else {
                    name
                }
            };
            let description = read(DESCRIPTION_VALUE);
            let id = format!("{{0.0.0.00000000}}.{{{}}}", guid.to_lowercase());
            devices.push(AudioDevice {
                id,
                name,
                description,
            });
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

#[cfg(test)]
mod tests {
    use super::{AudioDevice, Routing};

    #[test]
    fn device_fields_reach_the_ui_under_camel_case() {
        let json = serde_json::to_string(&AudioDevice {
            id: "{0.0.0.00000000}.{abc}".into(),
            name: "Speakers".into(),
            description: "Realtek(R) Audio".into(),
        })
        .unwrap();
        assert!(
            json.contains(r#""description":"Realtek(R) Audio""#),
            "{json}"
        );
        assert!(json.contains(r#""name":"Speakers""#), "{json}");
    }

    #[test]
    fn applies_the_items_device() {
        let mut routing = Routing::default();
        assert_eq!(routing.apply("tv", "laptop"), Some("tv".into()));
    }

    #[test]
    fn does_not_reapply_the_same_device() {
        let mut routing = Routing::default();
        routing.apply("tv", "laptop");
        assert_eq!(routing.apply("tv", "laptop"), None);
    }

    #[test]
    fn restore_puts_the_nominated_device_back() {
        let mut routing = Routing::default();
        routing.apply("tv", "laptop");
        assert_eq!(routing.restore("laptop"), Some("laptop".into()));
    }

    #[test]
    fn restore_is_a_no_op_when_nothing_was_applied() {
        let mut routing = Routing::default();
        assert_eq!(routing.restore("laptop"), None);
    }

    #[test]
    fn restore_only_needs_to_act_once() {
        let mut routing = Routing::default();
        routing.apply("tv", "laptop");
        assert_eq!(routing.restore("laptop"), Some("laptop".into()));
        assert_eq!(routing.restore("laptop"), None);
    }

    #[test]
    fn an_item_without_a_choice_restores() {
        let mut routing = Routing::default();
        routing.apply("tv", "laptop");
        assert_eq!(routing.apply("", "laptop"), Some("laptop".into()));
    }

    #[test]
    fn choosing_the_home_device_needs_no_restore() {
        let mut routing = Routing::default();
        assert_eq!(routing.apply("laptop", "laptop"), None);
        assert_eq!(routing.restore("laptop"), None);
    }

    #[test]
    fn applying_the_home_device_cancels_an_earlier_choice() {
        let mut routing = Routing::default();
        routing.apply("tv", "laptop");
        assert_eq!(routing.apply("laptop", "laptop"), None);
        assert_eq!(routing.restore("laptop"), None);
    }

    #[test]
    fn without_a_home_device_nothing_is_restored() {
        let mut routing = Routing::default();
        routing.apply("tv", "");
        assert_eq!(routing.restore(""), None);
    }

    #[test]
    fn a_forgotten_home_device_does_not_pin_the_previous_choice() {
        let mut routing = Routing::default();
        routing.apply("tv", "laptop");
        routing.apply("", "");
        // The app is no longer holding the endpoint, so nominating a device
        // later must not make it think it still owes a restore.
        assert_eq!(routing.restore("laptop"), None);
    }

    #[test]
    fn switching_devices_routes_to_the_new_one() {
        let mut routing = Routing::default();
        routing.apply("tv", "laptop");
        assert_eq!(routing.apply("receiver", "laptop"), Some("receiver".into()));
        assert_eq!(routing.restore("laptop"), Some("laptop".into()));
    }
}
