//! `rift://` links (Warp's `warp://` scheme): open a tab or a window in a
//! folder, a launch configuration or Tab Config by name, a file in the
//! editor at a line, or Settings. A link runs no command of its own (a
//! layout with commands asks first) and never runs a file: a tab's `path`
//! that is not an existing directory is refused, and `open_file` only
//! opens an existing regular file in the configured editor or the viewer.
//!
//! [`open_path_link`] turns a folder or file named from outside (the
//! command line, Finder, the Dock, a desktop launcher) into the same link,
//! so it reaches the running Rift the way a link does.

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
    /// `rift://action/open_file?path=…&line=…[&col=…]`: an existing
    /// regular file in `link_editor` at that line (1-based), else the
    /// viewer.
    OpenFile {
        path: PathBuf,
        line: Option<u32>,
        col: Option<u32>,
    },
}

/// The link that opens `path` (absolute; a file opens its folder) in a new
/// tab, or a new window with `new_window`. `Err` says why it cannot.
pub fn open_path_link(path: &std::path::Path, new_window: bool) -> Result<String, String> {
    if !path.is_absolute() {
        return Err(format!("not an absolute path: {}", path.display()));
    }
    let meta =
        std::fs::metadata(path).map_err(|e| format!("{}: {}", path.display(), io_reason(&e)))?;
    let dir = if meta.is_dir() {
        path
    } else {
        path.parent()
            .ok_or_else(|| format!("no folder for {}", path.display()))?
    };
    let action = if new_window { "new_window" } else { "new_tab" };
    Ok(format!(
        "{SCHEME}://action/{action}?path={}",
        percent_encode(&dir.to_string_lossy())
    ))
}

/// The local path a `file://` URL names (a desktop launcher's `%U`), or
/// `None` for another host or scheme.
pub fn file_url_path(url: &str) -> Option<PathBuf> {
    let rest = url.strip_prefix("file://")?;
    let rest = rest.strip_prefix("localhost").unwrap_or(rest);
    if !rest.starts_with('/') {
        return None;
    }
    let rest = rest.split(['?', '#']).next().unwrap_or(rest);
    // a folder's URL ends in `/`; its path does not (the root aside)
    let path = crate::osc::percent_decode(rest);
    let trimmed = path.trim_end_matches('/');
    Some(PathBuf::from(if trimmed.is_empty() {
        "/"
    } else {
        trimmed
    }))
}

/// The link for a URL macOS hands the app (`application:openURLs:`, an
/// open-documents event from Finder, the Dock or `open -a Rift <dir>`): a
/// `rift://` link as it is, a folder or file (`file://`) as
/// [`open_path_link`] makes it.
pub fn opened_url_link(url: &str) -> Result<String, String> {
    if url.starts_with(&format!("{SCHEME}://")) {
        return Ok(url.to_string());
    }
    match file_url_path(url) {
        Some(path) => open_path_link(&path, false),
        None => Err(format!("not a folder or {SCHEME}:// link: {url}")),
    }
}

/// "No such file or directory" without the `(os error 2)` suffix.
fn io_reason(e: &std::io::Error) -> String {
    match e.kind() {
        std::io::ErrorKind::NotFound => "no such file or directory".into(),
        std::io::ErrorKind::PermissionDenied => "permission denied".into(),
        _ => e.to_string(),
    }
}

/// Percent-encode everything but RFC 3986 unreserved characters and `/`.
fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~' | b'/') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
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
        ("action", "open_file") => {
            let Some(p) = param("path").filter(|p| !p.is_empty()) else {
                return Err("the link names no file".into());
            };
            let path = crate::launch::expand_tilde(&p);
            if !path.is_absolute() {
                return Err(format!("not an absolute path: {p}"));
            }
            if !path.is_file() {
                return Err(format!("not a file: {p}"));
            }
            // 1-based; anything else (0, a sign, text) is refused
            let number = |key: &str| -> Result<Option<u32>, String> {
                match param(key).filter(|v| !v.is_empty()) {
                    None => Ok(None),
                    Some(v) => match v.parse::<u32>() {
                        Ok(n) if n > 0 && v.bytes().all(|b| b.is_ascii_digit()) => Ok(Some(n)),
                        _ => Err(format!("not a {key} number: {v}")),
                    },
                }
            };
            let line = number("line")?;
            let col = number("col")?;
            Ok(UriAction::OpenFile {
                path,
                line,
                col: line.and(col),
            })
        }
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
    fn opens_files_at_a_line_and_never_folders() {
        let dir = std::env::temp_dir().join(format!("rift uri open {}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("main.rs");
        std::fs::write(&file, "fn main() {}\n").unwrap();
        let enc = percent_encode(&file.to_string_lossy());
        assert_eq!(
            parse(&format!("rift://action/open_file?path={enc}&line=12&col=3")),
            Ok(UriAction::OpenFile {
                path: file.clone(),
                line: Some(12),
                col: Some(3)
            })
        );
        assert_eq!(
            parse(&format!("rift://action/open_file?path={enc}")),
            Ok(UriAction::OpenFile {
                path: file.clone(),
                line: None,
                col: None
            })
        );
        // a column without a line means nothing
        assert_eq!(
            parse(&format!("rift://action/open_file?path={enc}&col=3")),
            Ok(UriAction::OpenFile {
                path: file.clone(),
                line: None,
                col: None
            })
        );
        let folder = percent_encode(&dir.to_string_lossy());
        for bad in [
            format!("rift://action/open_file?path={folder}"),
            format!("rift://action/open_file?path={enc}&line=0"),
            format!("rift://action/open_file?path={enc}&line=-1"),
            format!("rift://action/open_file?path={enc}&line=%2B4"),
            format!("rift://action/open_file?path={enc}&line=x"),
            format!("rift://action/open_file?path={enc}&line=99999999999"),
            "rift://action/open_file?path=main.rs&line=1".into(),
            "rift://action/open_file?path=/no/such/rift/file&line=1".into(),
            "rift://action/open_file".into(),
        ] {
            assert!(parse(&bad).is_err(), "{bad}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_path_from_outside_becomes_a_folder_link() {
        let dir = std::env::temp_dir().join(format!("rift uri path {}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("notes #1.txt");
        std::fs::write(&file, "x").unwrap();
        // a folder, and a file (its folder), round-trip through the parser
        let tab = open_path_link(&dir, false).unwrap();
        assert!(tab.starts_with("rift://action/new_tab?path=/"), "{tab}");
        assert_eq!(parse(&tab), Ok(UriAction::NewTab(Some(dir.clone()))));
        assert_eq!(
            parse(&open_path_link(&file, false).unwrap()),
            Ok(UriAction::NewTab(Some(dir.clone())))
        );
        assert_eq!(
            parse(&open_path_link(&file, true).unwrap()),
            Ok(UriAction::NewWindow(Some(dir.clone())))
        );
        let missing = open_path_link(&dir.join("gone"), false).unwrap_err();
        assert!(missing.ends_with("no such file or directory"), "{missing}");
        assert!(open_path_link(std::path::Path::new("rel/dir"), false).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn file_urls_name_local_paths() {
        assert_eq!(
            file_url_path("file:///tmp/a%20dir/"),
            Some(PathBuf::from("/tmp/a dir"))
        );
        assert_eq!(file_url_path("file:///"), Some(PathBuf::from("/")));
        assert_eq!(
            file_url_path("file://localhost/srv/x"),
            Some(PathBuf::from("/srv/x"))
        );
        assert_eq!(file_url_path("file://other-host/srv/x"), None);
        assert_eq!(file_url_path("https://example.com/x"), None);
    }

    #[test]
    fn opened_documents_become_links() {
        // what Finder and the Dock hand over: a folder's file URL (with
        // its trailing slash), a file's, a rift:// link
        let dir = std::env::temp_dir()
            .canonicalize()
            .unwrap()
            .join(format!("rift odoc {}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.txt"), "").unwrap();
        let url = format!("file://{}/", percent_encode(&dir.to_string_lossy()));
        let link = opened_url_link(&url).unwrap();
        assert_eq!(parse(&link), Ok(UriAction::NewTab(Some(dir.clone()))));
        let file = format!("file://{}/a.txt", percent_encode(&dir.to_string_lossy()));
        assert_eq!(
            parse(&opened_url_link(&file).unwrap()),
            Ok(UriAction::NewTab(Some(dir.clone())))
        );
        assert_eq!(
            opened_url_link("rift://settings").as_deref(),
            Ok("rift://settings")
        );
        assert!(opened_url_link("https://example.com/").is_err());
        assert!(opened_url_link(&format!("file://{}/gone/", dir.display())).is_err());
        let _ = std::fs::remove_dir_all(&dir);
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
