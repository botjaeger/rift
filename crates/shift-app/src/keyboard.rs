//! Kitty keyboard protocol encoder
//! (<https://sw.kovidgoyal.net/kitty/keyboard-protocol/>).
//!
//! Programs opt in with `CSI > flags u` (alacritty's `Term` keeps the mode
//! stack and answers `CSI ? u` queries); until then keys use the legacy
//! xterm encoding in `input::key_to_bytes`. Flags:
//!
//! | bit | meaning |
//! |---|---|
//! | 1 | disambiguate escape codes (Esc, Ctrl/Alt chords, keypad → `CSI u`) |
//! | 2 | report repeat and release events |
//! | 4 | report alternate (shifted) keys |
//! | 8 | report every key as an escape code, including text and modifiers |
//! | 16 | attach the key's text to `CSI u` (with 8) |

use winit::keyboard::{Key, KeyLocation, ModifiersState, NamedKey};

use crate::input::{KeyEncoding, KeyPress};

pub const DISAMBIGUATE: u8 = 1;
pub const EVENT_TYPES: u8 = 2;
pub const ALTERNATE_KEYS: u8 = 4;
pub const ALL_KEYS: u8 = 8;
pub const ASSOCIATED_TEXT: u8 = 16;

/// Kitty flags from alacritty's terminal mode bits.
pub fn flags_from_mode(mode: shift_pty::alacritty_terminal::term::TermMode) -> u8 {
    use shift_pty::alacritty_terminal::term::TermMode;
    let mut f = 0;
    for (bit, flag) in [
        (TermMode::DISAMBIGUATE_ESC_CODES, DISAMBIGUATE),
        (TermMode::REPORT_EVENT_TYPES, EVENT_TYPES),
        (TermMode::REPORT_ALTERNATE_KEYS, ALTERNATE_KEYS),
        (TermMode::REPORT_ALL_KEYS_AS_ESC, ALL_KEYS),
        (TermMode::REPORT_ASSOCIATED_TEXT, ASSOCIATED_TEXT),
    ] {
        if mode.contains(bit) {
            f |= flag;
        }
    }
    f
}

/// How a functional key is written: `CSI number ; mods terminator`.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Functional {
    number: u32,
    terminator: char,
}

const fn f(number: u32, terminator: char) -> Option<Functional> {
    Some(Functional { number, terminator })
}

fn functional(key: &NamedKey, location: KeyLocation) -> Option<Functional> {
    let right = location == KeyLocation::Right;
    let numpad = location == KeyLocation::Numpad;
    match key {
        NamedKey::Escape => f(27, 'u'),
        NamedKey::Enter if numpad => f(57414, 'u'),
        NamedKey::Enter => f(13, 'u'),
        NamedKey::Tab => f(9, 'u'),
        NamedKey::Backspace => f(127, 'u'),
        NamedKey::Insert if numpad => f(57425, 'u'),
        NamedKey::Delete if numpad => f(57426, 'u'),
        NamedKey::ArrowLeft if numpad => f(57417, 'u'),
        NamedKey::ArrowRight if numpad => f(57418, 'u'),
        NamedKey::ArrowUp if numpad => f(57419, 'u'),
        NamedKey::ArrowDown if numpad => f(57420, 'u'),
        NamedKey::PageUp if numpad => f(57421, 'u'),
        NamedKey::PageDown if numpad => f(57422, 'u'),
        NamedKey::Home if numpad => f(57423, 'u'),
        NamedKey::End if numpad => f(57424, 'u'),
        NamedKey::Insert => f(2, '~'),
        NamedKey::Delete => f(3, '~'),
        NamedKey::ArrowLeft => f(1, 'D'),
        NamedKey::ArrowRight => f(1, 'C'),
        NamedKey::ArrowUp => f(1, 'A'),
        NamedKey::ArrowDown => f(1, 'B'),
        NamedKey::PageUp => f(5, '~'),
        NamedKey::PageDown => f(6, '~'),
        NamedKey::Home => f(1, 'H'),
        NamedKey::End => f(1, 'F'),
        NamedKey::CapsLock => f(57358, 'u'),
        NamedKey::ScrollLock => f(57359, 'u'),
        NamedKey::NumLock => f(57360, 'u'),
        NamedKey::PrintScreen => f(57361, 'u'),
        NamedKey::Pause => f(57362, 'u'),
        NamedKey::ContextMenu => f(57363, 'u'),
        NamedKey::F1 => f(1, 'P'),
        NamedKey::F2 => f(1, 'Q'),
        NamedKey::F3 => f(13, '~'),
        NamedKey::F4 => f(1, 'S'),
        NamedKey::F5 => f(15, '~'),
        NamedKey::F6 => f(17, '~'),
        NamedKey::F7 => f(18, '~'),
        NamedKey::F8 => f(19, '~'),
        NamedKey::F9 => f(20, '~'),
        NamedKey::F10 => f(21, '~'),
        NamedKey::F11 => f(23, '~'),
        NamedKey::F12 => f(24, '~'),
        NamedKey::F13 => f(57376, 'u'),
        NamedKey::F14 => f(57377, 'u'),
        NamedKey::F15 => f(57378, 'u'),
        NamedKey::F16 => f(57379, 'u'),
        NamedKey::F17 => f(57380, 'u'),
        NamedKey::F18 => f(57381, 'u'),
        NamedKey::F19 => f(57382, 'u'),
        NamedKey::F20 => f(57383, 'u'),
        NamedKey::MediaPlay => f(57428, 'u'),
        NamedKey::MediaPause => f(57429, 'u'),
        NamedKey::MediaPlayPause => f(57430, 'u'),
        NamedKey::MediaStop => f(57432, 'u'),
        NamedKey::MediaFastForward => f(57433, 'u'),
        NamedKey::MediaRewind => f(57434, 'u'),
        NamedKey::MediaTrackNext => f(57435, 'u'),
        NamedKey::MediaTrackPrevious => f(57436, 'u'),
        NamedKey::AudioVolumeDown => f(57438, 'u'),
        NamedKey::AudioVolumeUp => f(57439, 'u'),
        NamedKey::AudioVolumeMute => f(57440, 'u'),
        NamedKey::Shift => f(if right { 57447 } else { 57441 }, 'u'),
        NamedKey::Control => f(if right { 57448 } else { 57442 }, 'u'),
        NamedKey::Alt => f(if right { 57449 } else { 57443 }, 'u'),
        NamedKey::Super => f(if right { 57450 } else { 57444 }, 'u'),
        NamedKey::Hyper => f(if right { 57451 } else { 57445 }, 'u'),
        NamedKey::Meta => f(if right { 57452 } else { 57446 }, 'u'),
        _ => None,
    }
}

fn is_modifier(key: &NamedKey) -> bool {
    matches!(
        key,
        NamedKey::Shift
            | NamedKey::Control
            | NamedKey::Alt
            | NamedKey::Super
            | NamedKey::Hyper
            | NamedKey::Meta
    )
}

/// Keypad digits and operators get their own numbers.
fn keypad_number(c: char) -> Option<u32> {
    Some(match c {
        '0'..='9' => 57399 + (c as u32 - '0' as u32),
        '.' | ',' => 57409,
        '/' => 57410,
        '*' => 57411,
        '-' => 57412,
        '+' => 57413,
        '=' => 57415,
        _ => return None,
    })
}

fn single_char(key: &Key) -> Option<char> {
    match key {
        Key::Character(s) => {
            let mut cs = s.chars();
            match (cs.next(), cs.next()) {
                (Some(c), None) => Some(c),
                _ => None,
            }
        }
        Key::Named(NamedKey::Space) => Some(' '),
        _ => None,
    }
}

/// Assemble `CSI key[:shifted] [; mods[:event] [; text]] terminator`.
fn csi(
    number: u32,
    shifted: Option<u32>,
    mods: u8,
    event: u8,
    text: Option<&str>,
    terminator: char,
) -> Vec<u8> {
    let mut s = String::from("\x1b[");
    let bare = mods <= 1 && event <= 1 && text.is_none();
    // `CSI A`, not `CSI 1 A`, when nothing follows the number
    if !(bare && number == 1 && terminator != 'u' && terminator != '~') {
        s.push_str(&number.to_string());
    }
    if let Some(sh) = shifted {
        s.push(':');
        s.push_str(&sh.to_string());
    }
    if !bare {
        s.push(';');
        s.push_str(&mods.max(1).to_string());
        if event > 1 {
            s.push(':');
            s.push_str(&event.to_string());
        }
        if let Some(t) = text {
            s.push(';');
            let cps: Vec<String> = t.chars().map(|c| (c as u32).to_string()).collect();
            s.push_str(&cps.join(":"));
        }
    }
    s.push(terminator);
    s.into_bytes()
}

/// Encode `event` under the active kitty flags. `None` means nothing is
/// sent (a release the program did not ask for, an unmapped key).
pub fn encode(event: &KeyPress, mods: ModifiersState, enc: KeyEncoding) -> Option<Vec<u8>> {
    let flags = enc.kitty_flags;
    let all = flags & ALL_KEYS != 0;
    let events = flags & EVENT_TYPES != 0;
    if event.released && !events {
        return None;
    }
    let event_type = if event.released {
        3
    } else if event.repeat {
        2
    } else {
        1
    };

    // macOS Option composes characters unless it is Meta: the composed
    // text is the key, and Option is consumed by it
    let composing = mods.alt_key()
        && !enc.option_as_meta
        && cfg!(target_os = "macos")
        && event.text.as_deref().is_some_and(|t| !t.is_empty());
    let alt = mods.alt_key() && !composing;
    let bits = 1
        + u8::from(mods.shift_key())
        + 2 * u8::from(alt)
        + 4 * u8::from(mods.control_key())
        + 8 * u8::from(mods.super_key());
    let only_shift = !alt && !mods.control_key() && !mods.super_key();
    // text delivered with this press, printable only
    let text = event
        .text
        .as_deref()
        .filter(|t| !t.is_empty() && !t.chars().any(char::is_control));

    if let Key::Named(named) = &event.logical_key
        && *named != NamedKey::Space
    {
        let Some(func) = functional(named, event.location) else {
            // no kitty number: legacy bytes for presses, nothing else
            return (!event.released)
                .then(|| legacy(event, mods, enc))
                .flatten();
        };
        if is_modifier(named) && !all {
            return None;
        }
        let legacy_text_key =
            matches!(named, NamedKey::Enter | NamedKey::Tab | NamedKey::Backspace)
                && event.location != KeyLocation::Numpad;
        if !all {
            if legacy_text_key {
                // Enter/Tab/Backspace keep legacy bytes unless modified,
                // and never report release without "all keys"
                if event.released {
                    return None;
                }
                if bits == 1 {
                    return legacy(event, mods, enc);
                }
            } else if func.terminator != 'u' && bits == 1 && event_type == 1 {
                // unmodified cursor/function key press: legacy form
                // (keeps DECCKM SS3 arrows)
                return legacy(event, mods, enc);
            }
        }
        let text = if all && flags & ASSOCIATED_TEXT != 0 && !event.released {
            text
        } else {
            None
        };
        return Some(csi(
            func.number,
            None,
            bits,
            event_type,
            text,
            func.terminator,
        ));
    }

    // text-producing keys
    let numpad = event.location == KeyLocation::Numpad;
    let base_key = event.unmodified.as_ref().unwrap_or(&event.logical_key);
    let base = single_char(base_key).or_else(|| single_char(&event.logical_key));
    let keypad = base.filter(|_| numpad).and_then(keypad_number);

    if !all && keypad.is_none() {
        // legacy text for plain/shifted keys (and composed characters)
        if only_shift && !event.released {
            return text.map(|t| t.as_bytes().to_vec());
        }
        if only_shift && event.released && text.is_none() && base.is_none() {
            return None;
        }
    }

    let (number, shifted) = if let Some(n) = keypad {
        (n, None)
    } else if composing {
        // the composed character is the key
        let c = text.and_then(|t| t.chars().next())?;
        (c as u32, None)
    } else {
        let b = base?;
        let lower = b.to_lowercase().next().unwrap_or(b);
        let shifted = (flags & ALTERNATE_KEYS != 0 && mods.shift_key())
            .then(|| single_char(&event.logical_key))
            .flatten()
            .filter(|c| *c != lower)
            .map(|c| c as u32);
        (lower as u32, shifted)
    };
    let text = if all && flags & ASSOCIATED_TEXT != 0 && !event.released {
        text
    } else {
        None
    };
    Some(csi(number, shifted, bits, event_type, text, 'u'))
}

/// Legacy xterm bytes for a press (flags cleared).
fn legacy(event: &KeyPress, mods: ModifiersState, enc: KeyEncoding) -> Option<Vec<u8>> {
    crate::input::key_to_bytes(
        event,
        mods,
        KeyEncoding {
            kitty_flags: 0,
            ..enc
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const NONE: ModifiersState = ModifiersState::empty();

    fn enc(flags: u8) -> KeyEncoding {
        KeyEncoding {
            app_cursor: false,
            option_as_meta: true,
            kitty_flags: flags,
        }
    }

    fn ch(c: &str, text: Option<&str>) -> KeyPress {
        KeyPress::new(
            Key::Character(c.into()),
            text.map(str::to_string),
            Some(Key::Character(c.to_lowercase().into())),
        )
    }

    fn named(k: NamedKey) -> KeyPress {
        KeyPress::new(Key::Named(k), None, Some(Key::Named(k)))
    }

    fn s(b: Option<Vec<u8>>) -> String {
        String::from_utf8(b.expect("bytes")).unwrap()
    }

    #[test]
    fn disambiguate_keeps_plain_text_and_encodes_chords() {
        let e = enc(DISAMBIGUATE);
        assert_eq!(s(encode(&ch("a", Some("a")), NONE, e)), "a");
        assert_eq!(
            s(encode(&ch("A", Some("A")), ModifiersState::SHIFT, e)),
            "A"
        );
        assert_eq!(
            s(encode(&ch("a", None), ModifiersState::CONTROL, e)),
            "\x1b[97;5u"
        );
        assert_eq!(
            s(encode(&ch("a", Some("a")), ModifiersState::ALT, e)),
            "\x1b[97;3u"
        );
        assert_eq!(
            s(encode(
                &ch("I", None),
                ModifiersState::CONTROL | ModifiersState::SHIFT,
                e
            )),
            "\x1b[105;6u"
        );
        // Esc is unambiguous
        assert_eq!(s(encode(&named(NamedKey::Escape), NONE, e)), "\x1b[27u");
    }

    #[test]
    fn legacy_bytes_survive_for_enter_tab_backspace() {
        let e = enc(DISAMBIGUATE);
        assert_eq!(s(encode(&named(NamedKey::Enter), NONE, e)), "\r");
        assert_eq!(s(encode(&named(NamedKey::Tab), NONE, e)), "\t");
        assert_eq!(s(encode(&named(NamedKey::Backspace), NONE, e)), "\x7f");
        assert_eq!(
            s(encode(&named(NamedKey::Enter), ModifiersState::SHIFT, e)),
            "\x1b[13;2u"
        );
        assert_eq!(
            s(encode(&named(NamedKey::Tab), ModifiersState::CONTROL, e)),
            "\x1b[9;5u"
        );
    }

    #[test]
    fn functional_keys() {
        let e = enc(DISAMBIGUATE);
        assert_eq!(s(encode(&named(NamedKey::ArrowUp), NONE, e)), "\x1b[A");
        let app = KeyEncoding {
            app_cursor: true,
            ..e
        };
        assert_eq!(s(encode(&named(NamedKey::ArrowUp), NONE, app)), "\x1bOA");
        assert_eq!(
            s(encode(
                &named(NamedKey::ArrowLeft),
                ModifiersState::CONTROL,
                e
            )),
            "\x1b[1;5D"
        );
        assert_eq!(
            s(encode(&named(NamedKey::F3), ModifiersState::SHIFT, e)),
            "\x1b[13;2~"
        );
        assert_eq!(s(encode(&named(NamedKey::F13), NONE, e)), "\x1b[57376u");
        assert_eq!(
            s(encode(&named(NamedKey::PageDown), ModifiersState::ALT, e)),
            "\x1b[6;3~"
        );
    }

    #[test]
    fn event_types() {
        let e = enc(DISAMBIGUATE | EVENT_TYPES);
        let mut a = ch("a", Some("a"));
        assert_eq!(s(encode(&a, NONE, e)), "a");
        a.repeat = true;
        // repeats of plain text stay text
        assert_eq!(s(encode(&a, NONE, e)), "a");
        a.repeat = false;
        a.released = true;
        a.text = None;
        assert_eq!(s(encode(&a, NONE, e)), "\x1b[97;1:3u");
        let mut up = named(NamedKey::ArrowUp);
        up.repeat = true;
        assert_eq!(s(encode(&up, NONE, e)), "\x1b[1;1:2A");
        up.repeat = false;
        up.released = true;
        assert_eq!(s(encode(&up, NONE, e)), "\x1b[1;1:3A");
        // Enter release needs "all keys"
        let mut enter = named(NamedKey::Enter);
        enter.released = true;
        assert!(encode(&enter, NONE, e).is_none());
        // no release without the flag
        assert!(encode(&up, NONE, enc(DISAMBIGUATE)).is_none());
    }

    #[test]
    fn all_keys_and_text() {
        let e = enc(DISAMBIGUATE | ALL_KEYS);
        assert_eq!(s(encode(&ch("a", Some("a")), NONE, e)), "\x1b[97u");
        assert_eq!(s(encode(&named(NamedKey::Enter), NONE, e)), "\x1b[13u");
        assert_eq!(
            s(encode(&named(NamedKey::ArrowUp), NONE, e)),
            "\x1b[A",
            "unmodified arrows keep the short CSI form"
        );
        let mut shift = named(NamedKey::Shift);
        shift.location = KeyLocation::Left;
        assert_eq!(s(encode(&shift, ModifiersState::SHIFT, e)), "\x1b[57441;2u");
        let t = enc(DISAMBIGUATE | ALL_KEYS | ASSOCIATED_TEXT | ALTERNATE_KEYS);
        assert_eq!(
            s(encode(&ch("A", Some("A")), ModifiersState::SHIFT, t)),
            "\x1b[97:65;2;65u"
        );
        // modifier keys alone are silent without "all keys"
        assert!(encode(&shift, ModifiersState::SHIFT, enc(DISAMBIGUATE)).is_none());
    }

    #[test]
    fn keypad_keys() {
        let e = enc(DISAMBIGUATE);
        let mut one = ch("1", Some("1"));
        one.location = KeyLocation::Numpad;
        assert_eq!(s(encode(&one, NONE, e)), "\x1b[57400u");
        let mut enter = named(NamedKey::Enter);
        enter.location = KeyLocation::Numpad;
        assert_eq!(s(encode(&enter, NONE, e)), "\x1b[57414u");
    }

    #[test]
    fn composed_option_characters() {
        let e = KeyEncoding {
            option_as_meta: false,
            ..enc(DISAMBIGUATE)
        };
        let press = KeyPress::new(
            Key::Character("∫".into()),
            Some("∫".into()),
            Some(Key::Character("b".into())),
        );
        let bytes = s(encode(&press, ModifiersState::ALT, e));
        if cfg!(target_os = "macos") {
            assert_eq!(bytes, "∫");
        } else {
            assert_eq!(bytes, "\x1b[98;3u");
        }
    }
}
