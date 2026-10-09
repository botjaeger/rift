//! Warp terminal theme (YAML) → Shift [`Theme`].
//!
//! Warp themes carry only a handful of base colors (accent, background,
//! foreground, a 16-entry ANSI palette and a light/dark hint). Every other
//! Shift token is derived from those, following the ratios the builtin TOML
//! themes use: chrome/gutter sink away from the text, the modal surface lifts
//! toward it, the hairline border is a low-alpha overlay of the lift color.

use super::{BackgroundImage, Color, SyntaxColors, Theme};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct WarpTheme {
    #[serde(default)]
    name: Option<String>,
    accent: Color,
    background: Color,
    foreground: Color,
    /// `darker` (dark theme, the default) or `lighter`.
    #[serde(default)]
    details: Option<String>,
    terminal_colors: WarpTerminalColors,
    #[serde(default)]
    background_image: Option<WarpBackgroundImage>,
}

#[derive(Debug, Deserialize)]
struct WarpTerminalColors {
    normal: WarpPalette,
    bright: WarpPalette,
}

#[derive(Debug, Deserialize)]
struct WarpPalette {
    black: Color,
    red: Color,
    green: Color,
    yellow: Color,
    blue: Color,
    magenta: Color,
    cyan: Color,
    white: Color,
}

impl WarpPalette {
    fn as_array(&self) -> [Color; 8] {
        [
            self.black,
            self.red,
            self.green,
            self.yellow,
            self.blue,
            self.magenta,
            self.cyan,
            self.white,
        ]
    }
}

#[derive(Debug, Deserialize)]
struct WarpBackgroundImage {
    path: String,
    /// Warp stores this as a percentage, 0..=100.
    #[serde(default = "hundred")]
    opacity: f32,
}

fn hundred() -> f32 {
    100.0
}

/// Parse a Warp theme YAML document into a full Shift [`Theme`].
///
/// `name` is the fallback display name used when the document has no `name:`
/// key (callers pass the file stem).
pub fn parse_warp_yaml(name: &str, src: &str) -> Result<Theme, String> {
    let w: WarpTheme = serde_yaml_ng::from_str(src).map_err(|e| e.to_string())?;

    let lighter = w
        .details
        .as_deref()
        .is_some_and(|d| d.trim().eq_ignore_ascii_case("lighter"));

    // `deep` sinks a surface away from the text, `lift` raises it toward the
    // text. On a dark theme that is black/white; on a light theme, reversed.
    let (deep, lift) = if lighter {
        (Color::WHITE, Color::BLACK)
    } else {
        (Color::BLACK, Color::WHITE)
    };

    let bg = w.background;
    let fg = w.foreground;
    let normal = w.terminal_colors.normal.as_array();
    let bright = w.terminal_colors.bright.as_array();

    let mut ansi = [Color::TRANSPARENT; 16];
    ansi[..8].copy_from_slice(&normal);
    ansi[8..].copy_from_slice(&bright);

    let n_red = normal[1];
    let n_green = normal[2];
    let n_yellow = normal[3];
    let n_cyan = normal[6];
    let orange = n_yellow.mix(n_red, 0.5);

    let text_muted = fg.mix(bg, 0.54);

    Ok(Theme {
        name: w.name.unwrap_or_else(|| name.to_string()),

        bg,
        bg_chrome: bg.mix(deep, 0.18),
        bg_gutter: bg.mix(deep, 0.26),
        surface: bg.mix(lift, 0.05),
        border: lift.with_alpha(0.07),
        pane_border_inactive: bg.mix(lift, 0.14),

        fg,
        text_bright: fg.mix(lift, 0.12),
        text_ui: fg.mix(bg, 0.08),
        text_dim: fg.mix(bg, 0.38),
        text_muted,

        accent: w.accent,
        success: n_green,
        error: n_red,
        warning: n_yellow,
        cursor: w.accent,
        selection: w.accent.with_alpha(0.25),
        search_match: n_yellow.with_alpha(0.40),
        search_active: orange.with_alpha(0.65),

        syntax: SyntaxColors {
            command: w.accent,
            flag: n_cyan,
            string: n_yellow.mix(n_red, 0.3),
            ghost: text_muted,
        },

        ansi,
        background_image: w.background_image.map(|b| BackgroundImage {
            path: b.path,
            opacity: (b.opacity / 100.0).clamp(0.0, 1.0),
        }),
        ground_alpha: 1.0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const DRACULA: &str = r#"
accent: '#bd93f9'
background: '#282a36'
foreground: '#f8f8f2'
details: darker
terminal_colors:
  normal:
    black: '#21222c'
    red: '#ff5555'
    green: '#50fa7b'
    yellow: '#f1fa8c'
    blue: '#bd93f9'
    magenta: '#ff79c6'
    cyan: '#8be9fd'
    white: '#f8f8f2'
  bright:
    black: '#6272a4'
    red: '#ff6e6e'
    green: '#69ff94'
    yellow: '#ffffa5'
    blue: '#d6acff'
    magenta: '#ff92df'
    cyan: '#a4ffff'
    white: '#ffffff'
background_image:
  path: warp.jpg
  opacity: 50
"#;

    #[test]
    fn dracula_yaml_derives_sane_tokens() {
        let t = parse_warp_yaml("dracula", DRACULA).unwrap();

        assert_eq!(t.name, "dracula", "no `name:` key → file stem");
        assert_eq!(t.bg, Color::hex("#282a36").unwrap());
        assert_eq!(t.fg, Color::hex("#f8f8f2").unwrap());
        assert_eq!(t.accent, Color::hex("#bd93f9").unwrap());
        assert_eq!(t.cursor, t.accent);
        assert_eq!(t.syntax.command, t.accent);

        // palette order: normal 0..8 then bright 8..16
        assert_eq!(t.ansi[0], Color::hex("#21222c").unwrap());
        assert_eq!(t.ansi[1], Color::hex("#ff5555").unwrap());
        assert_eq!(t.ansi[2], Color::hex("#50fa7b").unwrap());
        assert_eq!(t.ansi[6], Color::hex("#8be9fd").unwrap());
        assert_eq!(t.ansi[8], Color::hex("#6272a4").unwrap());
        assert_eq!(t.ansi[15], Color::WHITE);

        // semantics track the normal palette
        assert_eq!(t.error, t.ansi[1]);
        assert_eq!(t.success, t.ansi[2]);
        assert_eq!(t.warning, t.ansi[3]);
        assert_eq!(t.syntax.flag, t.ansi[6]);

        // `details: darker` → dark theme, chrome sinks, surface lifts
        assert!(!t.is_light());
        assert!(t.bg_chrome.luminance() < t.bg.luminance());
        assert!(t.bg_gutter.luminance() < t.bg_chrome.luminance());
        assert!(t.surface.luminance() > t.bg.luminance());
        assert!(t.border.a < 0.2, "hairline is a low-alpha overlay");

        // text ramp: bright > fg > ui > dim > muted
        assert!(t.text_bright.luminance() > t.fg.luminance());
        assert!(t.fg.luminance() > t.text_ui.luminance());
        assert!(t.text_ui.luminance() > t.text_dim.luminance());
        assert!(t.text_dim.luminance() > t.text_muted.luminance());
        assert_eq!(t.syntax.ghost, t.text_muted);

        // selection / search are translucent so text stays readable
        assert!((t.selection.a - 0.25).abs() < 1e-6);
        assert!(t.search_match.a < 1.0 && t.search_active.a < 1.0);
    }

    #[test]
    fn lighter_details_inverts_the_surface_ramp() {
        let src = r#"
name: Paper
accent: '#1f6fb2'
background: '#ffffff'
foreground: '#1f2328'
details: lighter
terminal_colors:
  normal: {black: '#383a42', red: '#e45649', green: '#50a14f', yellow: '#c18401', blue: '#4078f2', magenta: '#a626a4', cyan: '#0184bc', white: '#a0a1a7'}
  bright: {black: '#4f525e', red: '#e06c75', green: '#98c379', yellow: '#e5c07b', blue: '#61afef', magenta: '#c678dd', cyan: '#56b6c2', white: '#ffffff'}
"#;
        let t = parse_warp_yaml("paper", src).unwrap();

        assert_eq!(t.name, "Paper", "`name:` key wins over the file stem");
        assert!(t.is_light());
        assert!(
            t.surface.luminance() < t.bg.luminance(),
            "on a light theme the lifted surface is darker than the ground"
        );
        assert!(
            t.text_bright.luminance() < t.fg.luminance(),
            "emphasis pushes toward black on a light theme"
        );
        // muted text sinks toward the (light) background, i.e. gets lighter
        assert!(t.text_muted.luminance() > t.fg.luminance());
        assert!(t.background_image.is_none());
    }

    #[test]
    fn background_image_opacity_is_normalized() {
        let t = parse_warp_yaml("dracula", DRACULA).unwrap();
        let img = t.background_image.expect("background_image parsed");
        assert_eq!(img.path, "warp.jpg");
        assert!((img.opacity - 0.5).abs() < 1e-6, "100-scale → 0..1");
    }

    #[test]
    fn missing_required_key_is_an_error() {
        let err = parse_warp_yaml("broken", "accent: '#ffffff'\n").unwrap_err();
        assert!(!err.is_empty());
    }
}
