//! UI themes. egui's `Visuals` cover the chrome (menus, buttons, windows,
//! panel background); the rack canvas is hand-painted, so a `Palette` of
//! semantic colors is threaded through the painting code. Four presets:
//! dark, light, and high-contrast variants of each.

use eframe::egui::{Color32, Visuals};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Theme {
    Dark,
    Light,
    DarkContrast,
    LightContrast,
}

impl Theme {
    pub const ALL: [Theme; 4] =
        [Theme::Dark, Theme::Light, Theme::DarkContrast, Theme::LightContrast];

    pub fn label(self) -> &'static str {
        match self {
            Theme::Dark => "Dark",
            Theme::Light => "Light",
            Theme::DarkContrast => "Dark (high contrast)",
            Theme::LightContrast => "Light (high contrast)",
        }
    }

    /// Stable key for localStorage persistence.
    pub fn key(self) -> &'static str {
        match self {
            Theme::Dark => "dark",
            Theme::Light => "light",
            Theme::DarkContrast => "dark-hc",
            Theme::LightContrast => "light-hc",
        }
    }

    pub fn from_key(s: &str) -> Theme {
        match s {
            "light" => Theme::Light,
            "dark-hc" => Theme::DarkContrast,
            "light-hc" => Theme::LightContrast,
            _ => Theme::Dark,
        }
    }

    pub fn is_dark(self) -> bool {
        matches!(self, Theme::Dark | Theme::DarkContrast)
    }

    pub fn palette(self) -> Palette {
        match self {
            Theme::Dark => DARK,
            Theme::Light => LIGHT,
            Theme::DarkContrast => DARK_HC,
            Theme::LightContrast => LIGHT_HC,
        }
    }

    /// egui chrome visuals. `panel_fill` doubles as the rack canvas background;
    /// `selection.bg_fill` is the accent the knob pointer reads.
    pub fn visuals(self) -> Visuals {
        let p = self.palette();
        let mut v = if self.is_dark() { Visuals::dark() } else { Visuals::light() };
        v.panel_fill = p.canvas;
        v.window_fill = p.title;
        v.extreme_bg_color = p.scope_bg;
        v.selection.bg_fill = p.accent;
        v.hyperlink_color = p.accent2;
        if matches!(self, Theme::DarkContrast) {
            v.override_text_color = Some(Color32::from_gray(245));
        } else if matches!(self, Theme::LightContrast) {
            v.override_text_color = Some(Color32::from_gray(10));
        }
        v
    }
}

/// Semantic colors for the hand-painted rack canvas.
#[derive(Clone, Copy)]
pub struct Palette {
    pub canvas: Color32,
    pub panel: Color32,
    pub border: Color32,
    pub accent: Color32,
    pub accent2: Color32,
    pub title: Color32,
    pub title_group: Color32,
    pub title_text: Color32,
    pub text_dim: Color32,
    pub outline: Color32,
    pub led_off: Color32,
    pub port_in: Color32,
    pub port_out: Color32,
    pub port_ring: Color32,
    pub port_dot: Color32,
    pub cell_off: Color32,
    pub cell_off4: Color32,
    pub cell_border: Color32,
    pub hover: Color32,
    pub scope_bg: Color32,
    pub scope_border: Color32,
    pub scope_grid: Color32,
    pub scope_mid: Color32,
    pub scope_line: Color32,
    pub bar: Color32,
    pub readout: Color32,
}

const fn g(v: u8) -> Color32 {
    Color32::from_gray(v)
}
const fn rgb(r: u8, gg: u8, b: u8) -> Color32 {
    Color32::from_rgb(r, gg, b)
}

const DARK: Palette = Palette {
    canvas: g(27),
    panel: g(38),
    border: g(90),
    accent: rgb(255, 200, 80),
    accent2: rgb(110, 200, 240),
    title: g(55),
    title_group: rgb(60, 70, 88),
    title_text: g(225),
    text_dim: g(170),
    outline: g(20),
    led_off: g(60),
    port_in: g(25),
    port_out: g(15),
    port_ring: g(130),
    port_dot: g(70),
    cell_off: g(40),
    cell_off4: g(64),
    cell_border: g(24),
    hover: g(200),
    scope_bg: g(14),
    scope_border: g(70),
    scope_grid: g(34),
    scope_mid: g(55),
    scope_line: rgb(120, 230, 150),
    bar: rgb(110, 200, 240),
    readout: rgb(120, 230, 150),
};

const LIGHT: Palette = Palette {
    canvas: g(214),
    panel: g(232),
    border: g(150),
    accent: rgb(205, 130, 0),
    accent2: rgb(40, 120, 200),
    title: g(206),
    title_group: rgb(186, 200, 222),
    title_text: g(30),
    text_dim: g(95),
    outline: g(170),
    led_off: g(175),
    port_in: g(198),
    port_out: g(180),
    port_ring: g(95),
    port_dot: g(150),
    cell_off: g(208),
    cell_off4: g(188),
    cell_border: g(160),
    hover: g(70),
    scope_bg: g(244),
    scope_border: g(150),
    scope_grid: g(210),
    scope_mid: g(175),
    scope_line: rgb(20, 140, 70),
    bar: rgb(40, 120, 200),
    readout: rgb(20, 140, 70),
};

const DARK_HC: Palette = Palette {
    canvas: g(8),
    panel: g(22),
    border: g(160),
    accent: rgb(255, 215, 40),
    accent2: rgb(120, 210, 255),
    title: g(40),
    title_group: rgb(50, 72, 112),
    title_text: g(255),
    text_dim: g(225),
    outline: g(0),
    led_off: g(85),
    port_in: g(10),
    port_out: g(0),
    port_ring: g(205),
    port_dot: g(130),
    cell_off: g(28),
    cell_off4: g(70),
    cell_border: g(0),
    hover: g(255),
    scope_bg: g(0),
    scope_border: g(170),
    scope_grid: g(60),
    scope_mid: g(105),
    scope_line: rgb(120, 255, 140),
    bar: rgb(120, 210, 255),
    readout: rgb(120, 255, 140),
};

const LIGHT_HC: Palette = Palette {
    canvas: g(250),
    panel: g(252),
    border: g(40),
    accent: rgb(180, 100, 0),
    accent2: rgb(0, 85, 175),
    title: g(224),
    title_group: rgb(168, 188, 226),
    title_text: g(0),
    text_dim: g(40),
    outline: g(55),
    led_off: g(150),
    port_in: g(244),
    port_out: g(255),
    port_ring: g(20),
    port_dot: g(110),
    cell_off: g(224),
    cell_off4: g(198),
    cell_border: g(55),
    hover: g(0),
    scope_bg: g(255),
    scope_border: g(40),
    scope_grid: g(200),
    scope_mid: g(150),
    scope_line: rgb(0, 110, 45),
    bar: rgb(0, 85, 175),
    readout: rgb(0, 110, 45),
};
