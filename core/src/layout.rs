//! Where each physical input sits on a controller, so a UI can draw it and light up the
//! right part when something is pressed.
//!
//! Every pad is reduced to the same set of positions (`Part`). Standard Xbox / PlayStation /
//! Nintendo pads use fixed tables; anything else uses the layout games actually see: the
//! mapping the user saved in WinBridge, else SDL's built-in database.

use std::collections::HashMap;

use crate::{
    controllers::{Controller, Kind, PadState, ABS_HAT0X, ABS_HAT0Y, ABS_RX, ABS_RY, ABS_RZ, ABS_X, ABS_Y, ABS_Z},
    mapping::{self, Caps},
    paths::Paths,
    sdl,
};

/// A position on the controller. Face buttons are named by position (Xbox naming):
/// `A` = bottom, `B` = right, `X` = left, `Y` = top.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Part {
    A,
    B,
    X,
    Y,
    Back,
    Guide,
    Start,
    LeftStick,
    RightStick,
    LeftShoulder,
    RightShoulder,
    LeftTrigger,
    RightTrigger,
    DpadUp,
    DpadDown,
    DpadLeft,
    DpadRight,
}

/// Which raw input drives a part (evdev codes).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Button(u16),
    Axis(u16),
    Hat(u16, i32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    /// Fixed table for Xbox / PlayStation / Nintendo pads.
    Standard,
    /// The layout the user created with "Set up buttons".
    Saved,
    /// SDL's built-in knowledge of this device.
    Sdl,
    /// Nobody knows this pad's layout.
    Unknown,
}

#[derive(Clone, Debug)]
pub struct Layout {
    pub parts: HashMap<Part, Vec<Source>>,
    pub left_stick: Option<(u16, u16)>,
    pub right_stick: Option<(u16, u16)>,
    pub origin: Origin,
    /// Device name from the mapping source (e.g. "Twin USB PS2 Adapter"); used to pick the artwork.
    pub name: String,
}

impl Layout {
    pub fn unknown() -> Self {
        Self {
            parts: HashMap::new(),
            left_stick: None,
            right_stick: None,
            origin: Origin::Unknown,
            name: String::new(),
        }
    }

    /// Can this layout draw anything useful?
    pub fn is_known(&self) -> bool {
        self.origin != Origin::Unknown
    }

    pub fn has(&self, part: Part) -> bool {
        self.parts.get(&part).is_some_and(|s| !s.is_empty())
    }

    /// Fixed layout of standard pads as the Linux kernel drivers report them.
    pub fn standard(kind: Kind) -> Self {
        use Part::*;
        let ps_like = matches!(kind, Kind::PlayStation | Kind::Nintendo);
        // xpad: 0x133 = X (left), 0x134 = Y (top). hid-playstation / hid-nintendo: 0x133 = top, 0x134 = left.
        let (top, left) = if ps_like { (0x133, 0x134) } else { (0x134, 0x133) };
        let b = |c: u16| vec![Source::Button(c)];
        let mut parts = HashMap::new();
        parts.insert(A, b(0x130));
        parts.insert(B, b(0x131));
        parts.insert(Y, b(top));
        parts.insert(X, b(left));
        parts.insert(LeftShoulder, b(0x136));
        parts.insert(RightShoulder, b(0x137));
        parts.insert(LeftTrigger, vec![Source::Axis(ABS_Z), Source::Button(0x138)]);
        parts.insert(RightTrigger, vec![Source::Axis(ABS_RZ), Source::Button(0x139)]);
        parts.insert(Back, b(0x13a));
        parts.insert(Start, b(0x13b));
        parts.insert(Guide, b(0x13c));
        parts.insert(LeftStick, b(0x13d));
        parts.insert(RightStick, b(0x13e));
        parts.insert(DpadUp, vec![Source::Hat(ABS_HAT0Y, -1)]);
        parts.insert(DpadDown, vec![Source::Hat(ABS_HAT0Y, 1)]);
        parts.insert(DpadLeft, vec![Source::Hat(ABS_HAT0X, -1)]);
        parts.insert(DpadRight, vec![Source::Hat(ABS_HAT0X, 1)]);
        Self {
            parts,
            left_stick: Some((ABS_X, ABS_Y)),
            right_stick: Some((ABS_RX, ABS_RY)),
            origin: Origin::Standard,
            name: String::new(),
        }
    }

    /// Build a layout from an SDL mapping line (`guid,name,a:b2,leftx:a0,dpup:h0.1,…`).
    pub fn from_sdl_mapping(line: &str, caps: &Caps, origin: Origin) -> Option<Self> {
        use Part::*;
        let name = line.split(',').nth(1).unwrap_or("").to_string();
        let mut layout = Self { parts: HashMap::new(), left_stick: None, right_stick: None, origin, name };
        let (mut lx, mut ly, mut rx, mut ry) = (None, None, None, None);
        for field in line.split(',').skip(2) {
            let Some((name, value)) = field.split_once(':') else { continue };
            let Some(src) = parse_binding(value, caps) else { continue };
            let part = match name {
                "a" => A,
                "b" => B,
                "x" => X,
                "y" => Y,
                "back" => Back,
                "guide" => Guide,
                "start" => Start,
                "leftstick" => LeftStick,
                "rightstick" => RightStick,
                "leftshoulder" => LeftShoulder,
                "rightshoulder" => RightShoulder,
                "lefttrigger" => LeftTrigger,
                "righttrigger" => RightTrigger,
                "dpup" => DpadUp,
                "dpdown" => DpadDown,
                "dpleft" => DpadLeft,
                "dpright" => DpadRight,
                "leftx" => {
                    lx = axis_code(src);
                    continue;
                }
                "lefty" => {
                    ly = axis_code(src);
                    continue;
                }
                "rightx" => {
                    rx = axis_code(src);
                    continue;
                }
                "righty" => {
                    ry = axis_code(src);
                    continue;
                }
                _ => continue,
            };
            layout.parts.entry(part).or_default().push(src);
        }
        layout.left_stick = lx.zip(ly);
        layout.right_stick = rx.zip(ry);
        (!layout.parts.is_empty()).then_some(layout)
    }

    /// Pick the best known layout for a connected pad.
    pub fn resolve(pad: &Controller, paths: &Paths) -> Self {
        let guid = mapping::sdl_guid(pad.bus, pad.vendor, pad.product, pad.version, &pad.raw_name);
        if let Some(line) = mapping::saved_line(paths, pad.vendor, pad.product) {
            if let Some(l) = Self::from_sdl_mapping(&line, &pad.caps, Origin::Saved) {
                return l;
            }
        }
        if pad.kind != Kind::Generic {
            return Self::standard(pad.kind);
        }
        sdl::builtin_mapping(&guid)
            .and_then(|line| Self::from_sdl_mapping(&line, &pad.caps, Origin::Sdl))
            .unwrap_or_else(Self::unknown)
    }

    /// How "on" a part is right now, 0.0 (idle) to 1.0 (fully pressed).
    pub fn amount(&self, part: Part, st: &PadState) -> f32 {
        let Some(sources) = self.parts.get(&part) else { return 0.0 };
        sources
            .iter()
            .map(|s| match *s {
                Source::Button(c) => f32::from(st.pressed(c)),
                Source::Axis(c) => st.trigger(c),
                Source::Hat(c, dir) => f32::from(st.axes.get(&c).copied().unwrap_or(0).signum() == dir),
            })
            .fold(0.0, f32::max)
    }

    /// Stick position (-1..1 each way); (0,0) if this pad has no such stick.
    pub fn stick(&self, left: bool, kind: Kind, st: &PadState) -> (f32, f32) {
        match if left { self.left_stick } else { self.right_stick } {
            Some((x, y)) => st.stick(kind, x, y),
            None => (0.0, 0.0),
        }
    }
}

fn axis_code(s: Source) -> Option<u16> {
    match s {
        Source::Axis(c) => Some(c),
        _ => None,
    }
}

/// Parse an SDL binding (`b3`, `a2`, `a2~`, `+a2`, `h0.4`) into a raw input.
fn parse_binding(v: &str, caps: &Caps) -> Option<Source> {
    let v = v.trim_start_matches(['+', '-']).trim_end_matches('~');
    if let Some(n) = v.strip_prefix('b') {
        return caps.button_code(n.parse().ok()?).map(Source::Button);
    }
    if let Some(n) = v.strip_prefix('a') {
        return caps.axis_code(n.parse().ok()?).map(Source::Axis);
    }
    if let Some(rest) = v.strip_prefix('h') {
        let (hat, mask) = rest.split_once('.')?;
        let hat: u16 = hat.parse().ok()?;
        let (code, dir) = match mask.parse::<u8>().ok()? {
            1 => (ABS_HAT0Y + hat * 2, -1),
            4 => (ABS_HAT0Y + hat * 2, 1),
            2 => (ABS_HAT0X + hat * 2, 1),
            8 => (ABS_HAT0X + hat * 2, -1),
            _ => return None,
        };
        return Some(Source::Hat(code, dir));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::controllers::{EV_ABS, EV_KEY};

    /// SDL 2.30's own built-in mapping for the real "Twin USB Gamepad" adapter, read live.
    const TWIN: &str = "03004a9f100800000100000010010000,Twin USB PS2 Adapter,a:b2,b:b1,back:b8,dpdown:h0.4,dpleft:h0.8,dpright:h0.2,dpup:h0.1,leftshoulder:b6,leftstick:b10,lefttrigger:b4,leftx:a0,lefty:a1,rightshoulder:b7,rightstick:b11,righttrigger:b5,rightx:a3,righty:a2,start:b9,x:b3,y:b0,platform:Linux";
    const KEYS: &str = "fff00000000 0 0 0 0";
    const ABS: &str = "30027";

    #[test]
    fn generic_adapter_layout_from_sdl_mapping() {
        let caps = Caps::from_bitmaps(KEYS, ABS);
        let l = Layout::from_sdl_mapping(TWIN, &caps, Origin::Sdl).unwrap();
        // The button you pressed most (code 0x122 = SDL b2) is the bottom face button.
        assert_eq!(l.parts[&Part::A], vec![Source::Button(0x122)]);
        assert_eq!(l.parts[&Part::LeftTrigger], vec![Source::Button(0x124)]);
        assert_eq!(l.parts[&Part::DpadUp], vec![Source::Hat(ABS_HAT0Y, -1)]);
        assert_eq!(l.parts[&Part::DpadRight], vec![Source::Hat(ABS_HAT0X, 1)]);
        assert_eq!(l.left_stick, Some((0, 1)));
        assert_eq!(l.right_stick, Some((5, 2))); // ABS_RZ = a3 is rightx, ABS_Z = a2 is righty
    }

    #[test]
    fn presses_light_the_right_part() {
        let caps = Caps::from_bitmaps(KEYS, ABS);
        let l = Layout::from_sdl_mapping(TWIN, &caps, Origin::Sdl).unwrap();
        let mut st = PadState::default();
        assert_eq!(l.amount(Part::A, &st), 0.0);
        st.apply(EV_KEY, 0x122, 1);
        assert_eq!(l.amount(Part::A, &st), 1.0);
        assert_eq!(l.amount(Part::B, &st), 0.0);
        st.apply(EV_ABS, ABS_HAT0Y, -1);
        assert_eq!(l.amount(Part::DpadUp, &st), 1.0);
        assert_eq!(l.amount(Part::DpadDown, &st), 0.0);
    }

    #[test]
    fn standard_pads_differ_in_face_button_codes() {
        let xbox = Layout::standard(Kind::Xbox);
        let ps = Layout::standard(Kind::PlayStation);
        assert_eq!(xbox.parts[&Part::X], vec![Source::Button(0x133)]); // left
        assert_eq!(ps.parts[&Part::X], vec![Source::Button(0x134)]); // □ is left
        assert_eq!(ps.parts[&Part::Y], vec![Source::Button(0x133)]); // △ is top
        assert!(xbox.has(Part::Guide));
    }

    #[test]
    fn trigger_works_as_axis_or_button() {
        let l = Layout::standard(Kind::PlayStation);
        let mut st = PadState::default();
        st.apply(EV_ABS, ABS_Z, 255);
        assert!(l.amount(Part::LeftTrigger, &st) > 0.99);
        let mut st = PadState::default();
        st.apply(EV_KEY, 0x138, 1);
        assert_eq!(l.amount(Part::LeftTrigger, &st), 1.0);
    }

    #[test]
    fn bindings_parse() {
        let caps = Caps::from_bitmaps(KEYS, ABS);
        assert_eq!(parse_binding("b0", &caps), Some(Source::Button(0x120)));
        assert_eq!(parse_binding("a3~", &caps), Some(Source::Axis(5)));
        assert_eq!(parse_binding("h0.8", &caps), Some(Source::Hat(ABS_HAT0X, -1)));
        assert_eq!(parse_binding("b99", &caps), None);
        assert!(Layout::from_sdl_mapping("g,n,platform:Linux", &caps, Origin::Sdl).is_none());
    }
}
