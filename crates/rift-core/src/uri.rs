//! `rift://` links (Warp's `warp://` scheme): open a tab or a window in a
//! folder, a launch configuration or Tab Config by name, or Settings. A
//! link runs no command of its own (a layout with commands asks first) and
//! never points Rift at a file: a `path`
//! that is not an existing directory is refused.

use std::path::PathBuf;

pub const SCHEME: &str = "rift";

/// What a link asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UriAction {
    /// `rift://action/new_tab?path=…` (no `path`: where a new tab starts).
    NewTab(Option<PathBuf>),
    /// `rift://action/new_window?path=…`.
    NewWindow(Option<PathBuf>),
    /// `rift://launch/<name>` and `rift://tab_config/<name>`: a saved
    /// layout, matched by name without regard to case; `new_window`
    /// (`?new_window=true`) opens it in a window of its own.
    Launch { name: String, new_window: bool },
    /// `rift://settings[?q=…]`: Settings, its search filled in.
    Settings(Option<String>),
}

/// Parse a `rift://` link. `Err` says why it is refused.
pub fn parse(link: &str) -> Result<UriAction, String> {
    let rest = link
        .strip_prefix(SCHEME)
        .and_then(|r| r.strip_prefix("://"))
        .ok_or_else(|| format!("not a {SCHEME}:// link"))?;
    let (path, query) = rest.split_once('?').unwrap_or((rest, ""));
    let path = path.trim_end_matches('/');
    let param = |key: &str| -> Option<String> {
        query.split('&').find_map(|pair| {
            let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
            // `+` is a space in a query, as browsers encode it
            (crate::osc::percent_decode(k) == key)
                .then(|| crate::osc::percent_decode(&v.replace('+', " ")))
        })
    };
    let folder = || -> Result<Option<PathBuf>, String> {
        let Some(p) = param("path").filter(|p| !p.is_empty()) else {
            return Ok(None);
        };
        let dir = crate::launch::expand_tilde(&p);
        if !dir.is_absolute() {
            return Err(format!("not an absolute path: {p}"));
        }
        if !dir.is_dir() {
            return Err(format!("not a folder: {p}"));
        }
        Ok(Some(dir))
    };
    match path.split_once('/').unwrap_or((path, "")) {
        ("action", "new_tab") => folder().map(UriAction::NewTab),
        ("action", "new_window") => folder().map(UriAction::NewWindow),
        ("launch" | "tab_config", name) => {
            let name = crate::osc::percent_decode(name);
            if name.is_empty() {
                return Err("the link names no launch configuration".into());
            }
            if name.contains('/') || name.contains('\\') || name.contains("..") {
                return Err(format!("not a launch configuration name: {name}"));
            }
            Ok(UriAction::Launch {
                name,
                new_window: param("new_window").is_some_and(|v| v == "true" || v == "1"),
            })
        }
        ("settings", "") => Ok(UriAction::Settings(param("q").filter(|q| !q.is_empty()))),
        _ => Err(format!("unknown link: {link}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opens_tabs_and_windows_in_folders() {
        let dir = std::env::temp_dir().canonicalize().unwrap();
        let encoded = dir.to_string_lossy().replace(' ', "%20");
        assert_eq!(
            parse(&format!("rift://action/new_tab?path={encoded}")),
            Ok(UriAction::NewTab(Some(dir.clone())))
        );
        assert_eq!(
            parse(&format!("rift://action/new_window/?path={encoded}&x=1")),
            Ok(UriAction::NewWindow(Some(dir)))
        );
        assert_eq!(parse("rift://action/new_tab"), Ok(UriAction::NewTab(None)));
    }

    #[test]
    fn decodes_and_expands_the_path() {
        let home = std::env::var("HOME").unwrap();
        assert_eq!(
            parse("rift://action/new_tab?path=%7E"),
            Ok(UriAction::NewTab(Some(PathBuf::from(&home))))
        );
        let space = std::env::temp_dir().join("rift uri test");
        std::fs::create_dir_all(&space).unwrap();
        let link = format!(
            "rift://action/new_tab?path={}",
            space.to_string_lossy().replace(' ', "+")
        );
        assert_eq!(parse(&link), Ok(UriAction::NewTab(Some(space.clone()))));
        let _ = std::fs::remove_dir(&space);
    }

    #[test]
    fn refuses_files_relative_paths_and_missing_folders() {
        let file = std::env::temp_dir().join("rift-uri-test-file");
        std::fs::write(&file, "#!/bin/sh\necho no\n").unwrap();
        let link = format!("rift://action/new_tab?path={}", file.display());
        assert!(parse(&link).is_err(), "a file is never opened or run");
        let _ = std::fs::remove_file(&file);
        assert!(parse("rift://action/new_tab?path=tmp").is_err());
        assert!(parse("rift://action/new_window?path=/no/such/rift/dir").is_err());
    }

    #[test]
    fn names_layouts_without_paths() {
        assert_eq!(
            parse("rift://launch/My%20Layout"),
            Ok(UriAction::Launch {
                name: "My Layout".into(),
                new_window: false
            })
        );
        assert_eq!(
            parse("rift://tab_config/dev?new_window=true"),
            Ok(UriAction::Launch {
                name: "dev".into(),
                new_window: true
            })
        );
        for bad in [
            "rift://launch/",
            "rift://launch/../x",
            "rift://launch/a%2Fb",
            "rift://tab_config/..",
        ] {
            assert!(parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn opens_settings_with_a_search() {
        assert_eq!(parse("rift://settings"), Ok(UriAction::Settings(None)));
        assert_eq!(
            parse("rift://settings?q=font+size"),
            Ok(UriAction::Settings(Some("font size".into())))
        );
    }

    #[test]
    fn refuses_unknown_links() {
        for bad in [
            "warp://action/new_tab",
            "rift:action/new_tab",
            "rift://action/run?cmd=rm",
            "rift://action/open_file?path=/tmp",
            "rift://settings/appearance",
            "rift://",
        ] {
            assert!(parse(bad).is_err(), "{bad}");
        }
    }
}
