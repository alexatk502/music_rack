//! Hand-rolled rotary knob widget: vertical drag to change, Shift for fine
//! control, double-click to reset. Returns the inner Response; `changed()`
//! reports value changes so the caller can emit a SetParam message.

use eframe::egui::{self, Color32, Response, Sense, Stroke, Ui, Vec2};
use std::ops::RangeInclusive;

const KNOB_SIZE: f32 = 36.0;
/// Sweep from 7 o'clock to 5 o'clock, like hardware.
const ANGLE_MIN: f32 = -0.75 * std::f32::consts::PI;
const ANGLE_MAX: f32 = 0.75 * std::f32::consts::PI;
/// Full-range travel in drag pixels.
const DRAG_PIXELS: f32 = 200.0;

pub fn knob(
    ui: &mut Ui,
    label: &str,
    value: &mut f32,
    range: RangeInclusive<f32>,
    default: f32,
) -> Response {
    let (min, max) = (*range.start(), *range.end());

    ui.vertical(|ui| {
        ui.set_width(KNOB_SIZE + 12.0);
        ui.label(egui::RichText::new(label).size(10.0));

        let (rect, mut response) =
            ui.allocate_exact_size(Vec2::splat(KNOB_SIZE), Sense::click_and_drag());

        if response.double_clicked() {
            *value = default;
            response.mark_changed();
        } else if response.dragged() {
            let fine = if ui.input(|i| i.modifiers.shift) { 0.1 } else { 1.0 };
            let delta = -response.drag_delta().y * fine * (max - min) / DRAG_PIXELS;
            let new = (*value + delta).clamp(min, max);
            if new != *value {
                *value = new;
                response.mark_changed();
            }
        }

        let center = rect.center();
        let radius = KNOB_SIZE * 0.5 - 2.0;
        let t = ((*value - min) / (max - min)).clamp(0.0, 1.0);
        let angle = ANGLE_MIN + t * (ANGLE_MAX - ANGLE_MIN);

        let painter = ui.painter();
        let body = if response.hovered() {
            Color32::from_gray(70)
        } else {
            Color32::from_gray(55)
        };
        painter.circle(center, radius, body, Stroke::new(1.5, Color32::from_gray(140)));
        // Pointer line: angle 0 = straight up.
        let dir = Vec2::new(angle.sin(), -angle.cos());
        painter.line_segment(
            [center + dir * (radius * 0.35), center + dir * (radius * 0.95)],
            Stroke::new(2.0, Color32::from_rgb(255, 200, 80)),
        );

        ui.label(egui::RichText::new(format!("{value:.2}")).size(9.0).weak());

        response
    })
    .inner
}
