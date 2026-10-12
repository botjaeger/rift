use super::Theme;
use std::sync::OnceLock;

pub const DARK_DEFAULT: &str = include_str!("dark-default.toml");
pub const LIGHT_DEFAULT: &str = include_str!("light-default.toml");
pub const CATPPUCCIN_MOCHA: &str = include_str!("catppuccin-mocha.toml");
pub const TOKYO_NIGHT: &str = include_str!("tokyo-night.toml");

fn cached(cell: &'static OnceLock<Theme>, src: &str) -> Theme {
    cell.get_or_init(|| Theme::parse_toml(src).expect("builtin theme must parse"))
        .clone()
}

pub fn dark_default() -> Theme {
    static T: OnceLock<Theme> = OnceLock::new();
    cached(&T, DARK_DEFAULT)
}

pub fn light_default() -> Theme {
    static T: OnceLock<Theme> = OnceLock::new();
    cached(&T, LIGHT_DEFAULT)
}

pub fn catppuccin_mocha() -> Theme {
    static T: OnceLock<Theme> = OnceLock::new();
    cached(&T, CATPPUCCIN_MOCHA)
}

pub fn tokyo_night() -> Theme {
    static T: OnceLock<Theme> = OnceLock::new();
    cached(&T, TOKYO_NIGHT)
}

/// All builtin themes, in display order.
pub fn all() -> Vec<Theme> {
    vec![
        dark_default(),
        light_default(),
        catppuccin_mocha(),
        tokyo_night(),
    ]
}
