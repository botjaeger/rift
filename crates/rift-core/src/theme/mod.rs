use serde::{Deserialize, Serialize};

pub mod builtin;
pub mod warp;

pub use warp::parse_warp_yaml;

/// sRGB color with straight alpha, 0..=1.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Color {
    pub const fn rgba(r: f32, g: f32, b: f32, a: f32) -> Self {
        Self { r, g, b, a }
    }

    pub const TRANSPARENT: Self = Self::rgba(0.0, 0.0, 0.0, 0.0);
    pub const WHITE: Self = Self::rgba(1.0, 1.0, 1.0, 1.0);
    pub const BLACK: Self = Self::rgba(0.0, 0.0, 0.0, 1.0);

    /// Parse `#rgb`, `#rrggbb` or `#rrggbbaa`.
    pub fn hex(s: &str) -> Result<Self, ColorParseError> {
        let h = s.trim().trim_start_matches('#');
        // from_str_radix would accept a leading `+` (`#+12345`)
        if !h.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(ColorParseError(s.into()));
        }
        let (r, g, b, a) = match h.len() {
            3 => {
                let v = u16::from_str_radix(h, 16).map_err(|_| ColorParseError(s.into()))?;
                let e = |x: u16| (x | (x << 4)) as f32 / 255.0;
                (e((v >> 8) & 0xf), e((v >> 4) & 0xf), e(v & 0xf), 1.0)
            }
            6 | 8 => {
                let v = u64::from_str_radix(h, 16).map_err(|_| ColorParseError(s.into()))?;
                let v = if h.len() == 6 { (v << 8) | 0xff } else { v };
                (
                    ((v >> 24) & 0xff) as f32 / 255.0,
                    ((v >> 16) & 0xff) as f32 / 255.0,
                    ((v >> 8) & 0xff) as f32 / 255.0,
                    (v & 0xff) as f32 / 255.0,
                )
            }
            _ => return Err(ColorParseError(s.into())),
        };
        Ok(Self { r, g, b, a })
    }

    pub fn with_alpha(self, a: f32) -> Self {
        Self { a, ..self }
    }

    /// Linear interpolation toward `other` in sRGB space, `t` clamped to 0..=1.
    /// `t == 0` is `self`, `t == 1` is `other`. Alpha is mixed too.
    pub fn mix(self, other: Color, t: f32) -> Self {
        let t = t.clamp(0.0, 1.0);
        let l = |a: f32, b: f32| a + (b - a) * t;
        Self {
            r: l(self.r, other.r),
            g: l(self.g, other.g),
            b: l(self.b, other.b),
            a: l(self.a, other.a),
        }
    }

    /// WCAG 2.x relative luminance (0 = black, 1 = white). Alpha is ignored.
    pub fn luminance(self) -> f32 {
        let c = |v: f32| {
            let v = v.clamp(0.0, 1.0);
            if v <= 0.04045 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * c(self.r) + 0.7152 * c(self.g) + 0.0722 * c(self.b)
    }

    /// WCAG 2.x contrast ratio against `other`, 1.0..=21.0. Alpha is ignored.
    pub fn contrast(self, other: Color) -> f32 {
        let (x, y) = (self.luminance(), other.luminance());
        (x.max(y) + 0.05) / (x.min(y) + 0.05)
    }

    /// This colour as text on `bg` with at least `ratio` contrast
    /// (`minimum_contrast`): unchanged when it already has it, else mixed
    /// toward white or black, whichever reaches the ratio with the smaller
    /// change (the one with more contrast when neither does). Alpha is kept.
    pub fn with_min_contrast(self, bg: Color, ratio: f32) -> Color {
        if ratio <= 1.0 || self.contrast(bg) >= ratio {
            return self;
        }
        // the least mix toward `end` that reaches the ratio: (mix, colour),
        // or None when even `end` itself falls short
        let toward = |end: Color| {
            if end.contrast(bg) < ratio {
                return None;
            }
            let (mut lo, mut hi) = (0.0f32, 1.0f32);
            for _ in 0..16 {
                let mid = (lo + hi) / 2.0;
                if self.mix(end, mid).contrast(bg) >= ratio {
                    hi = mid;
                } else {
                    lo = mid;
                }
            }
            Some((hi, self.mix(end, hi).with_alpha(self.a)))
        };
        match (toward(Color::WHITE), toward(Color::BLACK)) {
            (Some((tw, white)), Some((tb, black))) => {
                if tb < tw {
                    black
                } else {
                    white
                }
            }
            (Some((_, c)), None) | (None, Some((_, c))) => c,
            // no colour reaches it: the extreme with more contrast
            (None, None) if Color::BLACK.contrast(bg) > Color::WHITE.contrast(bg) => {
                Color::BLACK.with_alpha(self.a)
            }
            (None, None) => Color::WHITE.with_alpha(self.a),
        }
    }

    pub fn to_hex(self) -> String {
        let b = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        if self.a >= 1.0 {
            format!("#{:02x}{:02x}{:02x}", b(self.r), b(self.g), b(self.b))
        } else {
            format!(
                "#{:02x}{:02x}{:02x}{:02x}",
                b(self.r),
                b(self.g),
                b(self.b),
                b(self.a)
            )
        }
    }
}

#[derive(Debug)]
pub struct ColorParseError(pub String);

impl std::fmt::Display for ColorParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "invalid color: {:?}", self.0)
    }
}
impl std::error::Error for ColorParseError {}

impl TryFrom<String> for Color {
    type Error = ColorParseError;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        Color::hex(&s)
    }
}
impl From<Color> for String {
    fn from(c: Color) -> String {
        c.to_hex()
    }
}

/// Syntax colors for the input editor and recognized output.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyntaxColors {
    pub command: Color, // accent by default
    pub flag: Color,    // #9cdcfe
    pub string: Color,  // #ce9178
    pub ghost: Color,   // #5a5c63
}

/// Optional wallpaper drawn behind the terminal body.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackgroundImage {
    /// Absolute path, or relative to the themes directory when it came from a
    /// user theme file (resolved by [`load_by_name`]).
    pub path: String,
    /// 0..=1. Warp's YAML gives 0..=100; [`parse_warp_yaml`] normalizes it.
    #[serde(default = "one")]
    pub opacity: f32,
}

fn one() -> f32 {
    1.0
}

/// A two-stop linear gradient (Warp's gradient `background` / `accent`).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Gradient {
    /// Top (vertical) or left (horizontal) stop.
    pub from: Color,
    /// Bottom or right stop.
    pub to: Color,
    /// Top to bottom; else left to right.
    #[serde(default)]
    pub vertical: bool,
}

impl Gradient {
    /// The colour halfway: what a surface that cannot show the gradient
    /// (and every token derived from it) uses.
    pub fn mid(&self) -> Color {
        self.from.mix(self.to, 0.5)
    }
}

/// Full theme token set — every rendered color resolves from here.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Theme {
    pub name: String,

    // surfaces
    pub bg: Color,        // terminal body
    pub bg_chrome: Color, // tab bar / sidebar
    pub bg_gutter: Color, // split-pane gutter
    pub surface: Color,   // modal / dropdown
    pub border: Color,    // hairline
    pub pane_border_inactive: Color,

    // text
    pub fg: Color,          // default terminal text
    pub text_bright: Color, // command / emphasized
    pub text_ui: Color,     // chrome labels
    pub text_dim: Color,    // secondary
    pub text_muted: Color,  // tertiary / meta

    // semantics
    pub accent: Color,
    pub success: Color,
    pub error: Color,
    pub warning: Color,
    pub cursor: Color,
    pub selection: Color,
    pub search_match: Color,
    pub search_active: Color,

    pub syntax: SyntaxColors,

    /// ANSI base palette, indices 0..=15.
    pub ansi: [Color; 16],

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background_image: Option<BackgroundImage>,

    /// A gradient terminal ground: drawn behind the panes of an opaque
    /// window without a background image; `bg` is its midpoint.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bg_gradient: Option<Gradient>,
    /// A gradient accent: the active tab's top edge; `accent` is its
    /// midpoint everywhere else.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accent_gradient: Option<Gradient>,

    /// Window opacity (`window_opacity`), set by the app, never by a theme
    /// file. Below 1 the window ground goes translucent the way Warp's does
    /// (see [`Theme::window_ground`] and [`Theme::ground`]).
    #[serde(skip, default = "one")]
    pub ground_alpha: f32,
}

impl Theme {
    /// Fill for a surface that is part of the window ground (tab bar, side
    /// panels, split gutter, pane body): its own colour while the window is
    /// opaque; none while translucent, so the uniform ground shows through
    /// — Warp's tab bar, panels and panes have no fill of their own.
    pub fn ground(&self, c: Color) -> Color {
        if self.ground_alpha < 1.0 {
            c.with_alpha(0.0)
        } else {
            c
        }
    }

    /// A chip's fill (prompt chips): a 4 % lift of `bg`, opaque, so a
    /// translucent window keeps the chips dark and their text readable over
    /// whatever shows through (Warp's chips are solid too).
    pub fn chip_fill(&self) -> Color {
        self.bg.mix(self.fg, 0.04).with_alpha(1.0)
    }

    /// The autosuggestion's ghost text. On a translucent window the ground is
    /// lighter than `bg` (a white desktop under 50 % shows mid grey), where
    /// the ghost colour vanished: it moves toward `fg` there, as Warp's does.
    pub fn ghost_text(&self) -> Color {
        if self.ground_alpha < 1.0 {
            self.syntax.ghost.mix(self.fg, 0.45)
        } else {
            self.syntax.ghost
        }
    }

    /// The window ground (the frame's clear colour, straight alpha). Opaque:
    /// `bg`. Translucent, as Warp draws it: `surface_2` (bg + 10 % fg) at the
    /// window opacity, then `bg` at the window opacity over it — two layers,
    /// so the covered fraction is `1 − (1 − a)²`.
    pub fn window_ground(&self) -> Color {
        let a = self.ground_alpha;
        if a >= 1.0 {
            return self.bg;
        }
        // Warp's neutral_2: fg at alpha byte 25 (10 %) blended onto bg
        let surface_2 = self.bg.mix(self.fg.with_alpha(self.bg.a), 25.0 / 255.0);
        let out_a = a + a * (1.0 - a);
        let ch = |top: f32, bottom: f32| (a * top + (1.0 - a) * a * bottom) / out_a;
        Color {
            r: ch(self.bg.r, surface_2.r),
            g: ch(self.bg.g, surface_2.g),
            b: ch(self.bg.b, surface_2.b),
            a: out_a,
        }
    }

    /// The fill a pane in this theme (a pane's own colour scheme) draws
    /// over the window ground of `window`. Opaque: its `bg`. Translucent:
    /// the colour that turns `window`'s ground into this theme's ground with
    /// the least cover, so the pane stays as see-through as the window
    /// where the two grounds agree and only gains what its colour needs.
    pub fn pane_ground(&self, window: &Theme) -> Color {
        if self.ground_alpha >= 1.0 {
            return self.bg;
        }
        let (t, w) = (self.window_ground(), window.window_ground());
        // per channel, the least cover x with x·c + (1 − x)·w = t, c in 0..=1
        let need = |t: f32, w: f32| {
            if t > w {
                (t - w) / (1.0 - w)
            } else if t < w {
                (w - t) / w
            } else {
                0.0
            }
        };
        let x = need(t.r, w.r)
            .max(need(t.g, w.g))
            .max(need(t.b, w.b))
            .clamp(0.0, 1.0);
        if x <= 0.0 {
            return t.with_alpha(0.0);
        }
        let c = |t: f32, w: f32| (w + (t - w) / x).clamp(0.0, 1.0);
        Color {
            r: c(t.r, w.r),
            g: c(t.g, w.g),
            b: c(t.b, w.b),
            a: x,
        }
    }

    pub fn parse_toml(s: &str) -> Result<Self, toml::de::Error> {
        toml::from_str(s)
    }

    /// True when the terminal ground is bright enough to want dark text.
    pub fn is_light(&self) -> bool {
        self.bg.luminance() > 0.5
    }
}

impl Default for Theme {
    fn default() -> Self {
        builtin::dark_default()
    }
}

/// Builtin theme names, in picker display order.
pub const BUILTIN_NAMES: [&str; builtin::SOURCES.len()] = {
    let mut out = [""; builtin::SOURCES.len()];
    let mut i = 0;
    while i < out.len() {
        out[i] = builtin::SOURCES[i].0;
        i += 1;
    }
    out
};

/// Extensions probed in `~/.config/rift/themes/`, in precedence order.
const USER_THEME_EXTS: [&str; 3] = ["toml", "yaml", "yml"];

/// Resolve a theme by config name: `~/.config/rift/themes/<name>.{toml,yaml,yml}`
/// first (a file named like a builtin replaces it, so a Warp `dracula.yaml`
/// imported before Dracula became a builtin keeps showing), then the
/// builtin, else dark-default.
///
/// `.yaml`/`.yml` files are read as Warp themes (see [`parse_warp_yaml`]).
pub fn load_by_name(name: &str) -> Theme {
    try_load_by_name(name).unwrap_or_else(|e| {
        log::warn!("theme {name:?}: {e}; using dark-default");
        builtin::dark_default()
    })
}

/// `load_by_name` without the fallback: why the theme could not be loaded
/// (no such theme, or the file's parse error).
pub fn try_load_by_name(name: &str) -> Result<Theme, String> {
    try_load_in(crate::config::themes_dir().as_deref(), name)
}

/// [`try_load_by_name`] over the themes directory `dir`.
fn try_load_in(dir: Option<&std::path::Path>, name: &str) -> Result<Theme, String> {
    let mut error = None;
    if let Some(dir) = dir {
        for ext in USER_THEME_EXTS {
            let path = dir.join(format!("{name}.{ext}"));
            let Ok(src) = std::fs::read_to_string(&path) else {
                continue;
            };
            let parsed = if ext == "toml" {
                Theme::parse_toml(&src).map_err(|e| e.to_string())
            } else {
                parse_warp_yaml(name, &src)
            };
            match parsed {
                Ok(mut t) => {
                    // A relative wallpaper path is relative to the themes dir.
                    if let Some(img) = t.background_image.as_mut() {
                        let p = std::path::Path::new(&img.path);
                        if p.is_relative() {
                            img.path = dir.join(p).to_string_lossy().into_owned();
                        }
                    }
                    return Ok(t);
                }
                Err(e) => {
                    // one line (toasts): the TOML error's location line and
                    // its message, without the source excerpt between
                    let lines: Vec<&str> = e.lines().filter(|l| !l.trim().is_empty()).collect();
                    let short = match (lines.first(), lines.last()) {
                        (Some(first), Some(last)) if lines.len() > 1 => {
                            format!("{} ({})", last.trim(), first.trim())
                        }
                        _ => e.trim().to_string(),
                    };
                    error = Some(format!("{name}.{ext}: {short}"));
                }
            }
        }
    }
    match (error, builtin::by_name(name)) {
        (Some(e), _) => Err(e),
        (None, Some(t)) => Ok(t),
        (None, None) => Err("not found".into()),
    }
}

/// The picker's label for each of `names` (`available_themes`): the theme's
/// own name; a file that does not load says so instead of showing the
/// fallback's name, and a name several files share carries the file stem.
pub fn picker_labels(names: &[String]) -> Vec<String> {
    let base: Vec<String> = names
        .iter()
        .map(|n| match try_load_by_name(n) {
            Ok(t) => t.name,
            Err(_) => format!("{n} (invalid theme file)"),
        })
        .collect();
    base.iter()
        .zip(names)
        .map(|(label, stem)| {
            if base.iter().filter(|l| *l == label).count() > 1 {
                format!("{label} ({stem})")
            } else {
                label.clone()
            }
        })
        .collect()
}

/// Names available in the theme picker: builtins (in order) then user theme
/// file stems, deduped and sorted.
pub fn available_themes() -> Vec<String> {
    let mut names: Vec<String> = BUILTIN_NAMES.iter().map(|s| s.to_string()).collect();
    let mut user: Vec<String> = Vec::new();
    if let Some(dir) = crate::config::themes_dir()
        && let Ok(entries) = std::fs::read_dir(dir)
    {
        for e in entries.flatten() {
            let p = e.path();
            let is_theme = p
                .extension()
                .and_then(|s| s.to_str())
                .is_some_and(|ext| USER_THEME_EXTS.contains(&ext));
            if is_theme
                && let Some(stem) = p.file_stem().and_then(|s| s.to_str())
                && !names.iter().any(|n| n == stem)
            {
                user.push(stem.to_string());
            }
        }
    }
    user.sort();
    user.dedup();
    names.extend(user);
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_roundtrip() {
        let c = Color::hex("#569cd6").unwrap();
        assert_eq!(c.to_hex(), "#569cd6");
        let c = Color::hex("#ffffff80").unwrap();
        assert!((c.a - 0.502).abs() < 0.01);
        assert!(Color::hex("nope").is_err());
        assert!(Color::hex("#+12345").is_err());
        assert!(Color::hex("#-12345").is_err());
    }

    #[test]
    fn builtin_themes_parse() {
        for t in builtin::all() {
            // serialize → parse round trip keeps every token
            let s = toml::to_string(&t).unwrap();
            let back = Theme::parse_toml(&s).unwrap();
            assert_eq!(back.accent, t.accent, "{}", t.name);
            assert_eq!(back.ansi[1], t.ansi[1], "{}", t.name);
            assert!(back.background_image.is_none(), "{}", t.name);
        }
        assert_eq!(builtin::all().len(), BUILTIN_NAMES.len());
    }

    #[test]
    fn window_ground_matches_warp_layers() {
        let mut t = Theme::default();
        assert_eq!(t.window_ground(), t.bg);
        assert_eq!(t.ground(t.bg_chrome), t.bg_chrome);
        t.ground_alpha = 0.5;
        let g = t.window_ground();
        // two stacked layers at 0.5 cover 0.75
        assert!((g.a - 0.75).abs() < 1e-6);
        // composite over black: 0.5·bg + 0.25·surface_2, surface_2 = bg + ~10 % fg
        let surface_2 = t.bg.r + (t.fg.r - t.bg.r) * (25.0 / 255.0);
        assert!((g.r * g.a - (0.5 * t.bg.r + 0.25 * surface_2)).abs() < 1e-5);
        // ground surfaces draw nothing of their own while translucent
        assert_eq!(t.ground(t.bg_chrome).a, 0.0);
    }

    #[test]
    fn a_pane_ground_turns_the_window_ground_into_its_own() {
        let mut window = Theme::default();
        let mut pane = builtin::tokyo_night();
        // opaque: the pane's own bg
        assert_eq!(pane.pane_ground(&window), pane.bg);
        window.ground_alpha = 0.6;
        pane.ground_alpha = 0.6;
        // the same theme adds no cover: the pane stays as see-through
        assert_eq!(window.pane_ground(&window).a, 0.0);
        // another theme: drawn over the window ground, the colour is the
        // pane theme's ground, with cover below full
        let (fill, w, t) = (
            pane.pane_ground(&window),
            window.window_ground(),
            pane.window_ground(),
        );
        assert!(fill.a > 0.0 && fill.a < 1.0, "{fill:?}");
        for (c, w, t) in [(fill.r, w.r, t.r), (fill.g, w.g, t.g), (fill.b, w.b, t.b)] {
            assert!((c * fill.a + w * (1.0 - fill.a) - t).abs() < 1e-4);
        }
    }

    #[test]
    fn chips_and_ghost_stay_readable_on_a_translucent_window() {
        let mut t = Theme::default();
        let opaque_ghost = t.ghost_text();
        assert_eq!(opaque_ghost, t.syntax.ghost);
        t.ground_alpha = 0.5;
        // the chip keeps a solid ground whatever shows through
        assert_eq!(t.chip_fill().a, 1.0);
        assert!(t.chip_fill().luminance() < 0.05);
        // the ghost moves toward the text colour, away from the lighter ground
        assert!(t.ghost_text().luminance() > opaque_ghost.luminance());
    }

    #[test]
    fn mix_and_luminance() {
        let mid = Color::BLACK.mix(Color::WHITE, 0.5);
        assert!((mid.r - 0.5).abs() < 1e-6 && (mid.a - 1.0).abs() < 1e-6);
        assert_eq!(Color::BLACK.mix(Color::WHITE, 0.0), Color::BLACK);
        assert_eq!(Color::BLACK.mix(Color::WHITE, 2.0), Color::WHITE, "clamped");
        assert!(Color::WHITE.luminance() > 0.999);
        assert!(Color::BLACK.luminance() < 1e-6);
        // WCAG reference: #767676 on white is ~4.54:1
        assert!((contrast(Color::hex("#767676").unwrap(), Color::WHITE) - 4.54).abs() < 0.02);
    }

    /// WCAG 2.x contrast ratio, 1.0..=21.0.
    fn contrast(a: Color, b: Color) -> f32 {
        a.contrast(b)
    }

    #[test]
    fn min_contrast_lifts_only_unreadable_text() {
        let bg = Color::hex("#1e1e1e").unwrap();
        // already readable, or the option off: unchanged
        let fg = Color::hex("#d4d4d4").unwrap();
        assert_eq!(fg.with_min_contrast(bg, 4.5), fg);
        let dark = Color::hex("#303030").unwrap();
        assert_eq!(dark.with_min_contrast(bg, 1.0), dark);
        // dark grey on a dark ground lightens just enough, keeping alpha
        let lifted = dark.with_alpha(0.66).with_min_contrast(bg, 4.5);
        assert!(contrast(lifted, bg) >= 4.5, "{:.2}", contrast(lifted, bg));
        assert!(contrast(lifted, bg) < 4.7, "a minimal change");
        assert!(lifted.luminance() > dark.luminance());
        assert_eq!(lifted.a, 0.66);
        // on a light ground the same grey darkens
        let light_bg = Color::hex("#f5f5f5").unwrap();
        let pale = Color::hex("#e0e0e0").unwrap();
        let darker = pale.with_min_contrast(light_bg, 7.0);
        assert!(contrast(darker, light_bg) >= 7.0);
        assert!(darker.luminance() < pale.luminance());
        // a mid grey ground cannot reach 21: the better extreme
        let grey = Color::hex("#777777").unwrap();
        let best = grey.with_min_contrast(grey, 21.0);
        assert_eq!(best, Color::BLACK);
    }

    #[test]
    fn light_default_is_light_and_readable() {
        let t = builtin::light_default();
        assert!(t.is_light());
        assert!(!builtin::dark_default().is_light());

        let bg = t.bg;
        assert!(
            contrast(t.fg, bg) >= 7.0,
            "fg {} on bg: {:.2}:1",
            t.fg.to_hex(),
            contrast(t.fg, bg)
        );
        for (label, c) in [
            ("text_bright", t.text_bright),
            ("text_ui", t.text_ui),
            ("text_dim", t.text_dim),
        ] {
            assert!(contrast(c, bg) >= 4.5, "{label}: {:.2}:1", contrast(c, bg));
        }
        assert!(
            contrast(t.text_muted, bg) >= 3.0,
            "text_muted: {:.2}:1",
            contrast(t.text_muted, bg)
        );
        for (label, c) in [
            ("accent", t.accent),
            ("success", t.success),
            ("error", t.error),
            ("warning", t.warning),
            ("syntax.command", t.syntax.command),
            ("syntax.flag", t.syntax.flag),
            ("syntax.string", t.syntax.string),
        ] {
            assert!(contrast(c, bg) >= 4.5, "{label}: {:.2}:1", contrast(c, bg));
        }

        // inverted surface ramp: chrome sinks toward white, surface lifts to white
        assert!(t.bg_chrome.luminance() < t.bg.luminance());
        assert!(t.surface.luminance() >= t.bg.luminance());
    }

    #[test]
    fn background_image_parses_from_toml_and_defaults_to_none() {
        let base = builtin::LIGHT_DEFAULT;
        let with_img = format!("{base}\n[background_image]\npath = \"wall.png\"\nopacity = 0.5\n");
        let t = Theme::parse_toml(&with_img).unwrap();
        let img = t.background_image.unwrap();
        assert_eq!(img.path, "wall.png");
        assert!((img.opacity - 0.5).abs() < 1e-6);

        assert!(Theme::parse_toml(base).unwrap().background_image.is_none());

        // opacity is optional and defaults to fully opaque
        let no_opacity = format!("{base}\n[background_image]\npath = \"wall.png\"\n");
        let img = Theme::parse_toml(&no_opacity)
            .unwrap()
            .background_image
            .unwrap();
        assert!((img.opacity - 1.0).abs() < 1e-6);
    }

    #[test]
    fn available_themes_lists_builtins_first_in_order() {
        let names = available_themes();
        assert_eq!(&names[..BUILTIN_NAMES.len()], &BUILTIN_NAMES[..]);
        let mut deduped = names.clone();
        deduped.sort();
        deduped.dedup();
        assert_eq!(deduped.len(), names.len(), "no duplicate stems");
    }

    /// Every builtin: parses, reads as dark or light as its ground says,
    /// and keeps text readable on its ground.
    #[test]
    fn every_builtin_is_readable() {
        assert!(BUILTIN_NAMES.len() >= 13);
        for name in BUILTIN_NAMES {
            let t = builtin::by_name(name).unwrap();
            let light = name.contains("light") || name.contains("latte");
            assert_eq!(t.is_light(), light, "{name}");
            // the colour ANSI slots stay visible on the ground (black and
            // white are the ground's own shade on one side or the other);
            // the schemes keep their upstream values, whose light yellows
            // sit near 2.2:1. light-default mirrors Warp's own (yellow
            // 1.3:1, user decision 2026-10-16) and is left as Warp has it.
            if name != "light-default" {
                for i in (1..7).chain(9..15) {
                    let r = contrast(t.ansi[i], t.bg);
                    assert!(r >= 2.0, "{name} ansi {i}: {r:.2}:1");
                }
            }
            for (label, c, floor) in [
                ("fg", t.fg, 4.5),
                ("text_ui", t.text_ui, 4.5),
                ("text_dim", t.text_dim, 3.0),
                ("text_muted", t.text_muted, 2.5),
                ("accent", t.accent, 3.0),
                ("error", t.error, 3.0),
            ] {
                let r = contrast(c, t.bg);
                assert!(r >= floor, "{name} {label}: {r:.2}:1");
            }
        }
        // a new builtin resolves by its name, with spaces, in any case
        assert_eq!(
            builtin::by_name("Solarized Light").unwrap().name,
            "Solarized Light"
        );
        assert!(builtin::by_name("solarized").is_none());
    }

    #[test]
    fn a_user_file_named_like_a_builtin_replaces_it() {
        let dir = std::env::temp_dir().join(format!("rift-theme-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mine =
            builtin::DARK_DEFAULT.replace("name = \"Dark Default\"", "name = \"My Dracula\"");
        std::fs::write(dir.join("dracula.toml"), mine).unwrap();
        assert_eq!(
            try_load_in(Some(&dir), "dracula").unwrap().name,
            "My Dracula"
        );
        // no file: the builtin
        assert_eq!(try_load_in(Some(&dir), "nord").unwrap().name, "Nord");
        // a broken file says so instead of quietly showing the builtin
        std::fs::write(dir.join("monokai.toml"), "name = ").unwrap();
        assert!(
            try_load_in(Some(&dir), "monokai")
                .unwrap_err()
                .contains("monokai.toml")
        );
        assert_eq!(try_load_in(None, "dracula").unwrap().name, "Dracula");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_by_name_resolves_builtins() {
        assert_eq!(load_by_name("light-default").name, "Light Default");
        assert_eq!(load_by_name("Light Default").name, "Light Default");
        assert_eq!(load_by_name("dark-default").name, "Dark Default");
        assert_eq!(
            load_by_name("definitely-not-a-theme").name,
            "Dark Default",
            "unknown falls back"
        );
    }
}
