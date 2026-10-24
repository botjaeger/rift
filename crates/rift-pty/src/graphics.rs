//! Inline images: the kitty graphics protocol (APC `G`) and iTerm2 inline
//! images (OSC 1337 `File=`), sharing one per-pane image model.
//!
//! <https://sw.kovidgoyal.net/kitty/graphics-protocol/>
//! <https://iterm2.com/documentation-images.html>
//!
//! Images are decoded to RGBA on the reader thread and placed at the cursor,
//! anchored to the absolute line model, so they scroll, survive scrollback
//! trimming (until their line leaves history) and follow reflow like block
//! anchors. Everything is bounded:
//!
//! - [`MAX_PAYLOAD`] bytes of (base64) data per image, including chunks
//! - [`MAX_PIXELS`] decoded pixels per image (decode time and memory)
//! - [`MAX_STORED_BYTES`] of decoded images per pane (oldest evicted)
//! - [`MAX_PLACEMENTS`] placements per pane (oldest dropped)
//!
//! Only direct transmission (`t=d`) is accepted: file, temp-file and shared
//! memory transfers would let a program (possibly over ssh) make the
//! terminal read local files. Animation frames and Unicode placeholders are
//! not implemented, and queries answer accordingly.

use std::collections::HashMap;
use std::sync::Arc;

pub const MAX_PAYLOAD: usize = 32 << 20;
pub const MAX_PIXELS: u64 = 40_000_000;
pub const MAX_STORED_BYTES: usize = 320 << 20;
pub const MAX_PLACEMENTS: usize = 512;
pub const MAX_CELL_SPAN: usize = 1000;

/// A decoded image. `key` is unique for the process (renderer cache key).
#[derive(Debug, Clone)]
pub struct ImageData {
    pub key: u64,
    pub width: u32,
    pub height: u32,
    pub rgba: Arc<Vec<u8>>,
}

/// An image shown in the grid: top-left at (`line`, `col`) spanning
/// `cols` × `rows` cells, showing the `src` pixel rect of the image.
#[derive(Debug, Clone, PartialEq)]
pub struct Placement {
    pub image: u64,
    /// kitty image id / placement id (0 for iTerm2 images).
    pub image_id: u32,
    pub placement_id: u32,
    pub line: u64,
    pub col: usize,
    pub cols: usize,
    pub rows: usize,
    /// Source rect in image pixels: x, y, w, h.
    pub src: [u32; 4],
    /// Negative: drawn under text; otherwise over it.
    pub z: i32,
    /// Placed while the alternate screen was active.
    pub alt: bool,
}

#[derive(Default)]
pub struct Graphics {
    /// kitty image id → image.
    images: HashMap<u32, ImageData>,
    /// Insertion order of `images` ids, for eviction.
    order: Vec<u32>,
    stored_bytes: usize,
    pub placements: Vec<Placement>,
    /// A chunked (`m=1`) transmission in progress.
    pending: Option<(Control, Vec<u8>)>,
    /// Ids for images sent without one (`a=T` with no `i`, iTerm2).
    next_anon_id: u32,
    /// kitty image number (`I=`) -> id of the newest live image sent with it.
    /// Removed with the image, including replacement and budget eviction.
    numbers: HashMap<u32, u32>,
}

fn next_key() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(1);
    N.fetch_add(1, Ordering::Relaxed)
}

/// Parsed kitty control data (`key=value,…`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Control {
    pub action: u8,
    pub format: u32,
    pub medium: u8,
    pub compressed: bool,
    pub width: u32,
    pub height: u32,
    pub id: u32,
    pub number: u32,
    pub placement: u32,
    pub more: bool,
    pub cols: u32,
    pub rows: u32,
    pub src: [u32; 4],
    pub z: i32,
    pub no_cursor_move: bool,
    pub quiet: u8,
    pub delete: u8,
}

impl Control {
    pub fn parse(s: &str) -> Control {
        let mut c = Control {
            action: b't',
            format: 32,
            medium: b'd',
            ..Default::default()
        };
        for kv in s.split(',') {
            let Some((k, v)) = kv.split_once('=') else {
                continue;
            };
            let num = || v.parse::<u32>().unwrap_or(0);
            let ch = || v.bytes().next().unwrap_or(0);
            match k {
                "a" => c.action = ch(),
                "f" => c.format = num(),
                "t" => c.medium = ch(),
                "o" => c.compressed = v == "z",
                "s" => c.width = num(),
                "v" => c.height = num(),
                "i" => c.id = num(),
                "I" => c.number = num(),
                "p" => c.placement = num(),
                "m" => c.more = v == "1",
                "c" => c.cols = num(),
                "r" => c.rows = num(),
                "x" => c.src[0] = num(),
                "y" => c.src[1] = num(),
                "w" => c.src[2] = num(),
                "h" => c.src[3] = num(),
                "z" => c.z = v.parse().unwrap_or(0),
                "C" => c.no_cursor_move = v == "1",
                "q" => c.quiet = num().min(2) as u8,
                "d" => c.delete = ch(),
                _ => {}
            }
        }
        c
    }
}

/// What the terminal must do after a graphics command.
#[derive(Debug, Default, PartialEq)]
pub struct Outcome {
    /// Bytes to write back to the program (kitty responses).
    pub reply: Option<String>,
    /// Place this image at the cursor: (image key, cols, rows, src, z,
    /// kitty ids, move the cursor after it).
    pub place: Option<PlaceRequest>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PlaceRequest {
    pub image: u64,
    pub image_id: u32,
    pub placement_id: u32,
    pub cols: usize,
    pub rows: usize,
    pub src: [u32; 4],
    pub z: i32,
    pub move_cursor: bool,
}

/// Standard base64 (padding optional, whitespace ignored).
pub fn base64_decode(input: &[u8]) -> Option<Vec<u8>> {
    let val = |c: u8| -> Option<u32> {
        Some(match c {
            b'A'..=b'Z' => (c - b'A') as u32,
            b'a'..=b'z' => (c - b'a' + 26) as u32,
            b'0'..=b'9' => (c - b'0' + 52) as u32,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            _ => return None,
        })
    };
    let mut out = Vec::with_capacity(input.len() / 4 * 3);
    let (mut acc, mut bits) = (0u32, 0u32);
    for &c in input {
        if c == b'=' || c.is_ascii_whitespace() {
            continue;
        }
        acc = (acc << 6) | val(c)?;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Some(out)
}

fn inflate(data: &[u8]) -> Option<Vec<u8>> {
    use std::io::Read;
    let mut out = Vec::new();
    flate2::read::ZlibDecoder::new(data)
        .take(MAX_PIXELS * 4 + 1)
        .read_to_end(&mut out)
        .ok()?;
    Some(out)
}

/// Decode PNG/JPEG/GIF/WebP bytes to RGBA within the pixel budget.
pub fn decode_encoded(bytes: &[u8]) -> Result<(u32, u32, Vec<u8>), &'static str> {
    let reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|_| "EBADF:unreadable image")?;
    let (w, h) = reader
        .into_dimensions()
        .map_err(|_| "EBADF:unsupported image format")?;
    if w == 0 || h == 0 || w as u64 * h as u64 > MAX_PIXELS {
        return Err("EFBIG:image too large");
    }
    let img = image::load_from_memory(bytes).map_err(|_| "EBADF:corrupt image data")?;
    let rgba = img.to_rgba8();
    Ok((rgba.width(), rgba.height(), rgba.into_raw()))
}

impl Graphics {
    pub fn image(&self, key: u64) -> Option<&ImageData> {
        self.images.values().find(|i| i.key == key)
    }

    fn store(&mut self, id: u32, width: u32, height: u32, rgba: Vec<u8>) -> u64 {
        let key = next_key();
        if let Some(old) = self.images.remove(&id) {
            self.stored_bytes -= old.rgba.len();
            self.order.retain(|i| *i != id);
            self.numbers.retain(|_, image| *image != id);
        }
        self.stored_bytes += rgba.len();
        self.images.insert(
            id,
            ImageData {
                key,
                width,
                height,
                rgba: Arc::new(rgba),
            },
        );
        self.order.push(id);
        // evict the oldest images (and their placements) past the budget
        while self.stored_bytes > MAX_STORED_BYTES && self.order.len() > 1 {
            let oldest = self.order.remove(0);
            if let Some(img) = self.images.remove(&oldest) {
                self.stored_bytes -= img.rgba.len();
                self.placements.retain(|p| p.image != img.key);
                self.numbers.retain(|_, image| *image != oldest);
            }
        }
        key
    }

    fn anon_id(&mut self) -> u32 {
        // high range so it never collides with ids programs pick
        self.next_anon_id = self.next_anon_id.wrapping_add(1);
        0x8000_0000 | self.next_anon_id
    }

    /// Handle one kitty APC body (the bytes after `G`).
    pub fn kitty(&mut self, body: &[u8], cell_px: (u32, u32)) -> Outcome {
        let (ctrl_s, payload) = match body.iter().position(|b| *b == b';') {
            Some(i) => (&body[..i], &body[i + 1..]),
            None => (body, &[][..]),
        };
        let ctrl = Control::parse(&String::from_utf8_lossy(ctrl_s));

        // continuation chunk of a pending transmission
        if let Some((first, mut data)) = self.pending.take() {
            if data.len() + payload.len() > MAX_PAYLOAD {
                return self.reply(&first, Err("EFBIG:payload too large"));
            }
            data.extend_from_slice(payload);
            if ctrl.more {
                self.pending = Some((first, data));
                return Outcome::default();
            }
            return self.transmit(first, &data, cell_px);
        }

        match ctrl.action {
            b't' | b'T' | b'q' => {
                if payload.len() > MAX_PAYLOAD {
                    return self.reply(&ctrl, Err("EFBIG:payload too large"));
                }
                if ctrl.more {
                    self.pending = Some((ctrl, payload.to_vec()));
                    return Outcome::default();
                }
                self.transmit(ctrl, payload, cell_px)
            }
            b'p' => {
                let Some(img) = self.images.get(&ctrl.id).cloned() else {
                    return self.reply(&ctrl, Err("ENOENT:no such image"));
                };
                let mut out = self.reply(&ctrl, Ok(()));
                out.place = Some(place_request(&ctrl, &img, cell_px));
                out
            }
            b'd' => {
                self.delete(&ctrl);
                Outcome::default()
            }
            _ => self.reply(&ctrl, Err("EINVAL:unsupported action")),
        }
    }

    fn transmit(&mut self, ctrl: Control, payload: &[u8], cell_px: (u32, u32)) -> Outcome {
        if ctrl.medium != b'd' {
            return self.reply(&ctrl, Err("EINVAL:only direct transmission is supported"));
        }
        let decoded = (|| -> Result<(u32, u32, Vec<u8>), &'static str> {
            let raw = base64_decode(payload).ok_or("EINVAL:bad base64")?;
            let raw = if ctrl.compressed {
                inflate(&raw).ok_or("EINVAL:bad zlib data")?
            } else {
                raw
            };
            match ctrl.format {
                100 => decode_encoded(&raw),
                24 | 32 => {
                    let (w, h) = (ctrl.width, ctrl.height);
                    if w == 0 || h == 0 || w as u64 * h as u64 > MAX_PIXELS {
                        return Err("EINVAL:bad dimensions");
                    }
                    let bpp = if ctrl.format == 24 { 3 } else { 4 };
                    if raw.len() < (w * h) as usize * bpp {
                        return Err("ENODATA:insufficient image data");
                    }
                    let rgba = if bpp == 4 {
                        raw[..(w * h * 4) as usize].to_vec()
                    } else {
                        raw.as_chunks::<3>()
                            .0
                            .iter()
                            .take((w * h) as usize)
                            .flat_map(|p| [p[0], p[1], p[2], 255])
                            .collect()
                    };
                    Ok((w, h, rgba))
                }
                _ => Err("EINVAL:unsupported format"),
            }
        })();
        let (w, h, rgba) = match decoded {
            Ok(d) => d,
            Err(e) => return self.reply(&ctrl, Err(e)),
        };
        // a query only checks that the data would load
        if ctrl.action == b'q' {
            return self.reply(&ctrl, Ok(()));
        }
        let id = if ctrl.id == 0 {
            self.anon_id()
        } else {
            ctrl.id
        };
        let key = self.store(id, w, h, rgba);
        if ctrl.number != 0 {
            self.numbers.insert(ctrl.number, id);
        }
        let mut out = self.reply(&ctrl, Ok(()));
        if ctrl.action == b'T' {
            let img = self.images[&id].clone();
            debug_assert_eq!(img.key, key);
            out.place = Some(place_request(&ctrl, &img, cell_px));
        }
        out
    }

    /// `a=d`: the selectors `a`/`A` (everything), `i`/`I` (image id, optional
    /// placement id) and `n`/`N` (newest image with number `I`). Uppercase
    /// also frees the image data. Any other selector (cursor, column, row,
    /// z-index, ...) is not implemented and deletes nothing: wiping every
    /// image for a narrower request would destroy what the program meant to
    /// keep.
    fn delete(&mut self, ctrl: &Control) {
        let free = ctrl.delete.is_ascii_uppercase();
        let (keys, pid): (Vec<u64>, u32) = match ctrl.delete.to_ascii_lowercase() {
            // `d` omitted (0) means `a`
            0 | b'a' => (self.images.values().map(|i| i.key).collect(), 0),
            b'i' => (
                self.images
                    .get(&ctrl.id)
                    .map(|i| vec![i.key])
                    .unwrap_or_default(),
                ctrl.placement,
            ),
            b'n' => (
                self.numbers
                    .get(&ctrl.number)
                    .and_then(|id| self.images.get(id))
                    .map(|i| vec![i.key])
                    .unwrap_or_default(),
                ctrl.placement,
            ),
            _ => return,
        };
        self.placements
            .retain(|p| !(keys.contains(&p.image) && (pid == 0 || p.placement_id == pid)));
        if free {
            let ids: Vec<u32> = self
                .images
                .iter()
                .filter(|(_, i)| keys.contains(&i.key))
                .map(|(id, _)| *id)
                .collect();
            for id in ids {
                if let Some(img) = self.images.remove(&id) {
                    self.stored_bytes -= img.rgba.len();
                }
                self.order.retain(|i| *i != id);
                self.numbers.retain(|_, image| *image != id);
            }
        }
    }

    fn reply(&self, ctrl: &Control, result: Result<(), &str>) -> Outcome {
        let quiet = match result {
            Ok(()) => ctrl.quiet >= 1,
            Err(_) => ctrl.quiet >= 2,
        };
        // only commands that name an image get a response
        if quiet || (ctrl.id == 0 && ctrl.number == 0) {
            return Outcome::default();
        }
        let mut head = if ctrl.number != 0 {
            format!("i={},I={}", ctrl.id, ctrl.number)
        } else {
            format!("i={}", ctrl.id)
        };
        if ctrl.placement != 0 {
            head.push_str(&format!(",p={}", ctrl.placement));
        }
        let msg = match result {
            Ok(()) => "OK",
            Err(e) => e,
        };
        Outcome {
            reply: Some(format!("\x1b_G{head};{msg}\x1b\\")),
            place: None,
        }
    }

    /// Handle an iTerm2 `File=` OSC (the bytes after `1337;File=`).
    pub fn iterm(
        &mut self,
        body: &[u8],
        cell_px: (u32, u32),
        screen_cols: usize,
        screen_rows: usize,
    ) -> Outcome {
        let Some(colon) = body.iter().position(|b| *b == b':') else {
            return Outcome::default();
        };
        let args = String::from_utf8_lossy(&body[..colon]);
        let mut inline = false;
        let (mut want_w, mut want_h) = (String::new(), String::new());
        let mut keep_aspect = true;
        for kv in args.split(';') {
            let Some((k, v)) = kv.split_once('=') else {
                continue;
            };
            match k {
                "inline" => inline = v == "1",
                "width" => want_w = v.to_string(),
                "height" => want_h = v.to_string(),
                "preserveAspectRatio" => keep_aspect = v != "0",
                _ => {}
            }
        }
        // download-to-disk (inline=0) is not supported: nothing is written
        if !inline || body.len() - colon > MAX_PAYLOAD {
            return Outcome::default();
        }
        let Some(raw) = base64_decode(&body[colon + 1..]) else {
            return Outcome::default();
        };
        let Ok((w, h, rgba)) = decode_encoded(&raw) else {
            return Outcome::default();
        };
        let (cw, ch) = (cell_px.0.max(1) as f64, cell_px.1.max(1) as f64);
        let span = |spec: &str, cell: f64, total_cells: usize| -> Result<Option<f64>, ()> {
            if spec.is_empty() || spec == "auto" {
                return Ok(None);
            }
            let (number, scale) = if let Some(p) = spec.strip_suffix("px") {
                (p, 1.0 / cell)
            } else if let Some(p) = spec.strip_suffix('%') {
                (p, total_cells as f64 / 100.0)
            } else {
                (spec, 1.0)
            };
            let value = number.parse::<f64>().map_err(|_| ())? * scale;
            if !value.is_finite() || value <= 0.0 {
                return Err(());
            }
            Ok(Some(value.min(MAX_CELL_SPAN as f64)))
        };
        let natural = (w as f64 / cw, h as f64 / ch);
        let (Ok(mut cols), Ok(mut rows)) = (
            span(&want_w, cw, screen_cols),
            span(&want_h, ch, screen_rows),
        ) else {
            return Outcome::default();
        };
        if keep_aspect {
            match (cols, rows) {
                (Some(c), None) => rows = Some(c * cw * h as f64 / w as f64 / ch),
                (None, Some(r)) => cols = Some(r * ch * w as f64 / h as f64 / cw),
                _ => {}
            }
        }
        let mut cols = cols.unwrap_or(natural.0);
        let mut rows = rows.unwrap_or(natural.1);
        // never wider than the screen
        if cols > screen_cols as f64 {
            let k = screen_cols as f64 / cols;
            cols *= k;
            if keep_aspect {
                rows *= k;
            }
        }
        let id = self.anon_id();
        let key = self.store(id, w, h, rgba);
        Outcome {
            reply: None,
            place: Some(PlaceRequest {
                image: key,
                image_id: 0,
                placement_id: 0,
                cols: cols.ceil().clamp(1.0, MAX_CELL_SPAN as f64) as usize,
                rows: rows.ceil().clamp(1.0, MAX_CELL_SPAN as f64) as usize,
                src: [0, 0, w, h],
                z: 0,
                move_cursor: true,
            }),
        }
    }

    /// Record a placement at (`line`, `col`), replacing an earlier one with
    /// the same kitty image + placement id.
    pub fn place(&mut self, req: &PlaceRequest, line: u64, col: usize, alt: bool) {
        if req.placement_id != 0 {
            self.placements
                .retain(|p| !(p.image_id == req.image_id && p.placement_id == req.placement_id));
        }
        self.placements.push(Placement {
            image: req.image,
            image_id: req.image_id,
            placement_id: req.placement_id,
            line,
            col,
            cols: req.cols,
            rows: req.rows,
            src: req.src,
            z: req.z,
            alt,
        });
        if self.placements.len() > MAX_PLACEMENTS {
            let extra = self.placements.len() - MAX_PLACEMENTS;
            self.placements.drain(..extra);
        }
    }

    /// Drop placements whose last row left the scrollback.
    pub fn prune(&mut self, oldest_kept: u64) {
        self.placements
            .retain(|p| p.line + p.rows as u64 > oldest_kept);
    }

    /// Leaving the alternate screen discards what was drawn on it.
    pub fn drop_alt(&mut self) {
        self.placements.retain(|p| !p.alt);
    }

    pub fn clear(&mut self) {
        self.placements.clear();
    }

    pub fn remap(&mut self, mut f: impl FnMut(u64, usize) -> (u64, usize)) {
        for p in &mut self.placements {
            let (l, c) = f(p.line, p.col);
            p.line = l;
            p.col = c;
        }
    }

    pub fn stored_bytes(&self) -> usize {
        self.stored_bytes
    }
}

fn place_request(ctrl: &Control, img: &ImageData, cell_px: (u32, u32)) -> PlaceRequest {
    let [sx, sy, sw, sh] = ctrl.src;
    let sx = sx.min(img.width);
    let sy = sy.min(img.height);
    let sw = if sw == 0 {
        img.width - sx
    } else {
        sw.min(img.width - sx)
    };
    let sh = if sh == 0 {
        img.height - sy
    } else {
        sh.min(img.height - sy)
    };
    let (cw, ch) = (cell_px.0.max(1), cell_px.1.max(1));
    let cols = if ctrl.cols > 0 {
        ctrl.cols
    } else {
        sw.div_ceil(cw)
    };
    let rows = if ctrl.rows > 0 {
        ctrl.rows
    } else {
        sh.div_ceil(ch)
    };
    PlaceRequest {
        image: img.key,
        image_id: ctrl.id,
        placement_id: ctrl.placement,
        cols: (cols as usize).clamp(1, MAX_CELL_SPAN),
        rows: (rows as usize).clamp(1, MAX_CELL_SPAN),
        src: [sx, sy, sw, sh],
        z: ctrl.z,
        move_cursor: !ctrl.no_cursor_move,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b64(data: &[u8]) -> String {
        const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = String::new();
        for c in data.chunks(3) {
            let n = (c[0] as u32) << 16
                | (*c.get(1).unwrap_or(&0) as u32) << 8
                | *c.get(2).unwrap_or(&0) as u32;
            for i in 0..4 {
                if i <= c.len() {
                    out.push(T[((n >> (18 - 6 * i)) & 63) as usize] as char);
                } else {
                    out.push('=');
                }
            }
        }
        out
    }

    fn png(w: u32, h: u32) -> Vec<u8> {
        let img = image::RgbaImage::from_pixel(w, h, image::Rgba([10, 20, 30, 255]));
        let mut out = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
            .unwrap();
        out
    }

    #[test]
    fn base64_roundtrip() {
        for s in [&b""[..], b"a", b"ab", b"abc", b"hello world!"] {
            assert_eq!(base64_decode(b64(s).as_bytes()).unwrap(), s);
        }
        assert!(base64_decode(b"a$b").is_none());
    }

    #[test]
    fn query_answers_ok_without_storing() {
        let mut g = Graphics::default();
        let out = g.kitty(b"a=q,i=31,s=1,v=1,t=d,f=24;AAAA", (10, 20));
        assert_eq!(out.reply.as_deref(), Some("\x1b_Gi=31;OK\x1b\\"));
        assert!(out.place.is_none());
        assert_eq!(g.stored_bytes(), 0);
        // file transmission is refused, never read
        let out = g.kitty(b"a=q,i=32,t=f,f=100;L2V0Yy9wYXNzd2Q=", (10, 20));
        assert!(out.reply.unwrap().contains("EINVAL"));
    }

    #[test]
    fn transmit_and_display_png_in_chunks() {
        let data = b64(&png(25, 50));
        let (a, b) = data.split_at(data.len() / 2);
        let mut g = Graphics::default();
        let first = format!("a=T,f=100,i=7,m=1;{a}");
        assert_eq!(g.kitty(first.as_bytes(), (10, 20)), Outcome::default());
        let out = g.kitty(format!("m=0;{b}").as_bytes(), (10, 20));
        assert_eq!(out.reply.as_deref(), Some("\x1b_Gi=7;OK\x1b\\"));
        let place = out.place.unwrap();
        assert_eq!((place.cols, place.rows), (3, 3));
        assert_eq!(place.src, [0, 0, 25, 50]);
        assert_eq!(g.stored_bytes(), 25 * 50 * 4);
        // put it again elsewhere by id, then delete
        let out = g.kitty(b"a=p,i=7,p=2,c=10,r=2,q=1", (10, 20));
        assert_eq!(out.reply, None);
        assert_eq!(out.place.as_ref().unwrap().cols, 10);
        g.place(out.place.as_ref().unwrap(), 5, 0, false);
        assert_eq!(g.placements.len(), 1);
        g.kitty(b"a=d,d=I,i=7", (10, 20));
        assert!(g.placements.is_empty());
        assert_eq!(g.stored_bytes(), 0);
    }

    #[test]
    fn image_numbers_follow_image_lifetimes() {
        let mut g = Graphics::default();
        for number in 1..=100 {
            let command = format!("a=t,f=24,s=1,v=1,i=7,I={number};AAAA");
            g.kitty(command.as_bytes(), (10, 20));
            assert_eq!(
                g.numbers.len(),
                1,
                "replacing an image forgets its old number"
            );
            assert_eq!(g.numbers.get(&number), Some(&7));
        }
        g.kitty(b"a=d,d=I,i=7", (10, 20));
        assert!(g.numbers.is_empty(), "freeing an image forgets its number");
        assert_eq!(g.stored_bytes(), 0);
        g.kitty(b"a=t,f=24,s=1,v=1,i=7;AAAA", (10, 20));
        g.kitty(b"a=d,d=N,I=100", (10, 20));
        assert!(
            g.images.contains_key(&7),
            "an old number cannot delete a reused id"
        );
    }

    #[test]
    fn deleting_numbered_images_keeps_no_aliases() {
        let mut g = Graphics::default();
        for number in 1..=100 {
            let command = format!("a=t,f=24,s=1,v=1,i=7,I={number};AAAA");
            g.kitty(command.as_bytes(), (10, 20));
            g.kitty(b"a=d,d=I,i=7", (10, 20));
            assert!(g.numbers.is_empty());
            assert!(g.images.is_empty());
            assert_eq!(g.stored_bytes(), 0);
        }
    }

    #[test]
    fn evicting_images_forgets_their_numbers() {
        let mut g = Graphics::default();
        g.kitty(b"a=t,f=24,s=1,v=1,i=7,I=100;AAAA", (10, 20));
        // Put accounting just over the eviction boundary without allocating
        // hundreds of MiB: store still performs the real eviction path.
        g.stored_bytes = MAX_STORED_BYTES;
        g.store(8, 1, 1, vec![0; 4]);
        assert!(!g.images.contains_key(&7));
        assert!(g.numbers.is_empty());
        assert!(g.images.contains_key(&8));
    }

    #[test]
    fn delete_selectors_are_scoped() {
        let px = b64(&[255, 0, 0, 255]);
        let put = |g: &mut Graphics, cmd: &str| {
            let out = g.kitty(format!("{cmd};{px}").as_bytes(), (10, 20));
            g.place(out.place.as_ref().unwrap(), 5, 0, false);
        };
        let setup = || {
            let mut g = Graphics::default();
            put(&mut g, "a=T,f=32,s=1,v=1,i=1,p=1");
            put(&mut g, "a=T,f=32,s=1,v=1,i=2,I=77,p=1");
            put(&mut g, "a=T,f=32,s=1,v=1,i=3,p=1");
            assert_eq!(g.placements.len(), 3);
            g
        };
        // selectors that are not implemented delete nothing
        for d in ["c", "C", "x", "y", "z", "r", "f", "q", "p", "P"] {
            let mut g = setup();
            g.kitty(format!("a=d,d={d}").as_bytes(), (10, 20));
            assert_eq!(g.placements.len(), 3, "d={d}");
            assert_eq!(g.images.len(), 3, "d={d}");
        }
        // by id: only that image's placements; uppercase frees the data
        let mut g = setup();
        g.kitty(b"a=d,d=i,i=1", (10, 20));
        assert_eq!(g.placements.len(), 2);
        assert_eq!(g.images.len(), 3);
        g.kitty(b"a=d,d=I,i=3", (10, 20));
        assert_eq!(g.placements.len(), 1);
        assert_eq!(g.images.len(), 2);
        // by number
        let mut g = setup();
        g.kitty(b"a=d,d=N,I=77", (10, 20));
        assert_eq!(g.placements.len(), 2);
        assert!(!g.images.contains_key(&2));
        // a / A (and the default) delete everything
        let mut g = setup();
        g.kitty(b"a=d", (10, 20));
        assert!(g.placements.is_empty());
        assert_eq!(g.images.len(), 3);
        g.kitty(b"a=d,d=A", (10, 20));
        assert_eq!(g.images.len(), 0);
        assert_eq!(g.stored_bytes(), 0);
    }

    #[test]
    fn raw_rgb_and_errors() {
        let mut g = Graphics::default();
        let px = b64(&[255, 0, 0, 0, 255, 0]);
        let out = g.kitty(format!("a=T,f=24,s=2,v=1,i=1;{px}").as_bytes(), (10, 20));
        assert!(out.place.is_some());
        let img = g.images[&1].clone();
        assert_eq!(*img.rgba, vec![255, 0, 0, 255, 0, 255, 0, 255]);
        // too little data
        let out = g.kitty(format!("a=t,f=32,s=4,v=4,i=2;{px}").as_bytes(), (10, 20));
        assert!(out.reply.unwrap().contains("ENODATA"));
        // oversize dimensions never allocate
        let out = g.kitty(b"a=t,f=32,s=100000,v=100000,i=3;AAAA", (10, 20));
        assert!(out.reply.unwrap().contains("EINVAL"));
        // placing an unknown image
        let out = g.kitty(b"a=p,i=99", (10, 20));
        assert!(out.reply.unwrap().contains("ENOENT"));
    }

    #[test]
    fn iterm_inline_image_sizes() {
        let mut g = Graphics::default();
        let body = format!("inline=1;width=5:{}", b64(&png(100, 50)));
        let place = g.iterm(body.as_bytes(), (10, 20), 80, 24).place.unwrap();
        assert_eq!((place.cols, place.rows), (5, 2)); // aspect kept: 50px tall
        let body = format!("inline=1:{}", b64(&png(100, 50)));
        let place = g.iterm(body.as_bytes(), (10, 20), 8, 24).place.unwrap();
        assert_eq!(place.cols, 8, "clamped to the screen width");
        // not inline: ignored (no download support)
        let body = format!("name=eA==;inline=0:{}", b64(&png(2, 2)));
        assert_eq!(
            g.iterm(body.as_bytes(), (10, 20), 80, 24),
            Outcome::default()
        );
        for height in ["NaN", "inf", "-1", "1e999%"] {
            let body = format!("inline=1;height={height}:{}", b64(&png(2, 2)));
            assert!(g.iterm(body.as_bytes(), (10, 20), 80, 24).place.is_none());
        }
        for (height, expected) in [("50%", 12), ("999999999999", MAX_CELL_SPAN)] {
            let body = format!(
                "inline=1;width=1;preserveAspectRatio=0;height={height}:{}",
                b64(&png(2, 2))
            );
            assert_eq!(
                g.iterm(body.as_bytes(), (10, 20), 80, 24)
                    .place
                    .unwrap()
                    .rows,
                expected
            );
        }
    }

    #[test]
    fn placements_prune_and_bound() {
        let mut g = Graphics::default();
        let req = PlaceRequest {
            image: 1,
            image_id: 0,
            placement_id: 0,
            cols: 1,
            rows: 3,
            src: [0; 4],
            z: 0,
            move_cursor: true,
        };
        for i in 0..MAX_PLACEMENTS as u64 + 10 {
            g.place(&req, i, 0, i % 2 == 0);
        }
        assert_eq!(g.placements.len(), MAX_PLACEMENTS);
        g.prune(400);
        assert!(g.placements.iter().all(|p| p.line + 3 > 400));
        g.drop_alt();
        assert!(g.placements.iter().all(|p| !p.alt));
    }
}
