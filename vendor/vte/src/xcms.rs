//! Xlib Xcms colour-spec parsing, ported to Rust.
//!
//! Reproduces what `XParseColor` does for the Xcms prefixes (`rgbi:`,
//! `CIEXYZ:`, `CIEuvY:`, `CIExyY:`, `CIELab:`, `CIELuv:`, `TekHVC:`) under
//! Xlib's default colour conversion context, so that OSC 4/10/11 colour
//! reports match xterm bit for bit (to the 8 bits a terminal reports).
//!
//! The defaults being reproduced:
//!
//! - Screen characterisation: the built-in `LINEAR_RGB` data of `LRGB.c`
//!   (matrices and per-channel intensity tables), 8 bits per RGB. The screen
//!   white point is the row sums of the RGB-to-XYZ matrix with `Y` forced to 1.
//! - The client white point is undefined, so it equals the screen white point
//!   and no white point adjustment runs.
//! - Out-of-gamut colours go through `XcmsTekHVCClipC` (chroma clipping).
//!
//! Xlib's own trigonometry, square root and cube root (`cmsTrig.c`,
//! `cmsMath.c`) are ported instead of calling libm: their iteration and
//! polynomial approximations differ from libm in the last bits, which can move
//! an 8-bit result. Quirks of the C code (for example CIELab's `Y = L / 9.03292`
//! branch) are kept on purpose.
//!
//! Not supported: hexadecimal floating point and `inf`/`nan` in the numeric
//! fields (Xlib reads them through `sscanf`).

#![allow(clippy::manual_range_contains, clippy::excessive_precision)]

extern crate alloc;

use alloc::vec::Vec;
use core::f64::consts::{FRAC_PI_2, FRAC_PI_4, PI, TAU};

use crate::ansi::Rgb;

/// `XMY_DBL_EPSILON` of the colour-space files (`float.h` is not included there).
const XMY_EPS: f64 = 0.00001;
/// `EPS` of LRGB.c, HVC.c and HVCMxC.c.
const EPS: f64 = 0.001;
/// `EPS` of xyY.c.
const XYY_EPS: f64 = 0.00001;
/// `DBL_EPSILON` as seen by cmsMath.c (`float.h` included).
const DBL_EPSILON: f64 = f64::EPSILON;
const CHROMA_SCALE_FACTOR: f64 = 7.50725;
const U_BR: f64 = 0.7127;
const V_BR: f64 = 0.4931;
const BITS_PER_RGB: u32 = 8;
/// `MASK[8]` of LRGB.c.
const MASK: u16 = 0xff00;
const MAXBISECTCOUNT: usize = 100;
const START_V: f64 = 40.0;
const START_C: f64 = 120.0;
/// Bound for the angle-normalising loops, which never end in C for huge input.
const WRAP_LIMIT: u32 = 1_000_000;

static XYZ_TO_RGB: [[f64; 3]; 3] = [
    [3.48340481253539000, -1.52176374927285200, -0.55923133354049780],
    [-1.07152751306193600, 1.96593795204372400, 0.03673691339553462],
    [0.06351179790497788, -0.20020501000496480, 0.81070942031648220],
];

static RGB_TO_XYZ: [[f64; 3]; 3] = [
    [0.38106149108714790, 0.32025712365352110, 0.24834578525933100],
    [0.20729745115140850, 0.68054638776373240, 0.11215616108485920],
    [0.02133944350088028, 0.14297193020246480, 1.24172892629665500],
];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Fmt {
    Xyz,
    UvY,
    XyY,
    Lab,
    Luv,
    Hvc,
    Rgbi,
}

/// `XcmsColor`: a format tag and the three doubles of the `spec` union, which
/// the C code reuses across formats (HVC's `H,V,C` are RGBi's `r,g,b`).
#[derive(Clone, Copy)]
struct Color {
    fmt: Fmt,
    s: [f64; 3],
}

type Res = Result<(), ()>;

// ---------------------------------------------------------------------------
// cmsMath.c / cmsTrig.c
// ---------------------------------------------------------------------------

fn fabs(x: f64) -> f64 {
    if x < 0.0 {
        -x
    } else {
        x
    }
}

fn xcms_cube_root(a: f64) -> f64 {
    if a == 0. {
        return 0.;
    }
    let abs_a = if a < 0. { -a } else { a };
    let mut cur_guess = if abs_a > 1. { abs_a / 8. } else { abs_a * 8. };
    loop {
        let mut delta = (cur_guess - abs_a / (cur_guess * cur_guess)) / 3.;
        cur_guess -= delta;
        if delta < 0. {
            delta = -delta;
        }
        if delta < cur_guess * DBL_EPSILON || delta.is_nan() {
            break;
        }
    }
    if a < 0. {
        cur_guess = -cur_guess;
    }
    cur_guess
}

fn xcms_square_root(a: f64) -> f64 {
    if a == 0. {
        return 0.;
    }
    if a < 0. {
        return 0.;
    }
    let mut cur_guess = if a > 1. { a / 4. } else { a * 4. };
    loop {
        let mut delta = (cur_guess - a / cur_guess) / 2.;
        cur_guess -= delta;
        if delta < 0. {
            delta = -delta;
        }
        if delta < cur_guess * DBL_EPSILON || delta.is_nan() {
            break;
        }
    }
    cur_guess
}

const XCMS_X6_UNDERFLOWS: f64 = 4.209340e-52;
/// `(double)0x7FFFFFFF * (1L << 22)`.
const XCMS_DMAXPOWTWO: f64 = 2147483647.0 * 4194304.0;

static COS_P: [f64; 4] = [
    0.12905394659037374438e7,
    -0.37456703915723204710e6,
    0.13432300986539084285e5,
    -0.11231450823340933092e3,
];
static COS_Q: [f64; 4] =
    [0.12905394659037373590e7, 0.23467773107245835052e5, 0.20969518196726306286e3, 1.0];
static SIN_P: [f64; 4] = [
    0.20664343336995858240e7,
    -0.18160398797407332550e6,
    0.35999306949636188317e4,
    -0.20107483294588615719e2,
];
static SIN_Q: [f64; 4] =
    [0.26310659102647698963e7, 0.39270242774649000308e5, 0.27811919481083844087e3, 1.0];

fn xcms_polynomial(coeffs: &[f64; 4], x: f64) -> f64 {
    let mut rtn = coeffs[3];
    for i in (0..3).rev() {
        rtn = coeffs[i] + (x * rtn);
    }
    rtn
}

fn xcms_modulo_f(val: f64) -> f64 {
    let abs = fabs(val);
    let ip = if abs >= XCMS_DMAXPOWTWO {
        val
    } else {
        let mut ip = abs + XCMS_DMAXPOWTWO;
        ip -= XCMS_DMAXPOWTWO;
        if ip > abs {
            ip -= 1.0;
        }
        fabs(ip)
    };
    val - ip
}

fn xcms_modulo(mut value: f64, base: f64) -> f64 {
    value /= base;
    value = xcms_modulo_f(value);
    value *= base;
    value
}

fn reduce_pi(mut x: f64) -> f64 {
    if x < -PI || x > PI {
        x = xcms_modulo(x, TAU);
        if x > PI {
            x -= TAU;
        } else if x < -PI {
            x += TAU;
        }
    }
    x
}

fn xcms_cos(x: f64) -> f64 {
    let x = reduce_pi(x);
    if x > FRAC_PI_2 {
        -xcms_cos(x - PI)
    } else if x < -FRAC_PI_2 {
        -xcms_cos(x + PI)
    } else if x > FRAC_PI_4 {
        xcms_sin(FRAC_PI_2 - x)
    } else if x < -FRAC_PI_4 {
        xcms_sin(FRAC_PI_2 + x)
    } else if x < XCMS_X6_UNDERFLOWS && x > -XCMS_X6_UNDERFLOWS {
        xcms_square_root(1.0 - (x * x))
    } else {
        let y = x / FRAC_PI_4;
        let yt2 = y * y;
        xcms_polynomial(&COS_P, yt2) / xcms_polynomial(&COS_Q, yt2)
    }
}

fn xcms_sin(x: f64) -> f64 {
    let x = reduce_pi(x);
    if x > FRAC_PI_2 {
        -xcms_sin(x - PI)
    } else if x < -FRAC_PI_2 {
        -xcms_sin(x + PI)
    } else if x > FRAC_PI_4 {
        xcms_cos(FRAC_PI_2 - x)
    } else if x < -FRAC_PI_4 {
        -xcms_cos(FRAC_PI_2 + x)
    } else if x < XCMS_X6_UNDERFLOWS && x > -XCMS_X6_UNDERFLOWS {
        x
    } else {
        let y = x / FRAC_PI_4;
        let yt2 = y * y;
        y * (xcms_polynomial(&SIN_P, yt2) / xcms_polynomial(&SIN_Q, yt2))
    }
}

fn xcms_atan(x: f64) -> f64 {
    const MAXERROR: f64 = 0.000001;
    const MAXITER: usize = 10000;
    if x == 0.0 {
        return 0.0;
    }
    let maxerror = if x < 1.0 { x * MAXERROR } else { MAXERROR };
    let mut ai = xcms_square_root(1.0 / (1.0 + (x * x)));
    let mut bi = 1.0;
    let mut a1 = 0.0;
    let mut b1 = 0.0;
    for _ in 0..MAXITER {
        a1 = (ai + bi) / 2.0;
        b1 = xcms_square_root(a1 * bi);
        if a1 == b1 {
            break;
        }
        let d = fabs(a1 - b1);
        if d < maxerror {
            break;
        }
        ai = a1;
        bi = b1;
    }
    let l = if a1 > b1 { b1 } else { a1 };
    a1 = xcms_square_root(1.0 + (x * x));
    x / (a1 * l)
}

fn degrees(r: f64) -> f64 {
    r * 180.0 / PI
}

fn radians(d: f64) -> f64 {
    d * PI / 180.0
}

/// `while (v < lo) v += step; while (v >= hi) v -= step;`, bounded.
fn wrap(mut v: f64, lo: f64, hi: f64, step: f64) -> f64 {
    let mut n = 0;
    while v < lo && n < WRAP_LIMIT {
        v += step;
        n += 1;
    }
    n = 0;
    while v >= hi && n < WRAP_LIMIT {
        v -= step;
        n += 1;
    }
    v
}

fn min2(x: f64, y: f64) -> f64 {
    if x > y {
        y
    } else {
        x
    }
}

fn max2(x: f64, y: f64) -> f64 {
    if x > y {
        x
    } else {
        y
    }
}

fn min3(x: f64, y: f64, z: f64) -> f64 {
    if x > min2(y, z) {
        min2(y, z)
    } else {
        x
    }
}

fn max3(x: f64, y: f64, z: f64) -> f64 {
    if x > max2(y, z) {
        x
    } else {
        max2(y, z)
    }
}

// ---------------------------------------------------------------------------
// LRGB.c
// ---------------------------------------------------------------------------

fn mat_vec(m: &[[f64; 3]; 3], v: &[f64; 3]) -> [f64; 3] {
    let mut out = [0.0; 3];
    for i in 0..3 {
        out[i] = 0.0;
        for j in 0..3 {
            out[i] += m[i][j] * v[j];
        }
    }
    out
}

/// `_XcmsLRGB_InitScrnDefault`: screen white point in CIEXYZ.
fn screen_white() -> [f64; 3] {
    [
        RGB_TO_XYZ[0][0] + RGB_TO_XYZ[0][1] + RGB_TO_XYZ[0][2],
        1.0,
        RGB_TO_XYZ[2][0] + RGB_TO_XYZ[2][1] + RGB_TO_XYZ[2][2],
    ]
}

/// `_XcmsIntensityCmp`.
fn intensity_cmp(key: f64, entry: f64) -> i32 {
    if key < entry {
        -1
    } else if key > entry {
        1
    } else {
        0
    }
}

/// `_XcmsTableSearch` with `_XcmsIntensityCmp` / `_XcmsIntensityInterpolation`:
/// intensity in, 16-bit value out.
fn table_search(intensity: f64, table: &[(u16, f64)]) -> u16 {
    let nel = table.len();
    let mut hi = nel - 1;
    let mut lo = 0;
    let mut mid = 0;
    let mut last = hi;

    if intensity_cmp(intensity, table[lo].1) <= 0 {
        return table[lo].0 & MASK;
    }
    while mid != last {
        last = mid;
        mid = lo + (hi - lo) / 2;
        let result = intensity_cmp(intensity, table[mid].1);
        if result == 0 {
            return table[mid].0 & MASK;
        } else if result < 0 {
            hi = mid;
        } else {
            lo = mid;
        }
    }

    // _XcmsIntensityInterpolation
    let shift = 16 - BITS_PER_RGB;
    let max_color: i64 = (1 << BITS_PER_RGB) - 1;
    let (lo_v, lo_i) = (table[lo].0, table[lo].1);
    let (hi_v, hi_i) = (table[hi].0, table[hi].1);
    let ratio = (intensity - lo_i) / (hi_i - lo_i);
    let mut target = i64::from(hi_v) - i64::from(lo_v);
    target = (target as f64 * ratio) as i64;
    target += i64::from(lo_v);

    let mut up = ((target >> shift) * 0xFFFF) / max_color;
    let down;
    if up < target {
        down = up;
        up = (core::cmp::min((down >> shift) + 1, max_color) * 0xFFFF) / max_color;
    } else {
        down = (core::cmp::max((up >> shift) - 1, 0) * 0xFFFF) / max_color;
    }
    let value = if (up - target) < (target - down) { up } else { down };
    (value as u16) & MASK
}

/// Default intensity tables of `LRGB.c` (Tektronix 19" Sony CRT): 16-bit value to intensity.
#[rustfmt::skip]
static RED_TABLE: [(u16, f64); 52] = [
    (0x0000, 0.000000),
    (0x0909, 0.000000),
    (0x0a0a, 0.000936),
    (0x0f0f, 0.001481),
    (0x1414, 0.002329),
    (0x1919, 0.003529),
    (0x1e1e, 0.005127),
    (0x2323, 0.007169),
    (0x2828, 0.009699),
    (0x2d2d, 0.012759),
    (0x3232, 0.016392),
    (0x3737, 0.020637),
    (0x3c3c, 0.025533),
    (0x4141, 0.031119),
    (0x4646, 0.037431),
    (0x4b4b, 0.044504),
    (0x5050, 0.052373),
    (0x5555, 0.061069),
    (0x5a5a, 0.070624),
    (0x5f5f, 0.081070),
    (0x6464, 0.092433),
    (0x6969, 0.104744),
    (0x6e6e, 0.118026),
    (0x7373, 0.132307),
    (0x7878, 0.147610),
    (0x7d7d, 0.163958),
    (0x8282, 0.181371),
    (0x8787, 0.199871),
    (0x8c8c, 0.219475),
    (0x9191, 0.240202),
    (0x9696, 0.262069),
    (0x9b9b, 0.285089),
    (0xa0a0, 0.309278),
    (0xa5a5, 0.334647),
    (0xaaaa, 0.361208),
    (0xafaf, 0.388971),
    (0xb4b4, 0.417945),
    (0xb9b9, 0.448138),
    (0xbebe, 0.479555),
    (0xc3c3, 0.512202),
    (0xc8c8, 0.546082),
    (0xcdcd, 0.581199),
    (0xd2d2, 0.617552),
    (0xd7d7, 0.655144),
    (0xdcdc, 0.693971),
    (0xe1e1, 0.734031),
    (0xe6e6, 0.775322),
    (0xebeb, 0.817837),
    (0xf0f0, 0.861571),
    (0xf5f5, 0.906515),
    (0xfafa, 0.952662),
    (0xffff, 1.000000),
];

#[rustfmt::skip]
static GREEN_TABLE: [(u16, f64); 50] = [
    (0x0000, 0.000000),
    (0x1313, 0.000000),
    (0x1414, 0.000832),
    (0x1919, 0.001998),
    (0x1e1e, 0.003612),
    (0x2323, 0.005736),
    (0x2828, 0.008428),
    (0x2d2d, 0.011745),
    (0x3232, 0.015740),
    (0x3737, 0.020463),
    (0x3c3c, 0.025960),
    (0x4141, 0.032275),
    (0x4646, 0.039449),
    (0x4b4b, 0.047519),
    (0x5050, 0.056520),
    (0x5555, 0.066484),
    (0x5a5a, 0.077439),
    (0x5f5f, 0.089409),
    (0x6464, 0.102418),
    (0x6969, 0.116485),
    (0x6e6e, 0.131625),
    (0x7373, 0.147853),
    (0x7878, 0.165176),
    (0x7d7d, 0.183604),
    (0x8282, 0.203140),
    (0x8787, 0.223783),
    (0x8c8c, 0.245533),
    (0x9191, 0.268384),
    (0x9696, 0.292327),
    (0x9b9b, 0.317351),
    (0xa0a0, 0.343441),
    (0xa5a5, 0.370580),
    (0xaaaa, 0.398747),
    (0xafaf, 0.427919),
    (0xb4b4, 0.458068),
    (0xb9b9, 0.489165),
    (0xbebe, 0.521176),
    (0xc3c3, 0.554067),
    (0xc8c8, 0.587797),
    (0xcdcd, 0.622324),
    (0xd2d2, 0.657604),
    (0xd7d7, 0.693588),
    (0xdcdc, 0.730225),
    (0xe1e1, 0.767459),
    (0xe6e6, 0.805235),
    (0xebeb, 0.843491),
    (0xf0f0, 0.882164),
    (0xf5f5, 0.921187),
    (0xfafa, 0.960490),
    (0xffff, 1.000000),
];

#[rustfmt::skip]
static BLUE_TABLE: [(u16, f64); 51] = [
    (0x0000, 0.000000),
    (0x0e0e, 0.000000),
    (0x0f0f, 0.001341),
    (0x1414, 0.002080),
    (0x1919, 0.003188),
    (0x1e1e, 0.004729),
    (0x2323, 0.006766),
    (0x2828, 0.009357),
    (0x2d2d, 0.012559),
    (0x3232, 0.016424),
    (0x3737, 0.021004),
    (0x3c3c, 0.026344),
    (0x4141, 0.032489),
    (0x4646, 0.039481),
    (0x4b4b, 0.047357),
    (0x5050, 0.056154),
    (0x5555, 0.065903),
    (0x5a5a, 0.076634),
    (0x5f5f, 0.088373),
    (0x6464, 0.101145),
    (0x6969, 0.114968),
    (0x6e6e, 0.129862),
    (0x7373, 0.145841),
    (0x7878, 0.162915),
    (0x7d7d, 0.181095),
    (0x8282, 0.200386),
    (0x8787, 0.220791),
    (0x8c8c, 0.242309),
    (0x9191, 0.264937),
    (0x9696, 0.288670),
    (0x9b9b, 0.313499),
    (0xa0a0, 0.339410),
    (0xa5a5, 0.366390),
    (0xaaaa, 0.394421),
    (0xafaf, 0.423481),
    (0xb4b4, 0.453547),
    (0xb9b9, 0.484592),
    (0xbebe, 0.516587),
    (0xc3c3, 0.549498),
    (0xc8c8, 0.583291),
    (0xcdcd, 0.617925),
    (0xd2d2, 0.653361),
    (0xd7d7, 0.689553),
    (0xdcdc, 0.726454),
    (0xe1e1, 0.764013),
    (0xe6e6, 0.802178),
    (0xebeb, 0.840891),
    (0xf0f0, 0.880093),
    (0xf5f5, 0.919723),
    (0xfafa, 0.959715),
    (0xffff, 1.00000),
];

/// `XcmsRGBiToRGB`.
fn rgbi_to_rgb(c: &Color) -> Option<[u16; 3]> {
    if c.fmt != Fmt::Rgbi {
        return None;
    }
    Some([
        table_search(c.s[0], &RED_TABLE),
        table_search(c.s[1], &GREEN_TABLE),
        table_search(c.s[2], &BLUE_TABLE),
    ])
}

/// `XcmsRGBiToCIEXYZ`.
fn rgbi_to_xyz(c: &mut Color) {
    c.s = mat_vec(&RGB_TO_XYZ, &c.s);
    c.fmt = Fmt::Xyz;
}

fn out_of_gamut(t: &[f64; 3]) -> bool {
    min3(t[0], t[1], t[2]) < -EPS || max3(t[0], t[1], t[2]) > (1.0 + EPS)
}

/// `XcmsCIEXYZToRGBi`. With `compress` false (a context without gamut
/// compression) an out-of-gamut colour comes back as an `Err` with the raw,
/// unclamped RGBi in `c`.
fn xyz_to_rgbi(c: &mut Color, compress: bool) -> Res {
    if c.fmt != Fmt::Xyz {
        return Err(());
    }
    let mut tmp = mat_vec(&XYZ_TO_RGB, &c.s);
    if out_of_gamut(&tmp) {
        if !compress {
            c.s = tmp;
            c.fmt = Fmt::Rgbi;
            return Err(());
        }
        tek_hvc_clip_c(c)?;
        if c.fmt != Fmt::Xyz {
            return Err(());
        }
        tmp = mat_vec(&XYZ_TO_RGB, &c.s);
        if out_of_gamut(&tmp) {
            return Err(());
        }
    }
    c.s = tmp;
    for v in c.s.iter_mut() {
        *v = v.clamp(0.0, 1.0);
    }
    c.fmt = Fmt::Rgbi;
    Ok(())
}

// ---------------------------------------------------------------------------
// XYZ.c, uvY.c, xyY.c, Lab.c, Luv.c
// ---------------------------------------------------------------------------

fn xyz_valid(c: &Color) -> Res {
    if c.fmt != Fmt::Xyz || c.s[1] < 0.0 - XMY_EPS || c.s[1] > 1.0 + XMY_EPS {
        return Err(());
    }
    Ok(())
}

fn uvy_valid(c: &Color) -> Res {
    if c.fmt != Fmt::UvY || c.s[2] < 0.0 - XMY_EPS || c.s[2] > 1.0 + XMY_EPS {
        return Err(());
    }
    Ok(())
}

fn xyy_valid(c: &Color) -> Res {
    let [x, y, yy] = c.s;
    if c.fmt != Fmt::XyY
        || x < 0.0 - XMY_EPS
        || x > 1.0 + XMY_EPS
        || y < 0.0 - XMY_EPS
        || y > 1.0 + XMY_EPS
        || yy < 0.0 - XMY_EPS
        || yy > 1.0 + XMY_EPS
    {
        return Err(());
    }
    Ok(())
}

fn lab_valid(c: &Color) -> Res {
    if c.fmt != Fmt::Lab || c.s[0] < 0.0 - XMY_EPS || c.s[0] > 100.0 + XMY_EPS {
        return Err(());
    }
    Ok(())
}

fn luv_valid(c: &Color) -> Res {
    if c.fmt != Fmt::Luv || c.s[0] < 0.0 - XMY_EPS || c.s[0] > 100.0 + XMY_EPS {
        return Err(());
    }
    Ok(())
}

/// `XcmsCIEXYZToCIEuvY`. `wp` is the XYZ white point, used only when the
/// divisor is zero.
fn xyz_to_uvy(c: &mut Color, wp: Option<&[f64; 3]>) -> Res {
    xyz_valid(c)?;
    let [x, y, z] = c.s;
    let div = x + (15.0 * y) + (3.0 * z);
    let out = if div == 0.0 {
        let w = white_uvy(wp.ok_or(())?)?;
        [w[0], w[1], y]
    } else {
        [4.0 * x / div, 9.0 * y / div, y]
    };
    c.s = out;
    c.fmt = Fmt::UvY;
    Ok(())
}

/// The white point converted to CIEuvY, checked to have `Y == 1`.
fn white_uvy(wp: &[f64; 3]) -> Result<[f64; 3], ()> {
    let mut w = Color { fmt: Fmt::Xyz, s: *wp };
    xyz_to_uvy(&mut w, None)?;
    if w.s[2] != 1.0 {
        return Err(());
    }
    Ok(w.s)
}

/// `XcmsCIEuvYToCIEXYZ`.
fn uvy_to_xyz(c: &mut Color, wp: &[f64; 3]) -> Res {
    uvy_valid(c)?;
    let [u, v, big_y] = c.s;
    let mut div = (6.0 * u) - (16.0 * v) + 12.0;
    let (x, y);
    if div == 0.0 {
        let w = white_uvy(wp)?;
        div = (6.0 * w[0]) - (16.0 * w[1]) + 12.0;
        if div == 0.0 {
            return Err(());
        }
        x = 9.0 * w[0] / div;
        y = 4.0 * w[1] / div;
    } else {
        x = 9.0 * u / div;
        y = 4.0 * v / div;
    }
    let z = 1.0 - x - y;
    let out_x = if y == 0.0 { x } else { x * big_y / y };
    let out_z = if y == 0.0 { z } else { z * big_y / y };
    c.s = [out_x, big_y, out_z];
    c.fmt = Fmt::Xyz;
    Ok(())
}

/// `XcmsCIExyYToCIEXYZ`. Xlib reads an uninitialised copy of the white point
/// in the (practically unreachable) zero-divisor case; zeros are used here.
fn xyy_to_xyz(c: &mut Color, wp: &[f64; 3]) -> Res {
    xyy_valid(c)?;
    let [x0, y0, yy] = c.s;
    let mut div = (-2.0 * x0) + (12.0 * y0) + 3.0;
    let out = if div == 0.0 {
        [0.0, 0.0, 0.0]
    } else {
        if wp[1] != 1.0 {
            return Err(());
        }
        let u = (4.0 * x0) / div;
        let v = (9.0 * y0) / div;
        div = (6.0 * u) - (16.0 * v) + 12.0;
        let (x, mut y);
        if div == 0.0 {
            let (wu, wv) = (0.0, 0.0);
            let mut d = (6.0 * wu) - (16.0 * wv) + 12.0;
            if d == 0.0 {
                d = XYY_EPS;
            }
            x = 9.0 * wu / d;
            y = 4.0 * wu / d;
        } else {
            x = 9.0 * u / div;
            y = 4.0 * v / div;
        }
        let z = 1.0 - x - y;
        if y == 0.0 {
            y = XYY_EPS;
        }
        [x * yy / y, yy, z * yy / y]
    };
    c.s = out;
    c.fmt = Fmt::Xyz;
    Ok(())
}

/// `XcmsCIELabToCIEXYZ`.
fn lab_to_xyz(c: &mut Color, wp: &[f64; 3]) -> Res {
    if wp[1] != 1.0 {
        return Err(());
    }
    lab_valid(c)?;
    let [l, a, b] = c.s;
    let mut tmp_l = (l + 16.0) / 116.0;
    let mut out_y = tmp_l * tmp_l * tmp_l;
    let (out_x, out_z);
    if out_y < 0.008856 {
        tmp_l = l / 9.03292;
        out_x = wp[0] * ((a / 3893.5) + tmp_l);
        out_y = tmp_l;
        out_z = wp[2] * (tmp_l - (b / 1557.4));
    } else {
        let mut t = tmp_l + (a / 5.0);
        out_x = wp[0] * t * t * t;
        t = tmp_l - (b / 2.0);
        out_z = wp[2] * t * t * t;
    }
    c.s = [out_x, out_y, out_z];
    c.fmt = Fmt::Xyz;
    Ok(())
}

/// `XcmsCIELuvToCIEuvY`.
fn luv_to_uvy(c: &mut Color, wp: &[f64; 3]) -> Res {
    let w = white_uvy(wp)?;
    luv_valid(c)?;
    let [l, u, v] = c.s;
    let out_y = if l < 7.99953624 {
        l / 903.29
    } else {
        let t = (l + 16.0) / 116.0;
        t * t * t
    };
    let (out_u, out_v);
    if l == 0.0 {
        out_u = w[0];
        out_v = w[1];
    } else {
        let t = 13.0 * (l / 100.0);
        out_u = u / t + w[0];
        out_v = v / t + w[1];
    }
    c.s = [out_u, out_v, out_y];
    c.fmt = Fmt::UvY;
    Ok(())
}

// ---------------------------------------------------------------------------
// HVC.c, HVCGcC.c, HVCMxC.c, HVCMxVC.c
// ---------------------------------------------------------------------------

fn hvc_valid(c: &mut Color) -> Res {
    if c.fmt != Fmt::Hvc {
        return Err(());
    }
    let [h, mut v, mut ch] = c.s;
    if v < (0.0 - XMY_EPS) || v > (100.0 + XMY_EPS) || ch < 0.0 - XMY_EPS {
        return Err(());
    }
    if v < 0.0 {
        v = 0.0 + XMY_EPS;
    } else if v > 100.0 {
        v = 100.0 - XMY_EPS;
    }
    if ch < 0.0 {
        ch = 0.0 - XMY_EPS;
    }
    c.s = [wrap(h, 0.0, 360.0, 360.0), v, ch];
    Ok(())
}

/// `_XcmsTekHVC_CheckModify`.
fn hvc_check_modify(c: &mut Color) -> Res {
    if c.fmt != Fmt::Hvc {
        return Err(());
    }
    if c.s[1] < 0.0 {
        c.s[1] = 0.0 + XMY_EPS;
    } else if c.s[1] > 100.0 {
        c.s[1] = 100.0 - XMY_EPS;
    }
    if c.s[2] < 0.0 {
        c.s[2] = 0.0 - XMY_EPS;
    }
    if c.s[0] < 0.0 {
        let n = (-c.s[0] / 360.0) as i32;
        c.s[0] += f64::from(n + 1) * 360.0;
        if c.s[0] >= 360.0 {
            c.s[0] -= 360.0;
        }
    } else if c.s[0] >= 360.0 {
        let n = (c.s[0] / 360.0) as i32;
        c.s[0] -= f64::from(n) * 360.0;
    }
    Ok(())
}

/// `ThetaOffset` with the white point in CIEuvY.
fn theta_offset(w: &[f64; 3]) -> Result<f64, ()> {
    let div = U_BR - w[0];
    if div == 0.0 {
        return Err(());
    }
    let slopeuv = (V_BR - w[1]) / div;
    Ok(degrees(xcms_atan(slopeuv)))
}

/// `XcmsTekHVCToCIEuvY`.
fn hvc_to_uvy(c: &mut Color, wp: &[f64; 3]) -> Res {
    let w = white_uvy(wp)?;
    let theta = theta_offset(&w)?;
    hvc_valid(c)?;
    let [h, v, ch] = c.s;
    let out = if v == 0.0 || v == 100.0 {
        let y = if v == 100.0 { 1.0 } else { 0.0 };
        [w[0], w[1], y]
    } else {
        let temp_hue = wrap(h + theta, 0.0, 360.0, 360.0);
        let temp_hue = radians(temp_hue);
        let u = (xcms_cos(temp_hue) * ch) / (v * CHROMA_SCALE_FACTOR);
        let vv = (xcms_sin(temp_hue) * ch) / (v * CHROMA_SCALE_FACTOR);
        let y = if v < 7.99953624 {
            v / 903.29
        } else {
            let t = (v + 16.0) / 116.0;
            t * t * t
        };
        [u + w[0], vv + w[1], y]
    };
    c.s = out;
    c.fmt = Fmt::UvY;
    Ok(())
}

/// `XcmsCIEuvYToTekHVC`.
fn uvy_to_hvc(c: &mut Color, wp: &[f64; 3]) -> Res {
    let w = white_uvy(wp)?;
    let theta_off = theta_offset(&w)?;
    uvy_valid(c)?;
    let u = c.s[0] - w[0];
    let v = c.s[1] - w[1];
    let big_y = c.s[2];
    let mut theta = if u == 0.0 {
        0.0
    } else {
        let t = v / u;
        degrees(xcms_atan(t))
    };
    let (mut n_low, mut n_high) = (0.0, 360.0);
    if u > 0.0 && v > 0.0 {
        n_low = 0.0;
        n_high = 90.0;
    } else if u < 0.0 && v > 0.0 {
        n_low = 90.0;
        n_high = 180.0;
    } else if u < 0.0 && v < 0.0 {
        n_low = 180.0;
        n_high = 270.0;
    } else if u > 0.0 && v < 0.0 {
        n_low = 270.0;
        n_high = 360.0;
    }
    theta = wrap(theta, n_low, n_high, 90.0);
    let l2 = if big_y < 0.008856 { big_y * 903.29 } else { (xcms_cube_root(big_y) * 116.0) - 16.0 };
    let chroma = l2 * CHROMA_SCALE_FACTOR * xcms_square_root((u * u) + (v * v));
    if chroma < 0.0 {
        theta = 0.0;
    }
    let h = wrap(theta - theta_off, -EPS, 360.0 + EPS, 360.0);
    c.s = [h, l2, chroma];
    c.fmt = Fmt::Hvc;
    Ok(())
}

/// Any device-independent format to CIEXYZ, client white point = screen white.
fn di_to_xyz(c: &mut Color, wp: &[f64; 3]) -> Res {
    match c.fmt {
        Fmt::Xyz => Ok(()),
        Fmt::UvY => uvy_to_xyz(c, wp),
        Fmt::XyY => xyy_to_xyz(c, wp),
        Fmt::Lab => lab_to_xyz(c, wp),
        Fmt::Luv => {
            luv_to_uvy(c, wp)?;
            uvy_to_xyz(c, wp)
        },
        Fmt::Hvc => {
            hvc_to_uvy(c, wp)?;
            uvy_to_xyz(c, wp)
        },
        Fmt::Rgbi => Err(()),
    }
}

/// CIEXYZ to TekHVC (`XcmsCIEXYZToCIEuvY`, `XcmsCIEuvYToTekHVC`).
fn xyz_to_hvc(c: &mut Color, wp: &[f64; 3]) -> Res {
    xyz_to_uvy(c, Some(wp))?;
    uvy_to_hvc(c, wp)
}

/// `XcmsTekHVCClipC`: the default gamut compression procedure. Takes and
/// returns a CIEXYZ colour.
fn tek_hvc_clip_c(c: &mut Color) -> Res {
    let wp = screen_white();
    if c.fmt != Fmt::Hvc {
        xyz_to_hvc(c, &wp)?;
    }
    let (hue, value) = (c.s[0], c.s[1]);
    tek_hvc_query_max_c(hue, value, c)?;
    di_to_xyz(c, &wp)
}

/// `_XcmsTekHVCQueryMaxVCRGB` in a context without gamut compression.
fn query_max_vc_rgb(hue: f64, out: &mut Color, rgb_out: &mut [f64; 3]) -> Res {
    let wp = screen_white();
    let mut tmp = Color { fmt: Fmt::Hvc, s: [hue, START_V, START_C] };

    // HVC to RGBi: an unreachable colour, so the out-of-gamut RGBi is stuffed
    // into `tmp` and reported as a failure.
    let r = di_to_xyz(&mut tmp, &wp).and_then(|()| xyz_to_rgbi(&mut tmp, false));
    if r.is_err() && tmp.fmt != Fmt::Rgbi {
        return Err(());
    }

    let n_small = min3(tmp.s[0], tmp.s[1], tmp.s[2]);
    for v in tmp.s.iter_mut() {
        *v -= n_small;
    }
    let n_large = max3(tmp.s[0], tmp.s[1], tmp.s[2]);
    for v in tmp.s.iter_mut() {
        *v /= n_large;
    }
    tmp.fmt = Fmt::Rgbi;
    *rgb_out = tmp.s;

    rgbi_to_xyz(&mut tmp);
    xyz_to_hvc(&mut tmp, &wp)?;
    tmp.s[0] = hue;
    *out = tmp;
    Ok(())
}

/// `XcmsTekHVCQueryMaxC`.
fn tek_hvc_query_max_c(hue: f64, value: f64, out: &mut Color) -> Res {
    let wp = screen_white();
    let mut tmp = Color { fmt: Fmt::Hvc, s: [hue, value, 100.0] };
    hvc_check_modify(&mut tmp)?;

    let mut max_vc = tmp;
    let mut rgb_saved = [0.0; 3];
    query_max_vc_rgb(hue, &mut max_vc, &mut rgb_saved)?;

    if value <= max_vc.s[1] {
        tmp.s[2] = value * max_vc.s[2] / max_vc.s[1];
        hvc_check_modify(&mut tmp)?;
        *out = tmp;
        return Ok(());
    }

    let saved_value = value;
    let mut n_value = value;
    let mut last_value = -1.0;
    let mut last_chroma = -1.0;
    let max_dist = 100.0 - max_vc.s[1];
    let mut r_factor = 1.0;

    for _ in 0..MAXBISECTCOUNT {
        let prev_value = last_value;
        last_value = tmp.s[1];
        last_chroma = tmp.s[2];
        let n_t = (n_value - max_vc.s[1]) / max_dist * r_factor;
        tmp.s[0] = rgb_saved[0] * (1.0 - n_t) + n_t;
        tmp.s[1] = rgb_saved[1] * (1.0 - n_t) + n_t;
        tmp.s[2] = rgb_saved[2] * (1.0 - n_t) + n_t;
        tmp.fmt = Fmt::Rgbi;

        rgbi_to_xyz(&mut tmp);
        xyz_to_hvc(&mut tmp, &wp)?;

        if tmp.s[1] <= saved_value + EPS && tmp.s[1] >= saved_value - EPS {
            tmp.s[0] = hue;
            hvc_check_modify(&mut tmp)?;
            *out = tmp;
            return Ok(());
        }
        n_value += saved_value - tmp.s[1];
        if n_value < max_vc.s[1] {
            n_value = max_vc.s[1];
            r_factor *= 0.5;
        } else if n_value > 100.0 {
            tmp.s[0] = hue;
            if fabs(last_value - saved_value) < fabs(tmp.s[1] - saved_value) {
                tmp.s[1] = last_value;
                tmp.s[2] = last_chroma;
            }
            hvc_check_modify(&mut tmp)?;
            *out = tmp;
            return Ok(());
        } else if tmp.s[1] <= prev_value + EPS && tmp.s[1] >= prev_value - EPS {
            r_factor *= 0.5;
        }
    }
    // The bisection ran out of iterations: keep the closer of the last two.
    if fabs(last_value - saved_value) < fabs(tmp.s[1] - saved_value) {
        tmp.s[1] = last_value;
        tmp.s[2] = last_chroma;
    }
    tmp.s[0] = hue;
    *out = tmp;
    Ok(())
}

// ---------------------------------------------------------------------------
// Parsing (cmsColNm.c `_XcmsParseColorString` and the `*_ParseString`s)
// ---------------------------------------------------------------------------

/// `sscanf("%lf")` for plain decimal floats; `i` is advanced past it.
fn scan_double(s: &[u8], i: &mut usize) -> Option<f64> {
    while *i < s.len() && matches!(s[*i], b' ' | 9..=13) {
        *i += 1;
    }
    let start = *i;
    let mut j = start;
    if j < s.len() && (s[j] == b'+' || s[j] == b'-') {
        j += 1;
    }
    let mut digits = 0;
    while j < s.len() && s[j].is_ascii_digit() {
        j += 1;
        digits += 1;
    }
    if j < s.len() && s[j] == b'.' {
        j += 1;
        while j < s.len() && s[j].is_ascii_digit() {
            j += 1;
            digits += 1;
        }
    }
    if digits == 0 {
        return None;
    }
    if j < s.len() && (s[j] == b'e' || s[j] == b'E') {
        let mut k = j + 1;
        if k < s.len() && (s[k] == b'+' || s[k] == b'-') {
            k += 1;
        }
        if k < s.len() && s[k].is_ascii_digit() {
            while k < s.len() && s[k].is_ascii_digit() {
                k += 1;
            }
            j = k;
        }
    }
    *i = j;
    core::str::from_utf8(&s[start..j]).ok()?.parse::<f64>().ok()
}

/// `sscanf(s, "%lf/%lf/%lf")` returning 3.
fn scan_three(s: &[u8]) -> Option<[f64; 3]> {
    let mut i = 0;
    let a = scan_double(s, &mut i)?;
    if s.get(i) != Some(&b'/') {
        return None;
    }
    i += 1;
    let b = scan_double(s, &mut i)?;
    if s.get(i) != Some(&b'/') {
        return None;
    }
    i += 1;
    let c = scan_double(s, &mut i)?;
    Some([a, b, c])
}

/// The numeric fields, with Xlib's retry that swaps `.` and `,`.
fn parse_fields(values: &[u8]) -> Option<[f64; 3]> {
    if let Some(v) = scan_three(values) {
        return Some(v);
    }
    let swapped: Vec<u8> = values
        .iter()
        .map(|&b| match b {
            b'.' => b',',
            b',' => b'.',
            other => other,
        })
        .collect();
    scan_three(&swapped)
}

/// Parse a colour specification the way `XParseColor` does for the Xcms
/// prefixes and return the 16-bit RGB it reports.
fn parse16(spec: &[u8]) -> Option<[u16; 3]> {
    let lowered: Vec<u8> =
        spec.iter().take_while(|&&b| b != 0).map(u8::to_ascii_lowercase).collect();
    let colon = lowered.iter().position(|&b| b == b':')?;
    let (prefix, values) = (&lowered[..colon], &lowered[colon + 1..]);

    let fmt = match prefix {
        b"rgbi" => Fmt::Rgbi,
        b"ciexyz" => Fmt::Xyz,
        b"cieuvy" => Fmt::UvY,
        b"ciexyy" => Fmt::XyY,
        b"cielab" => Fmt::Lab,
        b"cieluv" => Fmt::Luv,
        b"tekhvc" => Fmt::Hvc,
        _ => return None,
    };
    let mut c = Color { fmt, s: parse_fields(values)? };

    let wp = screen_white();
    match fmt {
        // DD to DD: only the intensity tables.
        Fmt::Rgbi => {},
        _ => {
            let valid = match fmt {
                Fmt::Xyz => xyz_valid(&c),
                Fmt::UvY => uvy_valid(&c),
                Fmt::XyY => xyy_valid(&c),
                Fmt::Lab => lab_valid(&c),
                Fmt::Luv => luv_valid(&c),
                _ => hvc_valid(&mut c),
            };
            valid.ok()?;
            di_to_xyz(&mut c, &wp).ok()?;
            xyz_to_rgbi(&mut c, true).ok()?;
        },
    }
    rgbi_to_rgb(&c)
}

/// Parse an Xcms colour string (`rgbi:`, `CIEXYZ:`, `CIEuvY:`, `CIExyY:`,
/// `CIELab:`, `CIELuv:`, `TekHVC:`; prefix case-insensitive) into the top byte
/// of each channel of the colour `XParseColor` would report.
pub(crate) fn parse(spec: &[u8]) -> Option<Rgb> {
    let [r, g, b] = parse16(spec)?;
    Some(Rgb { r: (r >> 8) as u8, g: (g >> 8) as u8, b: (b >> 8) as u8 })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check(spec: &str, want: (u8, u8, u8)) {
        let got = parse(spec.as_bytes()).map(|c| (c.r, c.g, c.b));
        assert_eq!(got, Some(want), "{spec}: got {got:02x?}, want {want:02x?}");
    }

    #[test]
    fn rgbi() {
        check("rgbi:1/1/1", (0xff, 0xff, 0xff));
        check("rgbi:0.5/0.5/0.5", (0xc1, 0xbb, 0xbb));
    }

    #[test]
    fn ciexyz() {
        check("CIEXYZ:1/1/1", (0xff, 0xff, 0xff));
        check("CIEXYZ:0.5/0.5/0.5", (0xdd, 0xb5, 0xa0));
    }

    #[test]
    fn cieuvy() {
        check("CIEuvY:1/1/1", (0xff, 0xff, 0xff));
        check("CIEuvY:0.5/0.5/0.5", (0xff, 0xa3, 0xae));
    }

    #[test]
    fn ciexyy() {
        check("CIExyY:1/1/1", (0xff, 0xff, 0xff));
        check("CIExyY:0.5/0.5/0.5", (0xf7, 0xb3, 0x0e));
    }

    #[test]
    fn cielab() {
        check("CIELab:1/1/1", (0x6c, 0x67, 0x67));
        check("CIELab:0.5/0.5/0.5", (0x52, 0x4f, 0x4f));
    }

    #[test]
    fn cieluv() {
        check("CIELuv:1/1/1", (0x16, 0x14, 0x0e));
        check("CIELuv:0.5/0.5/0.5", (0x0e, 0x13, 0x0e));
    }

    #[test]
    fn tekhvc() {
        check("TekHVC:1/1/1", (0x1a, 0x13, 0x0f));
        check("TekHVC:0.5/0.5/0.5", (0x11, 0x13, 0x0e));
    }

    #[test]
    fn prefix_is_case_insensitive() {
        check("RGBI:1/1/1", (0xff, 0xff, 0xff));
        check("ciexyz:1/1/1", (0xff, 0xff, 0xff));
    }

    #[test]
    fn rejects_garbage() {
        assert_eq!(parse(b"rgbi:1/1"), None);
        assert_eq!(parse(b"rgbi:a/b/c"), None);
        assert_eq!(parse(b"CIEXYZ:1/2/1"), None);
        assert_eq!(parse(b"CIELab:101/0/0"), None);
        assert_eq!(parse(b"TekHVC:0/101/0"), None);
        assert_eq!(parse(b"nope:1/1/1"), None);
        assert_eq!(parse(b"1/1/1"), None);
    }

    #[test]
    fn huge_hue_terminates() {
        let _ = parse(b"TekHVC:1e300/50/50");
        let _ = parse(b"TekHVC:-1e300/50/50");
    }
}
