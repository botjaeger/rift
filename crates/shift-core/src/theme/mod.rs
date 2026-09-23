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
}

impl Theme {
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
pub const BUILTIN_NAMES: [&str; 4] = [
    "dark-default",
    "light-default",
    "catppuccin-mocha",
    "tokyo-night",
];

/// Extensions probed in `~/.config/shift/themes/`, in precedence order.
const USER_THEME_EXTS: [&str; 3] = ["toml", "yaml", "yml"];

/// Resolve a theme by config name: builtin first, then
/// `~/.config/shift/themes/<name>.{toml,yaml,yml}`, else dark-default.
///
/// `.yaml`/`.yml` files are read as Warp themes (see [`parse_warp_yaml`]).
pub fn load_by_name(name: &str) -> Theme {
    match name.to_lowercase().as_str() {
        "dark-default" | "dark default" => return builtin::dark_default(),
        "light-default" | "light default" => return builtin::light_default(),
        "catppuccin-mocha" | "catppuccin mocha" => return builtin::catppuccin_mocha(),
        "tokyo-night" | "tokyo night" => return builtin::tokyo_night(),
        _ => {}
    }
    if let Some(dir) = crate::config::themes_dir() {
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
                    return t;
                }
                Err(e) => log::warn!("theme {name}.{ext}: parse error: {e}"),
            }
        }
    }
    log::warn!("theme {name:?} not found, using dark-default");
    builtin::dark_default()
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
        let (x, y) = (a.luminance(), b.luminance());
        let (hi, lo) = if x > y { (x, y) } else { (y, x) };
        (hi + 0.05) / (lo + 0.05)
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
