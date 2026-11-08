//! Optional lookup of SDL's built-in controller database.
//!
//! Games see controllers through SDL, so "what does SDL think this pad is?" is the most
//! truthful answer for drawing it. SDL is loaded at runtime (dlopen); if it isn't installed
//! the lookup simply returns `None`.

use std::{
    ffi::{c_char, c_void, CStr, CString},
    sync::Mutex,
};

/// SDL_Init/SDL_Quit are process-global, so only one lookup at a time.
static LOCK: Mutex<()> = Mutex::new(());

#[repr(C)]
#[derive(Clone, Copy)]
struct Guid {
    data: [u8; 16],
}

const SDL_INIT_GAMECONTROLLER: u32 = 0x0000_2000;

/// SDL's mapping line for this joystick GUID (32 hex characters), if SDL knows the device.
pub fn builtin_mapping(guid: &str) -> Option<String> {
    let _guard = LOCK.lock().ok()?;
    let cguid = CString::new(guid).ok()?;
    // SAFETY: we call documented SDL2 C functions with matching signatures, free what SDL
    // allocates, and shut SDL down again before returning.
    unsafe {
        let lib = ["libSDL2-2.0.so.0", "libSDL2.so"].iter().find_map(|n| libloading::Library::new(*n).ok())?;
        let init: libloading::Symbol<unsafe extern "C" fn(u32) -> i32> = lib.get(b"SDL_Init\0").ok()?;
        let quit: libloading::Symbol<unsafe extern "C" fn()> = lib.get(b"SDL_Quit\0").ok()?;
        let from_string: libloading::Symbol<unsafe extern "C" fn(*const c_char) -> Guid> =
            lib.get(b"SDL_JoystickGetGUIDFromString\0").ok()?;
        let mapping_for: libloading::Symbol<unsafe extern "C" fn(Guid) -> *mut c_char> =
            lib.get(b"SDL_GameControllerMappingForGUID\0").ok()?;
        let free: libloading::Symbol<unsafe extern "C" fn(*mut c_void)> = lib.get(b"SDL_free\0").ok()?;

        if init(SDL_INIT_GAMECONTROLLER) != 0 {
            return None;
        }
        let raw = mapping_for(from_string(cguid.as_ptr()));
        let out = if raw.is_null() {
            None
        } else {
            let s = CStr::from_ptr(raw).to_string_lossy().into_owned();
            free(raw as *mut c_void);
            Some(s)
        };
        quit();
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn knows_xbox_360_pad_if_sdl_is_installed() {
        // GUID of a USB Xbox 360 pad (vendor 045e, product 028e).
        match builtin_mapping("030000005e0400008e02000014010000") {
            Some(m) => assert!(m.contains("a:b0") && m.contains("platform:Linux"), "{m}"),
            None => eprintln!("SDL2 not installed here; skipping"),
        }
    }

    #[test]
    fn unknown_device_gives_none() {
        assert!(builtin_mapping("ffffffffffffffffffffffffffffffff").is_none());
    }
}
