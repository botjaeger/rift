//! xterm mouse reporting: the byte sequences sent to apps that enable
//! mouse tracking (DECSET 1000/1002/1003, SGR 1006, UTF-8 1005).

use winit::keyboard::ModifiersState;

/// Button codes of the xterm protocol.
pub const LEFT: u8 = 0;
pub const MIDDLE: u8 = 1;
pub const RIGHT: u8 = 2;
/// Motion with no button held (and the legacy release code).
pub const NONE: u8 = 3;
pub const WHEEL_UP: u8 = 64;
pub const WHEEL_DOWN: u8 = 65;

/// Encoding flags taken from the terminal mode.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Encoding {
    pub sgr: bool,
    pub utf8: bool,
}

/// One mouse report for `button` at 0-based cell (`col`, `row`).
/// `pressed` is false for a release; `motion` marks a drag/move report.
/// Returns `None` when the legacy encoding cannot express the position.
pub fn report(
    button: u8,
    pressed: bool,
    motion: bool,
    mods: ModifiersState,
    col: usize,
    row: usize,
    enc: Encoding,
) -> Option<Vec<u8>> {
    let mut code = button;
    if mods.shift_key() {
        code += 4;
    }
    if mods.alt_key() {
        code += 8;
    }
    if mods.control_key() {
        code += 16;
    }
    if motion {
        code += 32;
    }
    if enc.sgr {
        let end = if pressed { 'M' } else { 'm' };
        return Some(format!("\x1b[<{code};{};{}{end}", col + 1, row + 1).into_bytes());
    }
    // legacy X10/normal: a release is reported as button 3
    if !pressed {
        code = (code & !0b11) | NONE;
    }
    let mut out = b"\x1b[M".to_vec();
    out.push(32 + code);
    for v in [col, row] {
        let v = 32 + 1 + v;
        if enc.utf8 {
            let c = char::from_u32(v as u32).filter(|_| v < 2048)?;
            let mut tmp = [0u8; 4];
            out.extend_from_slice(c.encode_utf8(&mut tmp).as_bytes());
        } else {
            out.push(u8::try_from(v).ok()?);
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SGR: Encoding = Encoding {
        sgr: true,
        utf8: false,
    };
    const LEGACY: Encoding = Encoding {
        sgr: false,
        utf8: false,
    };
    const E: ModifiersState = ModifiersState::empty();

    #[test]
    fn sgr_press_release_and_drag() {
        assert_eq!(
            report(LEFT, true, false, E, 0, 0, SGR).unwrap(),
            b"\x1b[<0;1;1M"
        );
        assert_eq!(
            report(LEFT, false, false, E, 9, 4, SGR).unwrap(),
            b"\x1b[<0;10;5m"
        );
        assert_eq!(
            report(LEFT, true, true, E, 2, 3, SGR).unwrap(),
            b"\x1b[<32;3;4M"
        );
        assert_eq!(
            report(RIGHT, true, false, E, 0, 0, SGR).unwrap(),
            b"\x1b[<2;1;1M"
        );
        assert_eq!(
            report(WHEEL_UP, true, false, E, 4, 1, SGR).unwrap(),
            b"\x1b[<64;5;2M"
        );
        assert_eq!(
            report(LEFT, true, false, ModifiersState::CONTROL, 0, 0, SGR).unwrap(),
            b"\x1b[<16;1;1M"
        );
    }

    #[test]
    fn legacy_encoding() {
        // press left at (0,0): ESC [ M, 32+0, 33, 33
        assert_eq!(
            report(LEFT, true, false, E, 0, 0, LEGACY).unwrap(),
            b"\x1b[M !!"
        );
        // release is button 3
        assert_eq!(
            report(LEFT, false, false, E, 0, 0, LEGACY).unwrap(),
            b"\x1b[M#!!"
        );
        // wheel down keeps its code
        assert_eq!(
            report(WHEEL_DOWN, true, false, E, 0, 0, LEGACY).unwrap(),
            [0x1b, b'[', b'M', 32 + 65, 33, 33]
        );
        // beyond column 222 the legacy form cannot encode the position
        assert!(report(LEFT, true, false, E, 300, 0, LEGACY).is_none());
        let utf8 = Encoding {
            sgr: false,
            utf8: true,
        };
        let r = report(LEFT, true, false, E, 300, 0, utf8).unwrap();
        assert_eq!(&r[..4], b"\x1b[M ");
        assert_eq!(std::str::from_utf8(&r[4..]).unwrap().chars().count(), 2);
    }
}
