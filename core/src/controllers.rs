//! Controller detection and live input testing.
//!
//! The kernel (xpad, hid-playstation, hid-nintendo, hid-sony…) and SDL/Proton do the real
//! work of making pads function in games. This module's job is to *find* any gamepad,
//! explain what's wrong when one can't be used, show its battery, and let the user
//! verify it by pressing buttons.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, OpenOptions},
    io::{ErrorKind, Read},
    os::unix::fs::OpenOptionsExt,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::Sender,
        Arc,
    },
    time::Duration,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Connection {
    Usb,
    Bluetooth,
    Other,
}

impl Connection {
    pub fn label(self) -> &'static str {
        match self {
            Connection::Usb => "Wired / USB",
            Connection::Bluetooth => "Wireless (Bluetooth)",
            Connection::Other => "Connected",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Xbox,
    PlayStation,
    Nintendo,
    Generic,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Kind::Xbox => "Xbox",
            Kind::PlayStation => "PlayStation",
            Kind::Nintendo => "Nintendo",
            Kind::Generic => "Gamepad",
        }
    }

    fn detect(vendor: u16, name: &str) -> Kind {
        let n = name.to_ascii_lowercase();
        if vendor == 0x054c || ["dualsense", "dualshock", "playstation", "sony"].iter().any(|k| n.contains(k)) {
            Kind::PlayStation
        } else if vendor == 0x057e || ["nintendo", "joy-con", "switch pro"].iter().any(|k| n.contains(k)) {
            Kind::Nintendo
        } else if vendor == 0x045e || ["xbox", "x-box", "xinput"].iter().any(|k| n.contains(k)) {
            Kind::Xbox
        } else {
            Kind::Generic
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Battery {
    pub percent: u8,
    pub charging: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Controller {
    pub name: String,
    pub vendor: u16,
    pub product: u16,
    pub uniq: String,
    /// Name exactly as the kernel reports it (may have trailing spaces; SDL's GUID depends on it).
    pub raw_name: String,
    pub bus: u16,
    pub version: u16,
    pub caps: crate::mapping::Caps,
    pub kind: Kind,
    pub connection: Connection,
    /// `/dev/input/eventN`, used for the live test.
    pub event_path: Option<PathBuf>,
    pub battery: Option<Battery>,
    /// Plain-language problems/advice for this controller (empty = all good).
    pub hints: Vec<String>,
}

/// All gamepads the kernel currently exposes, with battery and hints filled in.
pub fn detect() -> Vec<Controller> {
    let mut pads = parse_proc_devices(&fs::read_to_string("/proc/bus/input/devices").unwrap_or_default());
    for p in &mut pads {
        p.battery = read_battery(&p.uniq);
        p.hints = hints_for(p);
    }
    pads
}

/// Is bit `bit` set in a /proc/bus/input/devices bitmap (space separated 64-bit hex words,
/// most significant word first)?
fn bit_set(words: &str, bit: usize) -> bool {
    let ws: Vec<&str> = words.split_whitespace().collect();
    let idx = bit / 64;
    if idx >= ws.len() {
        return false;
    }
    let w = u64::from_str_radix(ws[ws.len() - 1 - idx], 16).unwrap_or(0);
    (w >> (bit % 64)) & 1 == 1
}

const BTN_TRIGGER: usize = 0x120; // joysticks
const BTN_SOUTH: usize = 0x130; // gamepads ("A" / "✕")

pub fn parse_proc_devices(text: &str) -> Vec<Controller> {
    let mut out = Vec::new();
    for block in text.split("\n\n") {
        let (mut name, mut uniq, mut bus, mut vendor, mut product) = (None, String::new(), None, 0u16, 0u16);
        let (mut event, mut has_js, mut keys) = (None, false, String::new());
        let (mut raw_name, mut version, mut abs) = (String::new(), 0u16, String::new());
        for line in block.lines() {
            if let Some(r) = line.strip_prefix("N: Name=") {
                let unquoted = r.trim_end_matches('\n').trim_start_matches('"').trim_end_matches('"');
                raw_name = unquoted.to_string();
                name = Some(unquoted.trim().to_string());
            } else if let Some(r) = line.strip_prefix("I: Bus=") {
                for (i, tok) in r.split_whitespace().enumerate() {
                    if i == 0 {
                        bus = Some(tok.to_string());
                    } else if let Some(v) = tok.strip_prefix("Vendor=") {
                        vendor = u16::from_str_radix(v, 16).unwrap_or(0);
                    } else if let Some(p) = tok.strip_prefix("Product=") {
                        product = u16::from_str_radix(p, 16).unwrap_or(0);
                    } else if let Some(v) = tok.strip_prefix("Version=") {
                        version = u16::from_str_radix(v, 16).unwrap_or(0);
                    }
                }
            } else if let Some(r) = line.strip_prefix("U: Uniq=") {
                uniq = r.trim().to_string();
            } else if let Some(r) = line.strip_prefix("H: Handlers=") {
                for h in r.split_whitespace() {
                    if h.starts_with("event") {
                        event = Some(PathBuf::from("/dev/input").join(h));
                    } else if h.starts_with("js") {
                        has_js = true;
                    }
                }
            } else if let Some(r) = line.strip_prefix("B: KEY=") {
                keys = r.to_string();
            } else if let Some(r) = line.strip_prefix("B: ABS=") {
                abs = r.to_string();
            }
        }
        // A gamepad has gamepad/joystick buttons. This also skips the extra "Motion Sensors"
        // and "Touchpad" input nodes that PlayStation controllers create.
        let is_pad = has_js || (event.is_some() && (bit_set(&keys, BTN_SOUTH) || bit_set(&keys, BTN_TRIGGER)));
        if let (true, Some(name)) = (is_pad, name) {
            out.push(Controller {
                kind: Kind::detect(vendor, &name),
                name,
                vendor,
                product,
                uniq,
                raw_name,
                bus: match bus.as_deref() {
                    Some(b) => u16::from_str_radix(b, 16).unwrap_or(0),
                    None => 0,
                },
                version,
                caps: crate::mapping::Caps::from_bitmaps(&keys, &abs),
                connection: match bus.as_deref() {
                    Some("0003") => Connection::Usb,
                    Some("0005") => Connection::Bluetooth,
                    _ => Connection::Other,
                },
                event_path: event,
                battery: None,
                hints: vec![],
            });
        }
    }
    out
}

fn read_battery(uniq: &str) -> Option<Battery> {
    if uniq.len() < 6 {
        return None;
    }
    let uniq = uniq.to_ascii_lowercase();
    for e in fs::read_dir("/sys/class/power_supply").ok()?.flatten() {
        if e.file_name().to_string_lossy().to_ascii_lowercase().contains(&uniq) {
            let percent = fs::read_to_string(e.path().join("capacity")).ok()?.trim().parse::<u8>().ok()?;
            let status = fs::read_to_string(e.path().join("status")).unwrap_or_default();
            return Some(Battery { percent, charging: matches!(status.trim(), "Charging" | "Full") });
        }
    }
    None
}

fn hints_for(c: &Controller) -> Vec<String> {
    let mut h = Vec::new();
    if let Some(p) = &c.event_path {
        if let Err(e) = File::open(p) {
            if e.kind() == ErrorKind::PermissionDenied {
                h.push(
                    "Games may not be able to use this controller (no permission). Install the controller access rules with: sudo apt install steam-devices — then unplug and re-plug it."
                        .to_string(),
                );
            }
        }
    }
    if let Some(b) = &c.battery {
        if b.percent <= 15 && !b.charging {
            h.push(format!("Battery is low ({}%). Charge it soon to avoid dropping out mid-game.", b.percent));
        }
    }
    h
}

// ---------------------------------------------------------------------------------------
// Live input test
// ---------------------------------------------------------------------------------------

pub const EV_KEY: u16 = 1;
pub const EV_ABS: u16 = 3;

pub const ABS_X: u16 = 0x00;
pub const ABS_Y: u16 = 0x01;
pub const ABS_Z: u16 = 0x02;
pub const ABS_RX: u16 = 0x03;
pub const ABS_RY: u16 = 0x04;
pub const ABS_RZ: u16 = 0x05;
pub const ABS_HAT0X: u16 = 0x10;
pub const ABS_HAT0Y: u16 = 0x11;

/// `struct input_event` on 64-bit Linux: 16-byte timestamp, then type, code, value.
pub fn parse_events(buf: &[u8]) -> Vec<(u16, u16, i32)> {
    #[allow(clippy::chunks_exact_to_as_chunks)] // `as_chunks` needs a newer Rust than we support
    buf.chunks_exact(24)
        .map(|c| {
            (
                u16::from_le_bytes([c[16], c[17]]),
                u16::from_le_bytes([c[18], c[19]]),
                i32::from_le_bytes([c[20], c[21], c[22], c[23]]),
            )
        })
        .collect()
}

#[derive(Clone, Debug, Default)]
pub struct PadState {
    pub buttons: BTreeSet<u16>,
    pub axes: BTreeMap<u16, i32>,
    /// Every button code that has been pressed at least once (for pads with unusual layouts).
    pub seen_buttons: BTreeSet<u16>,
    max_seen: BTreeMap<u16, i32>,
    /// Real (min, max) of each axis as reported by the device, when known.
    ranges: BTreeMap<u16, (i32, i32)>,
}

impl PadState {
    /// Start from the device's real axis ranges and current positions (min, max, value), so
    /// sticks show as centred before anything has moved.
    pub fn with_axes(axes: &std::collections::HashMap<u16, (i32, i32, i32)>) -> Self {
        let mut s = Self::default();
        for (&code, &(min, max, value)) in axes {
            s.ranges.insert(code, (min, max));
            s.axes.insert(code, value);
        }
        s
    }

    pub fn apply(&mut self, ev_type: u16, code: u16, value: i32) {
        match ev_type {
            EV_KEY => {
                if value != 0 {
                    self.seen_buttons.insert(code);
                    self.buttons.insert(code);
                } else {
                    self.buttons.remove(&code);
                }
            }
            EV_ABS => {
                self.axes.insert(code, value);
                let m = self.max_seen.entry(code).or_insert(0);
                *m = (*m).max(value.abs());
            }
            _ => {}
        }
    }

    pub fn pressed(&self, code: u16) -> bool {
        self.buttons.contains(&code)
    }

    /// Stick position -1.0..1.0. PlayStation pads report 0..255; Xbox-style pads ±32768.
    /// For unknown pads the range is guessed from the largest value seen.
    pub fn stick(&self, kind: Kind, x: u16, y: u16) -> (f32, f32) {
        let norm = |code: u16| -> f32 {
            if let Some(&(min, max)) = self.ranges.get(&code) {
                let v = self.axes.get(&code).copied().unwrap_or((min + max) / 2) as f32;
                let mid = (min + max) as f32 / 2.0;
                return ((v - mid) / ((max - min) as f32 / 2.0).max(1.0)).clamp(-1.0, 1.0);
            }
            let v = *self.axes.get(&code).unwrap_or(&0) as f32;
            let max = *self.max_seen.get(&code).unwrap_or(&0);
            let byte_range = kind == Kind::PlayStation || (kind == Kind::Generic && max > 0 && max <= 255);
            if byte_range {
                (v - 128.0) / 128.0
            } else {
                v / 32768.0
            }
        };
        (norm(x).clamp(-1.0, 1.0), norm(y).clamp(-1.0, 1.0))
    }

    /// Trigger 0.0..1.0 using the largest value seen as full press.
    pub fn trigger(&self, code: u16) -> f32 {
        if let Some(&(min, max)) = self.ranges.get(&code) {
            let v = self.axes.get(&code).copied().unwrap_or(min) as f32;
            return ((v - min as f32) / ((max - min) as f32).max(1.0)).clamp(0.0, 1.0);
        }
        let v = *self.axes.get(&code).unwrap_or(&0) as f32;
        let max = (*self.max_seen.get(&code).unwrap_or(&0)).max(255) as f32;
        (v / max).clamp(0.0, 1.0)
    }

    /// D-pad direction from the hat axes: (-1|0|1, -1|0|1).
    pub fn dpad(&self) -> (i32, i32) {
        (*self.axes.get(&ABS_HAT0X).unwrap_or(&0), *self.axes.get(&ABS_HAT0Y).unwrap_or(&0))
    }
}

/// Human label for a button, matching what's printed on that kind of controller.
pub fn button_label(kind: Kind, code: u16) -> Option<&'static str> {
    let ps = kind == Kind::PlayStation;
    Some(match code {
        0x130 => {
            if ps {
                "✕"
            } else {
                "A"
            }
        }
        0x131 => {
            if ps {
                "○"
            } else {
                "B"
            }
        }
        0x133 => {
            if ps {
                "△"
            } else {
                "X"
            }
        }
        0x134 => {
            if ps {
                "□"
            } else {
                "Y"
            }
        }
        0x136 => {
            if ps {
                "L1"
            } else {
                "LB"
            }
        }
        0x137 => {
            if ps {
                "R1"
            } else {
                "RB"
            }
        }
        0x138 => {
            if ps {
                "L2"
            } else {
                "LT"
            }
        }
        0x139 => {
            if ps {
                "R2"
            } else {
                "RT"
            }
        }
        0x13a => {
            if ps {
                "Share"
            } else {
                "Back"
            }
        }
        0x13b => {
            if ps {
                "Options"
            } else {
                "Start"
            }
        }
        0x13c => {
            if ps {
                "PS"
            } else {
                "Guide"
            }
        }
        0x13d => "L3",
        0x13e => "R3",
        _ => return None,
    })
}

/// Name for any button: the printed label if it's a standard gamepad button, otherwise
/// "Button N" (older/generic joysticks use codes 0x120.. for their buttons).
pub fn button_name(kind: Kind, code: u16) -> String {
    match button_label(kind, code) {
        Some(l) => l.to_string(),
        None if (0x120..0x130).contains(&code) => format!("Button {}", code - 0x120 + 1),
        None => format!("Button {code:#x}"),
    }
}

/// Read events from `path` on a background thread until `stop` is set (or the pad is
/// unplugged). `on_event` is called after each batch so a UI can repaint.
pub fn watch(
    path: PathBuf,
    stop: Arc<AtomicBool>,
    tx: Sender<(u16, u16, i32)>,
    on_event: impl Fn() + Send + 'static,
) -> std::io::Result<()> {
    // Non-blocking so the thread notices `stop` even when the pad is idle.
    let mut f = OpenOptions::new().read(true).custom_flags(0o4000).open(path)?;
    std::thread::spawn(move || {
        let mut buf = [0u8; 24 * 64];
        while !stop.load(Ordering::Relaxed) {
            match f.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    for ev in parse_events(&buf[..n]) {
                        if tx.send(ev).is_err() {
                            return;
                        }
                    }
                    on_event();
                }
                Err(e) if e.kind() == ErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(5)),
                Err(_) => break, // unplugged
            }
        }
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"I: Bus=0019 Vendor=0000 Product=0001 Version=0000
N: Name="Power Button"
H: Handlers=kbd event0
B: KEY=10000000000000 0

I: Bus=0003 Vendor=046d Product=c077 Version=0111
N: Name="Logitech USB Optical Mouse"
H: Handlers=mouse0 event3
B: KEY=1f0000 0 0 0 0

I: Bus=0003 Vendor=045e Product=028e Version=0110
N: Name="Microsoft X-Box 360 pad"
U: Uniq=
H: Handlers=event12 js0
B: KEY=7cdb000000000000 0 0 0 0

I: Bus=0003 Vendor=054c Product=0ce6 Version=8111
N: Name="Sony Interactive Entertainment DualSense Wireless Controller"
U: Uniq=e8:47:3a:11:22:33
H: Handlers=event15 js1
B: KEY=7fdb000000000000 0 0 0 0

I: Bus=0003 Vendor=054c Product=0ce6 Version=8111
N: Name="Sony Interactive Entertainment DualSense Wireless Controller Motion Sensors"
H: Handlers=event16
B: KEY=0

I: Bus=0003 Vendor=054c Product=0ce6 Version=8111
N: Name="Sony Interactive Entertainment DualSense Wireless Controller Touchpad"
H: Handlers=mouse2 event17
B: KEY=e520 10000 0 0 0

I: Bus=0005 Vendor=045e Product=0b13 Version=0517
N: Name="Xbox Wireless Controller"
U: Uniq=aa:bb:cc:dd:ee:ff
H: Handlers=event20
B: KEY=7fff000000000000 0 0 0 0
"#;

    #[test]
    fn finds_all_pads_and_nothing_else() {
        let c = parse_proc_devices(SAMPLE);
        let names: Vec<_> = c.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(c.len(), 3, "{names:?}");
        assert_eq!((c[0].kind, c[0].connection), (Kind::Xbox, Connection::Usb));
        assert_eq!((c[1].kind, c[1].connection), (Kind::PlayStation, Connection::Usb));
        assert_eq!((c[2].kind, c[2].connection), (Kind::Xbox, Connection::Bluetooth));
        assert_eq!(c[1].uniq, "e8:47:3a:11:22:33");
        assert_eq!(c[2].event_path.as_deref().unwrap().to_str(), Some("/dev/input/event20"));
    }

    #[test]
    fn bitmaps() {
        assert!(bit_set("7cdb000000000000 0 0 0 0", BTN_SOUTH));
        assert!(!bit_set("1f0000 0 0 0 0", BTN_SOUTH));
        assert!(!bit_set("0", BTN_SOUTH));
    }

    #[test]
    fn kind_from_vendor_or_name() {
        assert_eq!(Kind::detect(0x2dc8, "8BitDo Pro 2"), Kind::Generic);
        assert_eq!(Kind::detect(0x0000, "My DualShock 4"), Kind::PlayStation);
        assert_eq!(Kind::detect(0x057e, "Pro Controller"), Kind::Nintendo);
    }

    fn event(t: u16, c: u16, v: i32) -> Vec<u8> {
        let mut b = vec![0u8; 16];
        b.extend(t.to_le_bytes());
        b.extend(c.to_le_bytes());
        b.extend(v.to_le_bytes());
        b
    }

    #[test]
    fn live_state_from_raw_events() {
        let mut raw = event(EV_KEY, 0x130, 1);
        raw.extend(event(EV_ABS, ABS_X, 16384));
        raw.extend(event(EV_ABS, ABS_HAT0Y, -1));
        let evs = parse_events(&raw);
        assert_eq!(evs.len(), 3);
        let mut s = PadState::default();
        for (t, c, v) in evs {
            s.apply(t, c, v);
        }
        assert!(s.pressed(0x130));
        assert_eq!(s.dpad(), (0, -1));
        let (x, y) = s.stick(Kind::Xbox, ABS_X, ABS_Y);
        assert!((x - 0.5).abs() < 0.01 && y == 0.0);
        s.apply(EV_KEY, 0x130, 0);
        assert!(!s.pressed(0x130));
    }

    #[test]
    fn watch_reads_raw_events_from_a_device_file() {
        // A plain file stands in for /dev/input/eventN: same binary format, same read path.
        let dir = std::env::temp_dir().join(format!("wb-watch-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("event99");
        let mut raw = event(EV_KEY, 0x131, 1);
        raw.extend(event(EV_ABS, ABS_RX, -20000));
        std::fs::write(&path, raw).unwrap();

        let (tx, rx) = std::sync::mpsc::channel();
        watch(path, Arc::new(AtomicBool::new(false)), tx, || {}).unwrap();
        let got: Vec<_> = (0..2).map(|_| rx.recv_timeout(Duration::from_secs(2)).unwrap()).collect();
        assert_eq!(got, vec![(EV_KEY, 0x131, 1), (EV_ABS, ABS_RX, -20000)]);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn sticks_start_centred_using_real_ranges() {
        // A PlayStation-style pad: 0..255, resting at 128. Before any event the stick must read ~0, not -1.
        let ranges = [(ABS_X, (0, 255, 128)), (ABS_Y, (0, 255, 128)), (ABS_Z, (0, 255, 0))].into_iter().collect();
        let mut s = PadState::with_axes(&ranges);
        let (x, y) = s.stick(Kind::PlayStation, ABS_X, ABS_Y);
        assert!(x.abs() < 0.02 && y.abs() < 0.02, "({x},{y})");
        assert_eq!(s.trigger(ABS_Z), 0.0);
        s.apply(EV_ABS, ABS_X, 255);
        assert!(s.stick(Kind::PlayStation, ABS_X, ABS_Y).0 > 0.98);
        s.apply(EV_ABS, ABS_Z, 255);
        assert_eq!(s.trigger(ABS_Z), 1.0);
        // Xbox-style signed range works too.
        let x360 = [(ABS_X, (-32768, 32767, 0))].into_iter().collect();
        let mut s = PadState::with_axes(&x360);
        s.apply(EV_ABS, ABS_X, -32768);
        assert!(s.stick(Kind::Xbox, ABS_X, ABS_Y).0 < -0.99);
    }

    #[test]
    fn playstation_sticks_use_byte_range() {
        let mut s = PadState::default();
        s.apply(EV_ABS, ABS_X, 255);
        s.apply(EV_ABS, ABS_Y, 0);
        let (x, y) = s.stick(Kind::PlayStation, ABS_X, ABS_Y);
        assert!(x > 0.98 && y < -0.99);
    }

    #[test]
    fn labels_match_controller_type() {
        assert_eq!(button_label(Kind::Xbox, 0x130), Some("A"));
        assert_eq!(button_label(Kind::PlayStation, 0x130), Some("✕"));
        assert_eq!(button_label(Kind::Xbox, 0x999), None);
        assert_eq!(button_name(Kind::Generic, 0x122), "Button 3");
        assert_eq!(button_name(Kind::Xbox, 0x131), "B");
    }
}
