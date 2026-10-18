use super::Theme;
use std::sync::OnceLock;

pub const DARK_DEFAULT: &str = include_str!("dark-default.toml");
pub const LIGHT_DEFAULT: &str = include_str!("light-default.toml");
pub const CATPPUCCIN_MOCHA: &str = include_str!("catppuccin-mocha.toml");
pub const TOKYO_NIGHT: &str = include_str!("tokyo-night.toml");

/// Every builtin theme as (config name, TOML source), in picker display
/// order: Rift's two defaults and the first two schemes it shipped, then
/// the well-known schemes alphabetically.
pub const SOURCES: [(&str, &str); 13] = [
    ("dark-default", DARK_DEFAULT),
    ("light-default", LIGHT_DEFAULT),
    ("catppuccin-mocha", CATPPUCCIN_MOCHA),
    ("tokyo-night", TOKYO_NIGHT),
    ("catppuccin-latte", include_str!("catppuccin-latte.toml")),
    ("dracula", include_str!("dracula.toml")),
    ("gruvbox-dark", include_str!("gruvbox-dark.toml")),
    ("gruvbox-light", include_str!("gruvbox-light.toml")),
    ("monokai", include_str!("monokai.toml")),
    ("nord", include_str!("nord.toml")),
    ("one-dark", include_str!("one-dark.toml")),
    ("solarized-dark", include_str!("solarized-dark.toml")),
    ("solarized-light", include_str!("solarized-light.toml")),
];

/// The builtin theme `name` (its config name, or that name with spaces
/// for dashes, any case), parsed once.
pub fn by_name(name: &str) -> Option<Theme> {
    static CACHE: [OnceLock<Theme>; SOURCES.len()] = [const { OnceLock::new() }; SOURCES.len()];
    let wanted = name.trim().to_lowercase().replace(' ', "-");
    let i = SOURCES.iter().position(|(n, _)| *n == wanted)?;
    Some(
        CACHE[i]
            .get_or_init(|| Theme::parse_toml(SOURCES[i].1).expect("builtin theme must parse"))
            .clone(),
    )
}

fn named(name: &str) -> Theme {
    by_name(name).expect("builtin theme")
}

pub fn dark_default() -> Theme {
    named("dark-default")
}

pub fn light_default() -> Theme {
    named("light-default")
}

pub fn catppuccin_mocha() -> Theme {
    named("catppuccin-mocha")
}

pub fn tokyo_night() -> Theme {
    named("tokyo-night")
}

/// All builtin themes, in display order.
pub fn all() -> Vec<Theme> {
    SOURCES.iter().map(|(n, _)| named(n)).collect()
}
