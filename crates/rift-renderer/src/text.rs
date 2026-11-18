use glyphon::{
    Attrs, Buffer, Cache, Family, FontSystem, Metrics, Shaping, SwashCache, TextArea, TextAtlas,
    TextRenderer, Viewport,
};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};

pub const FONT_MONO: &str = "JetBrains Mono";
pub const FONT_SANS: &str = "Inter";
/// Nerd Font symbols (brand and tool marks on the prompt chips, the running
/// spinner): an embedded subset of the glyphs Rift draws.
pub const FONT_SYMBOLS: &str = "Symbols Nerd Font";

/// Terminal font family currently in use (config `font_family`, falling
/// back to the embedded JetBrains Mono). Read by every mono text site.
static MONO_FAMILY: RwLock<&'static str> = RwLock::new(FONT_MONO);
/// Interned family names so repeated config changes don't keep leaking.
static INTERNED: Mutex<Vec<&'static str>> = Mutex::new(Vec::new());

pub fn mono_family() -> &'static str {
    *MONO_FAMILY.read().unwrap()
}

fn intern(name: &str) -> &'static str {
    let mut pool = INTERNED.lock().unwrap();
    if let Some(s) = pool.iter().find(|s| **s == name) {
        return s;
    }
    let leaked: &'static str = Box::leak(name.to_string().into_boxed_str());
    pool.push(leaked);
    leaked
}

const JBM_REGULAR: &[u8] = include_bytes!("../../../assets/fonts/JetBrainsMono-Regular.ttf");
const JBM_MEDIUM: &[u8] = include_bytes!("../../../assets/fonts/JetBrainsMono-Medium.ttf");
const JBM_SEMIBOLD: &[u8] = include_bytes!("../../../assets/fonts/JetBrainsMono-SemiBold.ttf");
const JBM_BOLD: &[u8] = include_bytes!("../../../assets/fonts/JetBrainsMono-Bold.ttf");
const JBM_ITALIC: &[u8] = include_bytes!("../../../assets/fonts/JetBrainsMono-Italic.ttf");
const JBM_BOLD_ITALIC: &[u8] = include_bytes!("../../../assets/fonts/JetBrainsMono-BoldItalic.ttf");
/// Hack, offered in the terminal font picker (Warp's default font).
const HACK: [&[u8]; 4] = [
    include_bytes!("../../../assets/fonts/Hack-Regular.ttf"),
    include_bytes!("../../../assets/fonts/Hack-Bold.ttf"),
    include_bytes!("../../../assets/fonts/Hack-Italic.ttf"),
    include_bytes!("../../../assets/fonts/Hack-BoldItalic.ttf"),
];
const INTER: &[u8] = include_bytes!("../../../assets/fonts/Inter-Variable.ttf");
const SYMBOLS: &[u8] = include_bytes!("../../../assets/fonts/SymbolsNerdFont-subset.ttf");

/// A face fit for the terminal grid: outline glyphs (not bitmap-only) and
/// one advance width across narrow, wide and punctuation glyphs.
fn is_terminal_font(data: &[u8], index: u32) -> bool {
    let Ok(face) = ttf_parser::Face::parse(data, index) else {
        return false;
    };
    let t = face.tables();
    if t.glyf.is_none() && t.cff.is_none() && t.cff2.is_none() {
        return false;
    }
    let advance = |c: char| face.glyph_index(c).and_then(|g| face.glyph_hor_advance(g));
    let m = advance('M');
    m.is_some_and(|m| m > 0) && ['i', 'W', 'l', '.', '0'].iter().all(|&c| advance(c) == m)
}

/// Fonts, glyph atlas and per-layer text renderers.
pub struct TextEngine {
    pub font_system: FontSystem,
    pub swash: SwashCache,
    pub viewport: Viewport,
    pub atlas: TextAtlas,
    renderers: Vec<TextRenderer>,
    shape_cache: HashMap<u64, ShapedEntry>,
    frame: u64,
    /// [`Self::family_covers`] answers.
    coverage: HashMap<(String, char), bool>,
    measured: MeasureCache,
}

struct ShapedEntry {
    buf: Arc<Buffer>,
    last_used: u64,
}

/// Entries [`MeasureCache`] keeps before it starts over.
const MEASURE_MAX_ENTRIES: usize = 4096;
/// Text bytes [`MeasureCache`] keeps before it starts over.
const MEASURE_MAX_BYTES: usize = 1 << 20;
/// Longer strings are measured but not kept: they are rarely asked twice
/// and terminal output is untrusted.
const MEASURE_MAX_TEXT: usize = 1024;

struct Measured {
    text: String,
    size: u32,
    family: &'static str,
    weight: u16,
    width: f32,
}

/// Memo of [`TextEngine::measure`] answers, keyed by a hash of every input
/// and checked against the stored inputs. Bounded in entries and text
/// bytes; it starts over when a bound is hit.
#[derive(Default)]
struct MeasureCache {
    map: HashMap<u64, Measured>,
    bytes: usize,
}

impl MeasureCache {
    fn width(
        &mut self,
        text: &str,
        size: f32,
        family: &'static str,
        weight: u16,
        shape: impl FnOnce() -> f32,
    ) -> f32 {
        use std::hash::{Hash, Hasher};
        let size = size.to_bits();
        let mut h = std::collections::hash_map::DefaultHasher::new();
        (text, size, family, weight).hash(&mut h);
        let key = h.finish();
        if let Some(m) = self.map.get(&key)
            && m.size == size
            && m.family == family
            && m.weight == weight
            && m.text == text
        {
            return m.width;
        }
        let width = shape();
        if text.len() <= MEASURE_MAX_TEXT {
            if self.map.len() >= MEASURE_MAX_ENTRIES || self.bytes + text.len() > MEASURE_MAX_BYTES
            {
                self.clear();
            }
            let old = self.map.insert(
                key,
                Measured {
                    text: text.to_string(),
                    size,
                    family,
                    weight,
                    width,
                },
            );
            self.bytes += text.len();
            if let Some(old) = old {
                self.bytes -= old.text.len();
            }
        }
        width
    }

    fn clear(&mut self) {
        self.map.clear();
        self.bytes = 0;
    }
}

impl TextEngine {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, format: wgpu::TextureFormat) -> Self {
        let mut font_system = FontSystem::new();
        {
            let db = font_system.db_mut();
            db.load_font_data(JBM_REGULAR.to_vec());
            db.load_font_data(JBM_MEDIUM.to_vec());
            db.load_font_data(JBM_SEMIBOLD.to_vec());
            db.load_font_data(JBM_BOLD.to_vec());
            db.load_font_data(JBM_ITALIC.to_vec());
            db.load_font_data(JBM_BOLD_ITALIC.to_vec());
            for face in HACK {
                db.load_font_data(face.to_vec());
            }
            db.load_font_data(INTER.to_vec());
            db.load_font_data(SYMBOLS.to_vec());
            // The embedded file's family is "Inter Variable" with a single
            // weight-400 face, so `Family::Name("Inter")` never matched: normal
            // text only reached it through fallback, and MEDIUM/SEMIBOLD/BOLD
            // sans text fell back to the monospace family. Register the face
            // as "Inter" under every weight; rendering applies the `wght`
            // axis from the requested weight.
            let inter: Vec<_> = db
                .faces()
                .filter(|f| f.families.iter().any(|(n, _)| n.starts_with(FONT_SANS)))
                .cloned()
                .collect();
            for face in inter {
                for w in (100..=900).step_by(100) {
                    let mut clone = face.clone();
                    clone.families = vec![(
                        FONT_SANS.to_string(),
                        glyphon::fontdb::Language::English_UnitedStates,
                    )];
                    clone.weight = glyphon::fontdb::Weight(w);
                    db.push_face_info(clone);
                }
            }
        }

        let swash = SwashCache::new();
        let cache = Cache::new(device);
        let viewport = Viewport::new(device, &cache);
        // ColorMode::Web: colors blended in sRGB space, matching the design
        // mock's CSS compositing (surface format is non-sRGB, see gpu.rs)
        let atlas =
            TextAtlas::with_color_mode(device, queue, &cache, format, glyphon::ColorMode::Web);

        TextEngine {
            font_system,
            swash,
            viewport,
            atlas,
            renderers: Vec::new(),
            shape_cache: HashMap::new(),
            frame: 0,
            coverage: HashMap::new(),
            measured: MeasureCache::default(),
        }
    }

    /// Whether `family` (its regular face) has a glyph for `c`. Cached: the
    /// grid asks per cell for the few characters that can draw as emoji.
    pub fn family_covers(&mut self, family: &str, c: char) -> bool {
        if let Some(&has) = self.coverage.get(&(family.to_string(), c)) {
            return has;
        }
        let families = [glyphon::fontdb::Family::Name(family)];
        let query = glyphon::fontdb::Query {
            families: &families,
            ..Default::default()
        };
        let has = self.font_system.db().query(&query).is_some_and(|id| {
            self.font_system
                .get_font(id, glyphon::fontdb::Weight::NORMAL)
                .is_some_and(|f| f.as_swash().charmap().map(c) != 0)
        });
        self.coverage.insert((family.to_string(), c), has);
        has
    }

    /// Cached shaping: return the shaped buffer for `key`, building it at
    /// most once. Callers must hash every shaping input (text, per-span
    /// colors/weights, metrics, font features) into the key.
    pub fn shaped(
        &mut self,
        key: u64,
        build: impl FnOnce(&mut FontSystem) -> Buffer,
    ) -> Arc<Buffer> {
        if let Some(e) = self.shape_cache.get_mut(&key) {
            e.last_used = self.frame;
            return e.buf.clone();
        }
        let buf = Arc::new(build(&mut self.font_system));
        self.shape_cache.insert(
            key,
            ShapedEntry {
                buf: buf.clone(),
                last_used: self.frame,
            },
        );
        buf
    }

    /// True when a font with this family name is loaded (embedded or system).
    pub fn has_family(&self, name: &str) -> bool {
        self.font_system.db().faces().any(|f| {
            f.families
                .iter()
                .any(|(n, _)| n.eq_ignore_ascii_case(name.trim()))
        })
    }

    /// Select the terminal font family. Unknown families fall back to the
    /// embedded JetBrains Mono. Returns the family actually in use; the
    /// shape cache is dropped when it changes.
    pub fn set_mono_family(&mut self, name: &str) -> &'static str {
        let resolved = if self.has_family(name) {
            intern(name.trim())
        } else {
            FONT_MONO
        };
        if resolved != mono_family() {
            *MONO_FAMILY.write().unwrap() = resolved;
            self.shape_cache.clear();
            self.measured.clear();
        }
        resolved
    }

    /// Installed monospace families (deduped, sorted), the default first.
    /// A family counts when its regular face draws outlines at one advance
    /// width: the fixed-pitch flag alone misses Monaco (it does not set
    /// it) and admits bitmap-only faces such as "GB18030 Bitmap".
    pub fn monospace_families(&self) -> Vec<String> {
        let db = self.font_system.db();
        let mut seen: Vec<&str> = Vec::new();
        let mut out: Vec<String> = Vec::new();
        for face in db.faces() {
            let Some((name, _)) = face.families.first() else {
                continue;
            };
            // dot-prefixed families (".SF NS Mono") are macOS-private;
            // judge each family once, by its first upright face
            if name.starts_with('.')
                || name == FONT_MONO
                || face.style != glyphon::fontdb::Style::Normal
                || seen.contains(&name.as_str())
            {
                continue;
            }
            seen.push(name);
            if db
                .with_face_data(face.id, is_terminal_font)
                .unwrap_or(false)
            {
                out.push(name.clone());
            }
        }
        out.sort();
        out.insert(0, FONT_MONO.to_string());
        out
    }

    /// Layout width of a one-line string (logical px). Memoized on
    /// (text, size, family, weight): shaping dominates frame time and the
    /// views ask for the same strings every frame. The memo is dropped when
    /// it outgrows its bounds or the terminal font changes.
    pub fn measure(
        &mut self,
        text: &str,
        font_size: f32,
        family: &'static str,
        weight: glyphon::Weight,
    ) -> f32 {
        let mut cache = std::mem::take(&mut self.measured);
        let width = cache.width(text, font_size, family, weight.0, || {
            self.shape_width(text, font_size, family, weight)
        });
        self.measured = cache;
        width
    }

    fn shape_width(
        &mut self,
        text: &str,
        font_size: f32,
        family: &'static str,
        weight: glyphon::Weight,
    ) -> f32 {
        let mut buf = Buffer::new(
            &mut self.font_system,
            Metrics::new(font_size, font_size * 1.4),
        );
        buf.set_text(
            &mut self.font_system,
            text,
            &Attrs::new().family(Family::Name(family)).weight(weight),
            Shaping::Advanced,
            None,
        );
        buf.layout_runs().map(|r| r.line_w).fold(0.0f32, f32::max)
    }

    /// Advance width of one terminal cell for `font_size` mono text.
    pub fn mono_cell_width(&mut self, font_size: f32) -> f32 {
        let mut buf = Buffer::new(
            &mut self.font_system,
            Metrics::new(font_size, font_size * 1.2),
        );
        buf.set_text(
            &mut self.font_system,
            "M",
            &Attrs::new().family(Family::Name(mono_family())),
            Shaping::Advanced,
            None,
        );
        buf.layout_runs()
            .next()
            .and_then(|run| run.glyphs.first().map(|g| g.w))
            .unwrap_or(font_size * 0.6)
    }

    /// Upload text for each layer. `layers[i]` renders between quad layer i
    /// and quad layer i+1.
    pub fn prepare_layers(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        physical: (u32, u32),
        layers: Vec<Vec<TextArea<'_>>>,
    ) {
        self.viewport.update(
            queue,
            glyphon::Resolution {
                width: physical.0,
                height: physical.1,
            },
        );
        while self.renderers.len() < layers.len() {
            self.renderers.push(TextRenderer::new(
                &mut self.atlas,
                device,
                wgpu::MultisampleState::default(),
                None,
            ));
        }
        for (i, areas) in layers.into_iter().enumerate() {
            let prepare = |this: &mut Self, areas: Vec<TextArea<'_>>| {
                this.renderers[i].prepare(
                    device,
                    queue,
                    &mut this.font_system,
                    &mut this.atlas,
                    &this.viewport,
                    areas,
                    &mut this.swash,
                )
            };
            if prepare(self, areas.clone()).is_err() {
                // atlas full: free glyphs not used this frame, retry once
                self.atlas.trim();
                if let Err(e) = prepare(self, areas) {
                    // degrade: drop this layer's text for the frame rather
                    // than crash (empty prepare clears stale vertices)
                    log::error!("text layer {i} skipped: {e}");
                    let _ = prepare(self, Vec::new());
                }
            }
        }
    }

    pub fn render_layer(&self, i: usize, pass: &mut wgpu::RenderPass<'_>) {
        if let Some(r) = self.renderers.get(i) {
            let _ = r.render(&self.atlas, &self.viewport, pass);
        }
    }

    /// Trim the atlas after a frame (glyphon requirement to avoid growth)
    /// and age out shape-cache entries that haven't been drawn recently.
    pub fn trim(&mut self) {
        self.atlas.trim();
        self.frame += 1;
        if self.shape_cache.len() > 4096 {
            let cutoff = self.frame.saturating_sub(300);
            self.shape_cache.retain(|_, e| e.last_used >= cutoff);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_fonts_are_outline_and_fixed_width() {
        assert!(is_terminal_font(JBM_REGULAR, 0));
        assert!(HACK.iter().all(|f| is_terminal_font(f, 0)));
        // proportional, symbol-only and malformed faces are not
        assert!(!is_terminal_font(INTER, 0));
        assert!(!is_terminal_font(SYMBOLS, 0));
        assert!(!is_terminal_font(b"not a font", 0));
    }

    /// `width` through `cache`, counting the times it shaped.
    fn ask(
        cache: &mut MeasureCache,
        shaped: &mut u32,
        text: &str,
        size: f32,
        family: &'static str,
    ) -> f32 {
        cache.width(text, size, family, 400, || {
            *shaped += 1;
            text.len() as f32 * size
        })
    }

    #[test]
    fn measure_shapes_a_repeated_input_once() {
        let (mut c, mut n) = (MeasureCache::default(), 0);
        let a = ask(&mut c, &mut n, "hello", 12.0, FONT_SANS);
        let b = ask(&mut c, &mut n, "hello", 12.0, FONT_SANS);
        assert_eq!((a, b, n), (60.0, 60.0, 1));
    }

    #[test]
    fn measure_keys_on_every_input() {
        let (mut c, mut n) = (MeasureCache::default(), 0);
        ask(&mut c, &mut n, "hello", 12.0, FONT_SANS);
        ask(&mut c, &mut n, "hello", 13.0, FONT_SANS);
        ask(&mut c, &mut n, "hello", 12.0, FONT_MONO);
        ask(&mut c, &mut n, "hellp", 12.0, FONT_SANS);
        c.width("hello", 12.0, FONT_SANS, 500, || {
            n += 1;
            0.0
        });
        assert_eq!(n, 5);
        assert_eq!(c.map.len(), 5);
    }

    #[test]
    fn measure_cache_stays_bounded() {
        let (mut c, mut n) = (MeasureCache::default(), 0);
        for i in 0..MEASURE_MAX_ENTRIES * 3 {
            ask(&mut c, &mut n, &format!("text {i}"), 12.0, FONT_SANS);
            assert!(c.map.len() <= MEASURE_MAX_ENTRIES);
        }
        // by bytes: 1000-byte strings reach the byte bound first
        let mut c = MeasureCache::default();
        for i in 0..(MEASURE_MAX_BYTES / 1000) * 3 {
            ask(&mut c, &mut n, &format!("{i:0>1000}"), 12.0, FONT_SANS);
            assert!(c.bytes <= MEASURE_MAX_BYTES);
        }
        // oversized strings are answered but never kept
        let long = "x".repeat(MEASURE_MAX_TEXT + 1);
        let before = n;
        ask(&mut c, &mut n, &long, 12.0, FONT_SANS);
        ask(&mut c, &mut n, &long, 12.0, FONT_SANS);
        assert_eq!(n, before + 2);
    }

    #[test]
    fn measure_cache_clear_forgets() {
        let (mut c, mut n) = (MeasureCache::default(), 0);
        ask(&mut c, &mut n, "hello", 12.0, FONT_SANS);
        c.clear();
        assert_eq!((c.map.len(), c.bytes), (0, 0));
        ask(&mut c, &mut n, "hello", 12.0, FONT_SANS);
        assert_eq!(n, 2);
    }
}
