// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 Old Growth Crypto

use std::cell::RefCell;
use std::rc::Rc;

use gtk::prelude::*;

pub fn drawing_area(address: Rc<RefCell<String>>) -> gtk::DrawingArea {
    let area = gtk::DrawingArea::new();
    area.set_content_width(220);
    area.set_content_height(220);
    area.set_hexpand(false);
    area.set_halign(gtk::Align::Start);
    area.add_css_class("og-card");
    area.set_draw_func(move |_, cr, w, h| {
        let addr = address.borrow().clone();
        cr.set_source_rgb(0.067, 0.067, 0.075);
        cr.rectangle(0.0, 0.0, f64::from(w), f64::from(h));
        let _ = cr.fill();
        if addr.is_empty() {
            return;
        }
        let Ok(code) = qrcode::QrCode::new(addr.as_bytes()) else {
            return;
        };
        let n = code.width();
        let colors = code.to_colors();
        let pad = 12.0;
        let size = f64::from(w.min(h)) - pad * 2.0;
        let cell = size / n as f64;
        cr.set_source_rgb(0.976, 0.976, 0.961);
        cr.rectangle(pad, pad, size, size);
        let _ = cr.fill();
        cr.set_source_rgb(0.04, 0.04, 0.04);
        for y in 0..n {
            for x in 0..n {
                if colors[y * n + x] == qrcode::Color::Dark {
                    cr.rectangle(
                        pad + x as f64 * cell,
                        pad + y as f64 * cell,
                        cell,
                        cell,
                    );
                    let _ = cr.fill();
                }
            }
        }
    });
    area
}
