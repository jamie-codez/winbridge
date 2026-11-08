//! Draws a controller (Xbox or PlayStation style) and lights up whatever is pressed.

use eframe::egui::{self, vec2, Align2, Color32, FontId, Pos2, Rect, Sense, Stroke};
use winbridge_core::{
    controllers::{Kind, PadState},
    layout::{Layout, Part},
};

const BODY: Color32 = Color32::from_rgb(40, 44, 56);
const EDGE: Color32 = Color32::from_rgb(66, 72, 90);
const OFF: Color32 = Color32::from_rgb(62, 67, 82);
const ON: Color32 = Color32::from_rgb(52, 199, 140);
const DIM: Color32 = Color32::from_rgb(110, 116, 130);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Style {
    Xbox,
    PlayStation,
}

/// Artwork for this pad: by controller type, or by the name SDL gives a generic adapter.
pub fn style_for(kind: Kind, layout_name: &str) -> Style {
    let n = layout_name.to_ascii_lowercase();
    if kind == Kind::PlayStation
        || ["ps2", "ps3", "ps4", "ps5", "psx", "playstation", "dualshock", "dualsense"].iter().any(|k| n.contains(k))
    {
        Style::PlayStation
    } else {
        Style::Xbox
    }
}

fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()))
}

struct Ctx<'a> {
    painter: &'a egui::Painter,
    layout: &'a Layout,
    st: &'a PadState,
    origin: Pos2,
}

impl Ctx<'_> {
    fn p(&self, x: f32, y: f32) -> Pos2 {
        self.origin + vec2(x, y)
    }
    fn amount(&self, part: Part) -> f32 {
        self.layout.amount(part, self.st)
    }
    fn has(&self, part: Part) -> bool {
        self.layout.has(part)
    }

    fn round_button(&self, part: Part, c: Pos2, r: f32, label: &str, label_color: Color32, size: f32) {
        let (a, avail) = (self.amount(part), self.has(part));
        if a > 0.0 {
            self.painter.circle_filled(c, r + 6.0, ON.gamma_multiply(0.30));
        }
        let fill = if avail { mix(OFF, ON, a) } else { mix(OFF, BODY, 0.5) };
        self.painter.circle_filled(c, r, fill);
        self.painter.circle_stroke(c, r, Stroke::new(1.5, if a > 0.0 { Color32::WHITE } else { EDGE }));
        let tc = if a > 0.5 {
            Color32::BLACK
        } else if avail {
            label_color
        } else {
            DIM.gamma_multiply(0.5)
        };
        self.painter.text(c, Align2::CENTER_CENTER, label, FontId::proportional(size), tc);
    }

    fn pill(&self, part: Part, rect: Rect, label: &str) {
        let (a, avail) = (self.amount(part), self.has(part));
        if a > 0.0 {
            self.painter.rect_filled(rect.expand(4.0), rect.height() / 2.0 + 4.0, ON.gamma_multiply(0.30));
        }
        let fill = if avail { mix(OFF, ON, a) } else { mix(OFF, BODY, 0.5) };
        self.painter.rect_filled(rect, rect.height() / 2.0, fill);
        self.painter.rect_stroke(
            rect,
            rect.height() / 2.0,
            Stroke::new(1.2, if a > 0.0 { Color32::WHITE } else { EDGE }),
            egui::StrokeKind::Inside,
        );
        let tc = if a > 0.5 {
            Color32::BLACK
        } else if avail {
            DIM
        } else {
            DIM.gamma_multiply(0.5)
        };
        self.painter.text(rect.center(), Align2::CENTER_CENTER, label, FontId::proportional(10.0), tc);
    }

    /// Triggers fill from the bottom up as they're squeezed.
    fn trigger(&self, part: Part, rect: Rect, label: &str) {
        let (a, avail) = (self.amount(part), self.has(part));
        self.painter.rect_filled(rect, 8.0, if avail { OFF } else { mix(OFF, BODY, 0.5) });
        if a > 0.0 {
            let h = rect.height() * a;
            let fill = Rect::from_min_max(Pos2::new(rect.min.x, rect.max.y - h), rect.max);
            self.painter.rect_filled(fill, 8.0, ON);
        }
        self.painter.rect_stroke(
            rect,
            8.0,
            Stroke::new(1.2, if a > 0.0 { Color32::WHITE } else { EDGE }),
            egui::StrokeKind::Inside,
        );
        let tc = if a > 0.5 { Color32::BLACK } else { DIM };
        self.painter.text(rect.center(), Align2::CENTER_CENTER, label, FontId::proportional(11.0), tc);
    }

    fn shoulder(&self, part: Part, rect: Rect, label: &str) {
        let (a, avail) = (self.amount(part), self.has(part));
        self.painter.rect_filled(rect, 6.0, if avail { mix(OFF, ON, a) } else { mix(OFF, BODY, 0.5) });
        self.painter.rect_stroke(
            rect,
            6.0,
            Stroke::new(1.2, if a > 0.0 { Color32::WHITE } else { EDGE }),
            egui::StrokeKind::Inside,
        );
        let tc = if a > 0.5 { Color32::BLACK } else { DIM };
        self.painter.text(rect.center(), Align2::CENTER_CENTER, label, FontId::proportional(10.0), tc);
    }

    fn stick(&self, left: bool, c: Pos2, r: f32, kind: Kind) {
        let part = if left { Part::LeftStick } else { Part::RightStick };
        let (x, y) = self.layout.stick(left, kind, self.st);
        let clicked = self.amount(part);
        self.painter.circle_filled(c, r, Color32::from_rgb(28, 31, 40));
        self.painter.circle_stroke(c, r, Stroke::new(1.5, EDGE));
        let thumb = c + vec2(x, y) * (r * 0.38);
        let moved = (x * x + y * y).sqrt().min(1.0);
        let fill = mix(mix(OFF, ON, moved * 0.8), ON, clicked);
        self.painter.circle_filled(thumb, r * 0.62, fill);
        self.painter.circle_stroke(
            thumb,
            r * 0.62,
            Stroke::new(1.5, if moved > 0.1 || clicked > 0.0 { Color32::WHITE } else { EDGE }),
        );
    }

    fn dpad(&self, c: Pos2) {
        let (w, l) = (22.0, 27.0);
        let off = l / 2.0 + w / 2.0 - 3.0;
        let arms = [
            (Part::DpadUp, vec2(0.0, -off), vec2(w, l)),
            (Part::DpadDown, vec2(0.0, off), vec2(w, l)),
            (Part::DpadLeft, vec2(-off, 0.0), vec2(l, w)),
            (Part::DpadRight, vec2(off, 0.0), vec2(l, w)),
        ];
        self.painter.rect_filled(Rect::from_center_size(c, vec2(w, w)), 3.0, OFF);
        for (part, d, size) in arms {
            let (a, avail) = (self.amount(part), self.has(part));
            let rect = Rect::from_center_size(c + d, size);
            self.painter.rect_filled(rect, 4.0, if avail { mix(OFF, ON, a) } else { mix(OFF, BODY, 0.5) });
            self.painter.rect_stroke(
                rect,
                4.0,
                Stroke::new(1.2, if a > 0.0 { Color32::WHITE } else { EDGE }),
                egui::StrokeKind::Inside,
            );
        }
    }
}

pub const SIZE: egui::Vec2 = egui::Vec2::new(480.0, 290.0);

pub fn draw(ui: &mut egui::Ui, style: Style, kind: Kind, layout: &Layout, st: &PadState) {
    let (resp, painter) = ui.allocate_painter(SIZE, Sense::hover());
    let g = Ctx { painter: &painter, layout, st, origin: resp.rect.min };

    // Body: a slightly larger lighter copy underneath gives the outline.
    for (grow, col) in [(3.0, EDGE), (0.0, BODY)] {
        painter.rect_filled(
            Rect::from_min_max(g.p(78.0 - grow, 62.0 - grow), g.p(402.0 + grow, 196.0 + grow)),
            56.0 + grow,
            col,
        );
        painter.circle_filled(g.p(122.0, 212.0), 58.0 + grow, col);
        painter.circle_filled(g.p(358.0, 212.0), 58.0 + grow, col);
    }

    let ps = style == Style::PlayStation;
    g.trigger(Part::LeftTrigger, Rect::from_min_max(g.p(96.0, 14.0), g.p(178.0, 42.0)), if ps { "L2" } else { "LT" });
    g.trigger(Part::RightTrigger, Rect::from_min_max(g.p(302.0, 14.0), g.p(384.0, 42.0)), if ps { "R2" } else { "RT" });
    g.shoulder(Part::LeftShoulder, Rect::from_min_max(g.p(90.0, 46.0), g.p(188.0, 62.0)), if ps { "L1" } else { "LB" });
    g.shoulder(
        Part::RightShoulder,
        Rect::from_min_max(g.p(292.0, 46.0), g.p(390.0, 62.0)),
        if ps { "R1" } else { "RB" },
    );

    // Face buttons (same positions on both; only the symbols differ).
    let fc = g.p(352.0, 112.0);
    let (a, b, x, y) = if ps {
        (
            ("✕", Color32::from_rgb(120, 160, 240)),
            ("○", Color32::from_rgb(240, 90, 100)),
            ("□", Color32::from_rgb(230, 140, 210)),
            ("△", Color32::from_rgb(80, 210, 160)),
        )
    } else {
        (
            ("A", Color32::from_rgb(70, 200, 100)),
            ("B", Color32::from_rgb(240, 80, 80)),
            ("X", Color32::from_rgb(80, 140, 240)),
            ("Y", Color32::from_rgb(245, 195, 60)),
        )
    };
    g.round_button(Part::A, fc + vec2(0.0, 27.0), 14.0, a.0, a.1, 15.0);
    g.round_button(Part::B, fc + vec2(27.0, 0.0), 14.0, b.0, b.1, 15.0);
    g.round_button(Part::X, fc + vec2(-27.0, 0.0), 14.0, x.0, x.1, 15.0);
    g.round_button(Part::Y, fc + vec2(0.0, -27.0), 14.0, y.0, y.1, 15.0);

    if ps {
        // DualShock / DualSense: d-pad top-left, symmetric sticks, touchpad, Share / Options.
        g.dpad(g.p(128.0, 112.0));
        g.stick(true, g.p(196.0, 180.0), 27.0, kind);
        g.stick(false, g.p(284.0, 180.0), 27.0, kind);
        let pad = Rect::from_min_max(g.p(192.0, 68.0), g.p(288.0, 118.0));
        painter.rect_filled(pad, 10.0, Color32::from_rgb(30, 34, 44));
        painter.rect_stroke(pad, 10.0, Stroke::new(1.2, EDGE), egui::StrokeKind::Inside);
        g.pill(Part::Back, Rect::from_center_size(g.p(168.0, 80.0), vec2(34.0, 16.0)), "Share");
        g.pill(Part::Start, Rect::from_center_size(g.p(312.0, 80.0), vec2(46.0, 16.0)), "Options");
        g.round_button(Part::Guide, g.p(240.0, 140.0), 11.0, "PS", DIM, 10.0);
    } else {
        // Xbox: left stick top-left, d-pad lower-left, right stick lower-right.
        g.stick(true, g.p(128.0, 112.0), 28.0, kind);
        g.dpad(g.p(196.0, 176.0));
        g.stick(false, g.p(284.0, 176.0), 28.0, kind);
        g.pill(Part::Back, Rect::from_center_size(g.p(214.0, 112.0), vec2(34.0, 16.0)), "Back");
        g.pill(Part::Start, Rect::from_center_size(g.p(266.0, 112.0), vec2(34.0, 16.0)), "Start");
        g.round_button(Part::Guide, g.p(240.0, 84.0), 13.0, "", DIM, 11.0);
        painter.circle_stroke(
            g.p(240.0, 84.0),
            6.0,
            Stroke::new(1.5, if g.amount(Part::Guide) > 0.5 { Color32::BLACK } else { DIM }),
        );
    }
}
