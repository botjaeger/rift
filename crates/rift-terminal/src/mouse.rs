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

/// Most wheel reports or arrow keys one wheel event (or one phone `scroll`)
/// sends a program.
pub const MAX_WHEEL_EVENTS: u32 = 20;

/// What a wheel of `lines` over a pane does (positive: up): the one rule the
/// desktop's mouse wheel and a paired phone's `scroll` share.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wheel {
    /// `count` wheel reports of `button` to a program that tracks the mouse.
    Report { button: u8, count: u32 },
    /// `count` times the arrow key `seq`: a full-screen program without
    /// mouse tracking under alternate scroll (DECSET 1007: less, man).
    Arrows { seq: &'static [u8], count: u32 },
    /// The pane's own view moves this many lines (positive: up).
    View(i32),
    /// Nothing: the alternate screen without either.
    Nothing,
}

impl Wheel {
    /// It sends the program input (which a view-only phone may not).
    pub fn reaches_program(self) -> bool {
        matches!(self, Wheel::Report { .. } | Wheel::Arrows { .. })
    }
}

/// [`Wheel`] for `lines` (not 0) over a pane: `report` when its program
/// tracks the mouse and reporting applies (mouse reporting on, Shift not
/// held, a cell to report at), then by its alternate screen, alternate
/// scroll (DECSET 1007) and application cursor (DECCKM) modes.
pub fn wheel(
    lines: i32,
    report: bool,
    alt_screen: bool,
    alternate_scroll: bool,
    app_cursor: bool,
) -> Wheel {
    let count = lines.unsigned_abs().min(MAX_WHEEL_EVENTS);
    if report {
        let button = if lines > 0 { WHEEL_UP } else { WHEEL_DOWN };
        return Wheel::Report { button, count };
    }
    if !alt_screen {
        return Wheel::View(lines);
    }
    if !alternate_scroll {
        return Wheel::Nothing;
    }
    let seq: &'static [u8] = match (lines > 0, app_cursor) {
        (true, false) => b"\x1b[A",
        (true, true) => b"\x1bOA",
        (false, false) => b"\x1b[B",
        (false, true) => b"\x1bOB",
    };
    Wheel::Arrows { seq, count }
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

    #[test]
    fn the_wheel_reports_to_a_tracking_program_else_scrolls_or_sends_arrows() {
        // mouse tracking wins on either screen, capped per event
        assert_eq!(
            wheel(3, true, true, true, false),
            Wheel::Report {
                button: WHEEL_UP,
                count: 3
            }
        );
        assert_eq!(
            wheel(-50, true, false, false, false),
            Wheel::Report {
                button: WHEEL_DOWN,
                count: MAX_WHEEL_EVENTS
            }
        );
        // the primary screen: Rift's own view, every line
        assert_eq!(wheel(-50, false, false, true, true), Wheel::View(-50));
        assert_eq!(wheel(7, false, false, false, false), Wheel::View(7));
        // alternate scroll: arrows in the cursor mode the program set
        assert_eq!(
            wheel(2, false, true, true, false),
            Wheel::Arrows {
                seq: b"\x1b[A",
                count: 2
            }
        );
        assert_eq!(
            wheel(-1, false, true, true, true),
            Wheel::Arrows {
                seq: b"\x1bOB",
                count: 1
            }
        );
        // the alternate screen without either does nothing
        assert_eq!(wheel(4, false, true, false, false), Wheel::Nothing);
        // only reports and arrows are input to the program
        assert!(wheel(1, true, false, false, false).reaches_program());
        assert!(wheel(1, false, true, true, false).reaches_program());
        assert!(!wheel(1, false, false, false, false).reaches_program());
        assert!(!wheel(1, false, true, false, false).reaches_program());
    }
}
