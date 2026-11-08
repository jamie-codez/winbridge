//! `winbridge-gui --preview xbox|ps|generic [--step N]`: shows the controller artwork with
//! parts lighting up one after another (or frozen on step N). Handy for checking the drawing
//! without a controller in hand.

use std::time::Instant;

use eframe::egui::{self, Color32};
use winbridge_core::{
    controllers::{Kind, PadState, EV_ABS, EV_KEY},
    layout::{Layout, Origin, Part, Source},
    mapping::Caps,
};

use crate::pad_view;

const ORDER: [Part; 17] = [
    Part::A,
    Part::B,
    Part::X,
    Part::Y,
    Part::LeftShoulder,
    Part::RightShoulder,
    Part::LeftTrigger,
    Part::RightTrigger,
    Part::Back,
    Part::Start,
    Part::Guide,
    Part::LeftStick,
    Part::RightStick,
    Part::DpadUp,
    Part::DpadDown,
    Part::DpadLeft,
    Part::DpadRight,
];
const STEPS: usize = ORDER.len() + 2; // + two stick pushes

/// The real SDL mapping of the "Twin USB Gamepad" PlayStation-style adapter, for the generic preview.
const TWIN: &str = "03004a9f100800000100000010010000,Twin USB PS2 Adapter,a:b2,b:b1,back:b8,dpdown:h0.4,dpleft:h0.8,dpright:h0.2,dpup:h0.1,leftshoulder:b6,leftstick:b10,lefttrigger:b4,leftx:a0,lefty:a1,rightshoulder:b7,rightstick:b11,righttrigger:b5,rightx:a3,righty:a2,start:b9,x:b3,y:b0,platform:Linux";

struct Preview {
    kind: Kind,
    layout: Layout,
    step: Option<usize>,
    t0: Instant,
}

pub fn maybe_run() -> Option<eframe::Result> {
    let args: Vec<String> = std::env::args().collect();
    let i = args.iter().position(|a| a == "--preview")?;
    let which = args.get(i + 1).map(String::as_str).unwrap_or("xbox");
    let step = args.iter().position(|a| a == "--step").and_then(|j| args.get(j + 1)).and_then(|v| v.parse().ok());
    let (kind, layout) = match which {
        "ps" => (Kind::PlayStation, Layout::standard(Kind::PlayStation)),
        "generic" => {
            let caps = Caps::from_bitmaps("fff00000000 0 0 0 0", "30027");
            (Kind::Generic, Layout::from_sdl_mapping(TWIN, &caps, Origin::Sdl).expect("built-in sample mapping"))
        }
        _ => (Kind::Xbox, Layout::standard(Kind::Xbox)),
    };
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([540.0, 380.0]).with_title("WinBridge preview"),
        ..Default::default()
    };
    Some(eframe::run_native(
        "WinBridge preview",
        options,
        Box::new(move |cc| {
            let mut v = egui::Visuals::dark();
            v.panel_fill = Color32::from_rgb(22, 24, 30);
            cc.egui_ctx.set_visuals(v);
            Ok(Box::new(Preview { kind, layout, step, t0: Instant::now() }))
        }),
    ))
}

impl Preview {
    fn state(&self, step: usize) -> PadState {
        // Seed every axis like a real device would report at rest.
        let byte_range = self.kind != Kind::Xbox;
        let mut ranges = std::collections::HashMap::new();
        for code in [0u16, 1, 2, 3, 4, 5] {
            let trigger_axis = matches!(code, 2 | 5) && self.kind != Kind::Generic;
            ranges.insert(
                code,
                match (byte_range, trigger_axis) {
                    (_, true) => (0, 255, 0),
                    (true, false) => (0, 255, 128),
                    (false, false) => (-32768, 32767, 0),
                },
            );
        }
        let mut st = PadState::with_axes(&ranges);
        let (full, rest) = if byte_range { (255, 128) } else { (32767, 0) };
        let axis = |st: &mut PadState, pair: Option<(u16, u16)>, fx: f32, fy: f32| {
            if let Some((x, y)) = pair {
                let span = (full - rest) as f32;
                st.apply(EV_ABS, x, rest + (fx * span) as i32);
                st.apply(EV_ABS, y, rest + (fy * span) as i32);
            }
        };
        if step < ORDER.len() {
            for src in self.layout.parts.get(&ORDER[step]).into_iter().flatten() {
                match *src {
                    Source::Button(c) => st.apply(EV_KEY, c, 1),
                    Source::Axis(c) => st.apply(EV_ABS, c, full),
                    Source::Hat(c, d) => st.apply(EV_ABS, c, d),
                }
            }
        } else if step == ORDER.len() {
            axis(&mut st, self.layout.left_stick, 1.0, 0.45);
        } else {
            axis(&mut st, self.layout.right_stick, -0.8, 0.9);
        }
        st
    }
}

impl eframe::App for Preview {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let step = self.step.unwrap_or(((self.t0.elapsed().as_secs_f32() / 0.8) as usize) % STEPS);
        let st = self.state(step);
        let style = pad_view::style_for(self.kind, &self.layout.name);
        egui::CentralPanel::default().show(ui, |ui| {
            ui.vertical_centered(|ui| {
                let what = ORDER.get(step).map(|p| format!("{p:?}")).unwrap_or_else(|| {
                    if step == ORDER.len() {
                        "Left stick pushed".into()
                    } else {
                        "Right stick pushed".into()
                    }
                });
                ui.label(format!("{style:?} artwork — lighting: {what}"));
                ui.add_space(10.0);
                pad_view::draw(ui, style, self.kind, &self.layout, &st);
            });
        });
        if self.step.is_none() {
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
        }
    }
}
