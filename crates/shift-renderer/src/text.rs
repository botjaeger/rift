use glyphon::{
    Attrs, Buffer, Cache, Family, FontSystem, Metrics, Shaping, SwashCache, TextArea, TextAtlas,
    TextRenderer, Viewport,
};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};

pub const FONT_MONO: &str = "JetBrains Mono";
pub const FONT_SANS: &str = "Inter";

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
const INTER: &[u8] = include_bytes!("../../../assets/fonts/Inter-Variable.ttf");

/// Fonts, glyph atlas and per-layer text renderers.
pub struct TextEngine {
    pub font_system: FontSystem,
    pub swash: SwashCache,
    pub viewport: Viewport,
    pub atlas: TextAtlas,
    renderers: Vec<TextRenderer>,
    shape_cache: HashMap<u64, ShapedEntry>,
    frame: u64,
}

struct ShapedEntry {
    buf: Arc<Buffer>,
    last_used: u64,
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
            db.load_font_data(INTER.to_vec());
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
        }
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
        }
        resolved
    }

    /// Installed monospace families (deduped, sorted), embedded first.
    pub fn monospace_families(&self) -> Vec<String> {
        let mut out: Vec<String> = vec![FONT_MONO.to_string()];
        for face in self.font_system.db().faces() {
            if !face.monospaced {
                continue;
            }
            if let Some((name, _)) = face.families.first()
                && !out.iter().any(|n| n == name)
            {
                out.push(name.clone());
            }
        }
        out[1..].sort();
        out
    }

    /// Layout width of a one-line string (logical px).
    pub fn measure(
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
            Metrics::new(font_size, font_size * 1.65),
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
