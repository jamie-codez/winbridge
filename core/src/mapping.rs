//! Button mapping for controllers that games don't recognise.
//!
//! Games (through Proton) read controllers via SDL. SDL ships a database of known pads;
//! for anything else it treats the device as a raw joystick and games guess wrong. This
//! module runs a "press the button for A… B…" wizard and produces an SDL mapping string
//! that WinBridge hands to games through `SDL_GAMECONTROLLERCONFIG`.
//!
//! The GUID, button numbering and axis numbering follow SDL2's Linux evdev backend and were
//! checked against a real SDL2 instance (see the tests).

use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fs::{self, File},
    os::fd::AsRawFd,
    path::{Path, PathBuf},
};

use anyhow::Result;

use crate::{
    controllers::{EV_ABS, EV_KEY},
    paths::Paths,
};

// ---------------------------------------------------------------------------------------
// Device capabilities and SDL numbering
// ---------------------------------------------------------------------------------------

/// Which button and axis codes a device has, parsed from /proc/bus/input/devices bitmaps.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Caps {
    pub keys: Vec<u16>,
    pub abs: Vec<u16>,
}

/// All set bit positions of a bitmap of space-separated 64-bit hex words (most significant first).
pub fn bits(words: &str) -> Vec<u16> {
    let ws: Vec<&str> = words.split_whitespace().collect();
    let mut out = Vec::new();
    for (i, w) in ws.iter().rev().enumerate() {
        let v = u64::from_str_radix(w, 16).unwrap_or(0);
        for b in 0..64 {
            if (v >> b) & 1 == 1 {
                out.push((i * 64 + b) as u16);
            }
        }
    }
    out
}

const BTN_JOYSTICK: u16 = 0x120;
const ABS_HAT0X: u16 = 0x10;
const ABS_HAT3Y: u16 = 0x17;

impl Caps {
    pub fn from_bitmaps(keys: &str, abs: &str) -> Self {
        Self { keys: bits(keys), abs: bits(abs) }
    }

    /// SDL numbers buttons: codes from BTN_JOYSTICK upward first, then the lower codes.
    pub fn button_index(&self, code: u16) -> Option<usize> {
        let mut order: Vec<u16> = self.keys.iter().copied().filter(|c| *c >= BTN_JOYSTICK).collect();
        order.extend(self.keys.iter().copied().filter(|c| *c < BTN_JOYSTICK));
        order.iter().position(|c| *c == code)
    }

    /// Inverse of `button_index`: the evdev code behind SDL button number `idx`.
    pub fn button_code(&self, idx: usize) -> Option<u16> {
        let mut order: Vec<u16> = self.keys.iter().copied().filter(|c| *c >= BTN_JOYSTICK).collect();
        order.extend(self.keys.iter().copied().filter(|c| *c < BTN_JOYSTICK));
        order.get(idx).copied()
    }

    /// Inverse of `axis_index`.
    pub fn axis_code(&self, idx: usize) -> Option<u16> {
        self.abs.iter().copied().filter(|c| !(ABS_HAT0X..=ABS_HAT3Y).contains(c)).nth(idx)
    }

    /// SDL numbers axes in ascending code order, with the hat axes (0x10–0x17) excluded.
    pub fn axis_index(&self, code: u16) -> Option<usize> {
        self.abs.iter().copied().filter(|c| !(ABS_HAT0X..=ABS_HAT3Y).contains(c)).position(|c| c == code)
    }
}

fn crc16(data: &[u8]) -> u16 {
    let mut crc = 0u16;
    for &b in data {
        crc ^= b as u16;
        for _ in 0..8 {
            crc = if crc & 1 == 1 { (crc >> 1) ^ 0xA001 } else { crc >> 1 };
        }
    }
    crc
}

/// SDL2's joystick GUID. `raw_name` must be the name exactly as the kernel reports it
/// (including any trailing spaces).
pub fn sdl_guid(bus: u16, vendor: u16, product: u16, version: u16, raw_name: &str) -> String {
    let mut b = Vec::with_capacity(16);
    for v in [bus, crc16(raw_name.as_bytes()), vendor, 0, product, 0, version, 0] {
        b.extend(v.to_le_bytes());
    }
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// (minimum, maximum, current value) of each axis, via the EVIOCGABS ioctl.
pub fn read_abs_ranges(event_path: &Path, codes: &[u16]) -> HashMap<u16, (i32, i32, i32)> {
    let mut out = HashMap::new();
    let Ok(f) = File::open(event_path) else { return out };
    for &code in codes {
        let mut info = [0i32; 6]; // value, minimum, maximum, fuzz, flat, resolution
                                  // _IOR('E', 0x40 + code, struct input_absinfo): 2<<30 | 24<<16 | 'E'<<8 | nr
        let req: u64 = (2u64 << 30) | (24 << 16) | (0x45 << 8) | (0x40 + code as u64);
        // SAFETY: valid fd, and `info` is a 24-byte buffer exactly matching struct input_absinfo.
        let rc = unsafe { libc::ioctl(f.as_raw_fd(), req as _, info.as_mut_ptr()) };
        if rc == 0 {
            out.insert(code, (info[1], info[2], info[0]));
        }
    }
    out
}

// ---------------------------------------------------------------------------------------
// The wizard
// ---------------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    A,
    B,
    X,
    Y,
    LeftShoulder,
    RightShoulder,
    LeftTrigger,
    RightTrigger,
    Back,
    Start,
    Guide,
    LeftStick,
    RightStick,
    DpadUp,
    DpadDown,
    DpadLeft,
    DpadRight,
    LeftStickRight,
    LeftStickDown,
    RightStickRight,
    RightStickDown,
}

impl Target {
    pub const ALL: [Target; 21] = [
        Target::A,
        Target::B,
        Target::X,
        Target::Y,
        Target::LeftShoulder,
        Target::RightShoulder,
        Target::LeftTrigger,
        Target::RightTrigger,
        Target::Back,
        Target::Start,
        Target::Guide,
        Target::LeftStick,
        Target::RightStick,
        Target::DpadUp,
        Target::DpadDown,
        Target::DpadLeft,
        Target::DpadRight,
        Target::LeftStickRight,
        Target::LeftStickDown,
        Target::RightStickRight,
        Target::RightStickDown,
    ];

    pub fn prompt(self) -> &'static str {
        match self {
            Target::A => "Press the BOTTOM face button (A on Xbox, ✕ on PlayStation)",
            Target::B => "Press the RIGHT face button (B on Xbox, ○ on PlayStation)",
            Target::X => "Press the LEFT face button (X on Xbox, □ on PlayStation)",
            Target::Y => "Press the TOP face button (Y on Xbox, △ on PlayStation)",
            Target::LeftShoulder => "Press the LEFT bumper (LB / L1), the button on top-left",
            Target::RightShoulder => "Press the RIGHT bumper (RB / R1), the button on top-right",
            Target::LeftTrigger => "Squeeze the LEFT trigger (LT / L2) fully",
            Target::RightTrigger => "Squeeze the RIGHT trigger (RT / R2) fully",
            Target::Back => "Press BACK / SELECT / SHARE (small button, left of centre)",
            Target::Start => "Press START / OPTIONS (small button, right of centre)",
            Target::Guide => "Press the HOME / GUIDE / PS button (skip if there isn't one)",
            Target::LeftStick => "Click the LEFT stick straight down (L3)",
            Target::RightStick => "Click the RIGHT stick straight down (R3)",
            Target::DpadUp => "Press D-pad UP",
            Target::DpadDown => "Press D-pad DOWN",
            Target::DpadLeft => "Press D-pad LEFT",
            Target::DpadRight => "Press D-pad RIGHT",
            Target::LeftStickRight => "Push the LEFT stick all the way to the RIGHT",
            Target::LeftStickDown => "Push the LEFT stick all the way DOWN",
            Target::RightStickRight => "Push the RIGHT stick all the way to the RIGHT",
            Target::RightStickDown => "Push the RIGHT stick all the way DOWN",
        }
    }

    pub fn sdl_name(self) -> &'static str {
        match self {
            Target::A => "a",
            Target::B => "b",
            Target::X => "x",
            Target::Y => "y",
            Target::LeftShoulder => "leftshoulder",
            Target::RightShoulder => "rightshoulder",
            Target::LeftTrigger => "lefttrigger",
            Target::RightTrigger => "righttrigger",
            Target::Back => "back",
            Target::Start => "start",
            Target::Guide => "guide",
            Target::LeftStick => "leftstick",
            Target::RightStick => "rightstick",
            Target::DpadUp => "dpup",
            Target::DpadDown => "dpdown",
            Target::DpadLeft => "dpleft",
            Target::DpadRight => "dpright",
            Target::LeftStickRight => "leftx",
            Target::LeftStickDown => "lefty",
            Target::RightStickRight => "rightx",
            Target::RightStickDown => "righty",
        }
    }

    fn is_stick_axis(self) -> bool {
        matches!(
            self,
            Target::LeftStickRight | Target::LeftStickDown | Target::RightStickRight | Target::RightStickDown
        )
    }
    fn is_trigger(self) -> bool {
        matches!(self, Target::LeftTrigger | Target::RightTrigger)
    }
    fn is_dpad(self) -> bool {
        matches!(self, Target::DpadUp | Target::DpadDown | Target::DpadLeft | Target::DpadRight)
    }
}

/// A physical input the user just operated.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Input {
    Button(u16),
    /// Axis code and whether it moved in the positive direction.
    Axis(u16, bool),
    /// Hat axis code and direction (-1 / +1).
    Hat(u16, i32),
}

impl Input {
    /// Identity for "already used": an axis is the same input whichever way it moves.
    fn key(self) -> (u8, u16) {
        match self {
            Input::Button(c) => (0, c),
            Input::Axis(c, _) => (1, c),
            Input::Hat(c, d) => (2, c * 4 + (d + 1) as u16),
        }
    }
}

pub struct Wizard {
    caps: Caps,
    ranges: HashMap<u16, (i32, i32, i32)>,
    step: usize,
    bindings: BTreeMap<&'static str, String>,
    used: HashSet<(u8, u16)>,
    /// After capturing an input, ignore everything until it is released.
    waiting_release: Option<Input>,
}

impl Wizard {
    pub fn new(caps: Caps, ranges: HashMap<u16, (i32, i32, i32)>) -> Self {
        Self { caps, ranges, step: 0, bindings: BTreeMap::new(), used: HashSet::new(), waiting_release: None }
    }

    pub fn current(&self) -> Option<Target> {
        Target::ALL.get(self.step).copied()
    }

    /// (steps done, total steps)
    pub fn progress(&self) -> (usize, usize) {
        (self.step, Target::ALL.len())
    }

    pub fn is_done(&self) -> bool {
        self.step >= Target::ALL.len()
    }

    pub fn skip(&mut self) {
        if !self.is_done() {
            self.step += 1;
            self.waiting_release = None;
        }
    }

    pub fn mapped_count(&self) -> usize {
        self.bindings.len()
    }

    /// Axis deflection from rest as a fraction of its range (-1..1), if the range is known.
    fn deflection(&self, code: u16, value: i32) -> Option<f32> {
        let &(min, max, rest) = self.ranges.get(&code)?;
        let span = (max - min).max(1) as f32;
        Some((value - rest) as f32 / span)
    }

    fn classify(&self, t: u16, c: u16, v: i32) -> Option<Input> {
        match t {
            EV_KEY if v == 1 && self.caps.keys.contains(&c) && c >= 0x100 => Some(Input::Button(c)),
            EV_ABS if (ABS_HAT0X..=ABS_HAT3Y).contains(&c) && v != 0 => Some(Input::Hat(c, v.signum())),
            EV_ABS if !(ABS_HAT0X..=ABS_HAT3Y).contains(&c) => {
                let d = self.deflection(c, v)?;
                (d.abs() > 0.4).then_some(Input::Axis(c, d > 0.0))
            }
            _ => None,
        }
    }

    fn released(&self, input: Input, t: u16, c: u16, v: i32) -> bool {
        match input {
            Input::Button(b) => t == EV_KEY && c == b && v == 0,
            Input::Hat(h, _) => t == EV_ABS && c == h && v == 0,
            Input::Axis(a, _) => t == EV_ABS && c == a && self.deflection(a, v).is_some_and(|d| d.abs() < 0.25),
        }
    }

    /// Feed one raw input event. Returns true when the wizard advanced to the next step.
    pub fn feed(&mut self, t: u16, c: u16, v: i32) -> bool {
        let Some(target) = self.current() else { return false };
        if let Some(w) = self.waiting_release {
            if self.released(w, t, c, v) {
                self.waiting_release = None;
            }
            return false;
        }
        let Some(input) = self.classify(t, c, v) else { return false };
        if self.used.contains(&input.key()) {
            return false;
        }
        let Some(value) = self.binding_for(target, input) else { return false };
        self.bindings.insert(target.sdl_name(), value);
        self.used.insert(input.key());
        self.waiting_release = Some(input);
        self.step += 1;
        true
    }

    /// SDL mapping value for `target` given the operated `input`, or None if that kind of
    /// input doesn't fit the target (e.g. a stick push when a button is wanted).
    fn binding_for(&self, target: Target, input: Input) -> Option<String> {
        match input {
            Input::Button(code) if !target.is_stick_axis() => Some(format!("b{}", self.caps.button_index(code)?)),
            Input::Hat(code, dir) if target.is_dpad() => {
                let mask = match (code, dir) {
                    (0x10, d) if d > 0 => 2,
                    (0x10, _) => 8,
                    (_, d) if d < 0 => 1,
                    _ => 4,
                };
                Some(format!("h{}.{}", (code - ABS_HAT0X) / 2, mask))
            }
            Input::Axis(code, positive) if target.is_stick_axis() || target.is_trigger() => {
                let idx = self.caps.axis_index(code)?;
                Some(format!("a{idx}{}", if positive { "" } else { "~" }))
            }
            _ => None,
        }
    }

    /// The finished SDL mapping line.
    pub fn mapping_line(&self, guid: &str, name: &str) -> String {
        let name = name.trim().replace(',', " ");
        let body: Vec<String> = self.bindings.iter().map(|(k, v)| format!("{k}:{v}")).collect();
        format!("{guid},{name},{},platform:Linux", body.join(","))
    }
}

// ---------------------------------------------------------------------------------------
// Saved mappings
// ---------------------------------------------------------------------------------------

fn dir(paths: &Paths) -> PathBuf {
    paths.config.join("controllers")
}

fn file(paths: &Paths, vendor: u16, product: u16) -> PathBuf {
    dir(paths).join(format!("{vendor:04x}-{product:04x}.map"))
}

pub fn save(paths: &Paths, vendor: u16, product: u16, line: &str) -> Result<()> {
    fs::create_dir_all(dir(paths))?;
    fs::write(file(paths, vendor, product), format!("{line}\n"))?;
    Ok(())
}

pub fn remove(paths: &Paths, vendor: u16, product: u16) -> Result<()> {
    let f = file(paths, vendor, product);
    if f.exists() {
        fs::remove_file(f)?;
    }
    Ok(())
}

/// The saved mapping line for a pad, if the user created one.
pub fn saved_line(paths: &Paths, vendor: u16, product: u16) -> Option<String> {
    fs::read_to_string(file(paths, vendor, product))
        .ok()
        .and_then(|s| s.lines().next().map(str::trim).map(str::to_string))
}

pub fn has_saved(paths: &Paths, vendor: u16, product: u16) -> bool {
    file(paths, vendor, product).exists()
}

/// All saved mappings, one per line, ready for `SDL_GAMECONTROLLERCONFIG`.
pub fn load_all(paths: &Paths) -> Vec<String> {
    let Ok(rd) = fs::read_dir(dir(paths)) else { return vec![] };
    let mut files: Vec<PathBuf> =
        rd.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "map")).collect();
    files.sort();
    files
        .iter()
        .filter_map(|f| fs::read_to_string(f).ok())
        .flat_map(|s| s.lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_string).collect::<Vec<_>>())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The real "Twin USB Gamepad" adapter: values taken from this machine's /proc and from
    /// a live SDL 2.30 instance (GUID, and its built-in mapping, used as the answer key).
    const KEYS: &str = "fff00000000 0 0 0 0";
    const ABS: &str = "30027";
    const RAW_NAME: &str = "Twin USB Gamepad      ";
    const SDL_GUID: &str = "03004a9f100800000100000010010000";

    #[test]
    fn guid_matches_real_sdl() {
        assert_eq!(sdl_guid(3, 0x0810, 0x0001, 0x0110, RAW_NAME), SDL_GUID);
    }

    #[test]
    fn numbering_matches_real_sdl() {
        let caps = Caps::from_bitmaps(KEYS, ABS);
        assert_eq!(caps.keys.len(), 12); // SDL reported 12 buttons
        assert_eq!(caps.button_index(0x122), Some(2)); // the button you pressed most = SDL b2
        assert_eq!(caps.axis_index(5), Some(3)); // ABS_RZ -> a3 (SDL: rightx:a3)
        assert_eq!(caps.axis_index(2), Some(2)); // ABS_Z  -> a2 (SDL: righty:a2)
        assert_eq!(caps.axis_index(0x10), None); // hats are not axes
        assert_eq!(caps.axis_index(1), Some(1));
    }

    fn ranges() -> HashMap<u16, (i32, i32, i32)> {
        [0u16, 1, 2, 5].iter().map(|c| (*c, (0, 255, 127))).collect()
    }

    fn press(w: &mut Wizard, code: u16) {
        assert!(w.feed(EV_KEY, code, 1), "press {code:#x} should advance");
        w.feed(EV_KEY, code, 0);
    }

    /// Operating this pad the way a person would reproduces SDL's own built-in mapping for it.
    #[test]
    fn wizard_reproduces_sdls_builtin_mapping() {
        let mut w = Wizard::new(Caps::from_bitmaps(KEYS, ABS), ranges());
        // Targets in order: A B X Y LB RB LT RT Back Start. On this pad the triggers are buttons.
        // (SDL: a:b2 b:b1 x:b3 y:b0 leftshoulder:b6 rightshoulder:b7 lefttrigger:b4 righttrigger:b5 back:b8 start:b9)
        for code in [0x122, 0x121, 0x123, 0x120, 0x126, 0x127, 0x124, 0x125, 0x128, 0x129] {
            press(&mut w, code);
        }
        w.skip(); // Guide: this pad has none
        press(&mut w, 0x12a); // left stick click
        press(&mut w, 0x12b); // right stick click
                              // D-pad via hat
        for (axis, val) in [(ABS_HAT0Y_T, -1), (ABS_HAT0Y_T, 1), (ABS_HAT0X_T, -1), (ABS_HAT0X_T, 1)] {
            assert!(w.feed(EV_ABS, axis, val));
            w.feed(EV_ABS, axis, 0);
        }
        // Sticks: left X (code 0), left Y (1), right X (ABS_RZ=5), right Y (ABS_Z=2)
        for axis in [0u16, 1, 5, 2] {
            assert!(w.feed(EV_ABS, axis, 255), "axis {axis}");
            w.feed(EV_ABS, axis, 127);
        }
        assert!(w.is_done());

        let line = w.mapping_line(SDL_GUID, RAW_NAME);
        let want = "a:b2,b:b1,back:b8,dpdown:h0.4,dpleft:h0.8,dpright:h0.2,dpup:h0.1,leftshoulder:b6,leftstick:b10,\
                    lefttrigger:b4,leftx:a0,lefty:a1,rightshoulder:b7,rightstick:b11,righttrigger:b5,rightx:a3,righty:a2,start:b9,x:b3,y:b0";
        assert_eq!(line, format!("{SDL_GUID},Twin USB Gamepad,{want},platform:Linux"));
    }
    const ABS_HAT0X_T: u16 = 0x10;
    const ABS_HAT0Y_T: u16 = 0x11;

    #[test]
    fn same_input_is_not_used_twice_and_wrong_kinds_are_ignored() {
        let mut w = Wizard::new(Caps::from_bitmaps(KEYS, ABS), ranges());
        press(&mut w, 0x122); // A
                              // Target is now B. Pressing A's button again must not advance:
        assert!(!w.feed(EV_KEY, 0x122, 1));
        // A stick push is the wrong kind of input for a button target:
        assert!(!w.feed(EV_ABS, 0, 255));
        // Small stick wobble is ignored while a stick is wanted later; here it's still a button step.
        press(&mut w, 0x121);
        assert_eq!(w.progress().0, 2);
    }

    #[test]
    fn inverted_axis_is_marked() {
        let mut w = Wizard::new(Caps::from_bitmaps(KEYS, ABS), ranges());
        for _ in 0..17 {
            w.skip(); // jump to the stick steps (index 17 = left stick right)
        }
        assert_eq!(w.current(), Some(Target::LeftStickRight));
        assert!(w.feed(EV_ABS, 0, 0)); // pushed to 0 (rest is 127): negative direction
        assert!(w.mapping_line("g", "n").contains("leftx:a0~"));
    }

    #[test]
    fn save_load_remove_roundtrip() {
        let d = tempfile::tempdir().unwrap();
        let paths = Paths { config: d.path().join("c"), data: d.path().join("d") };
        assert!(load_all(&paths).is_empty());
        save(&paths, 0x0810, 1, "guid,Name,a:b0,platform:Linux").unwrap();
        save(&paths, 0x045e, 2, "guid2,Other,a:b1,platform:Linux").unwrap();
        assert_eq!(load_all(&paths).len(), 2);
        assert!(has_saved(&paths, 0x0810, 1));
        remove(&paths, 0x0810, 1).unwrap();
        assert_eq!(load_all(&paths), vec!["guid2,Other,a:b1,platform:Linux".to_string()]);
    }
}
