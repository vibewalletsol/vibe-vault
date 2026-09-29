// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 Old Growth Crypto

use gtk::prelude::*;

const PALETTE: [(f64, f64, f64); 8] = [
    (0.545, 0.486, 0.969),
    (0.243, 0.910, 0.659),
    (0.961, 0.757, 0.424),
    (0.357, 0.753, 0.922),
    (0.941, 0.443, 0.647),
    (0.655, 0.545, 0.980),
    (0.204, 0.827, 0.600),
    (0.976, 0.451, 0.086),
];

#[derive(Clone, Debug)]
pub struct Slice {
    pub symbol: String,
    pub value: f64,
    pub pct: f64,
}

pub fn drawing_area(slices: std::rc::Rc<std::cell::RefCell<Vec<Slice>>>) -> gtk::DrawingArea {
    let area = gtk::DrawingArea::new();
    area.set_content_width(245);
    area.set_content_height(245);
    area.set_hexpand(false);
    area.add_css_class("og-donut");
    area.set_draw_func(move |_, cr, w, h| {
        let data = slices.borrow().clone();
        let cx = f64::from(w) / 2.0;
        let cy = f64::from(h) / 2.0;
        let radius = cx.min(cy) - 8.0;
        let inner = radius * 0.62;
        cr.set_source_rgb(0.078, 0.078, 0.110);
        cr.arc(cx, cy, radius, 0.0, std::f64::consts::TAU);
        let _ = cr.fill();
        if data.is_empty() || data.iter().all(|s| s.pct <= 0.0) {
            cr.set_source_rgba(0.545, 0.486, 0.969, 0.18);
            cr.arc(cx, cy, radius, 0.0, std::f64::consts::TAU);
            cr.arc_negative(cx, cy, inner, std::f64::consts::TAU, 0.0);
            cr.close_path();
            let _ = cr.fill();
            return;
        }
        let mut angle = -std::f64::consts::FRAC_PI_2;
        let gap = 0.045;
        for (i, slice) in data.iter().enumerate() {
            let span = std::f64::consts::TAU * (slice.pct.max(0.0) / 100.0) - gap;
            if span <= 0.0 {
                continue;
            }
            let (r, g, b) = PALETTE[i % PALETTE.len()];
            cr.set_source_rgb(r, g, b);
            cr.arc(cx, cy, radius, angle, angle + span);
            cr.arc_negative(cx, cy, inner, angle + span, angle);
            cr.close_path();
            let _ = cr.fill();
            angle += span + gap;
        }
        cr.set_source_rgb(0.067, 0.067, 0.075);
        cr.arc(cx, cy, inner - 1.0, 0.0, std::f64::consts::TAU);
        let _ = cr.fill();
    });
    area
}

pub fn color_hex(i: usize) -> String {
    let (r, g, b) = PALETTE[i % PALETTE.len()];
    format!(
        "#{:02x}{:02x}{:02x}",
        (r * 255.0) as u8,
        (g * 255.0) as u8,
        (b * 255.0) as u8
    )
}
