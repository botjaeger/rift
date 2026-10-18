use crate::color::to_vec4;
use crate::quad::Quad;
use glyphon::{Buffer, TextBounds};
use rift_core::Color;
use std::sync::Arc;

/// One positioned text run (shaped buffer shared with the engine's cache).
pub struct TextItem {
    pub buffer: Arc<Buffer>,
    pub left: f32,
    pub top: f32,
    pub color: glyphon::Color,
    pub bounds: Option<TextBounds>,
}

/// Draw list layer: quads render first, then text on top. Later layers
/// composite over earlier ones (chrome → terminal → overlays).
#[derive(Default)]
pub struct Layer {
    pub quads: Vec<Quad>,
    pub texts: Vec<TextItem>,
    /// Inline images (drawn under or over this layer's text).
    pub sprites: Vec<crate::sprite::Sprite>,
    /// Rounded clip `([x, y, w, h], radius)` applied to quads pushed while
    /// set (see [`Layer::set_clip`]).
    pub clip: Option<([f32; 4], f32)>,
}

/// Per-frame draw list.
#[derive(Default)]
pub struct Scene {
    pub layers: Vec<Layer>,
}

pub fn text_color(c: Color) -> glyphon::Color {
    let b = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    glyphon::Color::rgba(b(c.r), b(c.g), b(c.b), b(c.a))
}

impl Scene {
    pub fn clear(&mut self) {
        self.layers.clear();
    }

    /// Get (creating) layer `i`.
    pub fn layer(&mut self, i: usize) -> &mut Layer {
        while self.layers.len() <= i {
            self.layers.push(Layer::default());
        }
        &mut self.layers[i]
    }
}

impl Layer {
    /// Clip every quad pushed from now on to a rounded rect (a panel's inner
    /// edge), so rows and highlights follow its corners. `None` ends it.
    pub fn set_clip(&mut self, clip: Option<([f32; 4], f32)>) {
        self.clip = clip;
    }

    fn push(&mut self, mut q: Quad) {
        if let Some((r, radius)) = self.clip {
            q.clip = r;
            q.clip_radius = radius;
        }
        self.quads.push(q);
    }

    /// Round-capped stroke from `a` to `b` (icon lines: ×, chevrons).
    pub fn line(&mut self, a: [f32; 2], b: [f32; 2], width: f32, color: Color) {
        self.push(Quad::line(a, b, width, to_vec4(color)));
    }

    pub fn rect(&mut self, pos: [f32; 2], size: [f32; 2], color: Color, radius: f32) {
        self.push(Quad::rect(pos, size, to_vec4(color), radius));
    }

    pub fn bordered(
        &mut self,
        pos: [f32; 2],
        size: [f32; 2],
        fill: Color,
        border: Color,
        radius: f32,
        border_width: f32,
    ) {
        self.push(Quad::bordered(
            pos,
            size,
            to_vec4(fill),
            to_vec4(border),
            radius,
            border_width,
        ));
    }

    /// A rounded box filled with a two-stop linear gradient, `from` at the
    /// top (or left) edge and `to` at the bottom (or right).
    pub fn gradient(
        &mut self,
        pos: [f32; 2],
        size: [f32; 2],
        from: Color,
        to: Color,
        vertical: bool,
        radius: f32,
    ) {
        self.push(Quad::gradient(
            pos,
            size,
            to_vec4(from),
            to_vec4(to),
            vertical,
            radius,
        ));
    }

    pub fn circle(&mut self, center: [f32; 2], diameter: f32, color: Color) {
        self.push(Quad::circle(center, diameter, to_vec4(color)));
    }

    pub fn hline(&mut self, x: f32, y: f32, w: f32, thickness: f32, color: Color) {
        self.rect([x, y], [w, thickness], color, 0.0);
    }

    pub fn shadow(&mut self, pos: [f32; 2], size: [f32; 2], color: Color, radius: f32, sigma: f32) {
        self.push(Quad::shadow(pos, size, to_vec4(color), radius, sigma));
    }

    pub fn text(&mut self, buffer: impl Into<Arc<Buffer>>, left: f32, top: f32, color: Color) {
        self.texts.push(TextItem {
            buffer: buffer.into(),
            left,
            top,
            color: text_color(color),
            bounds: None,
        });
    }

    pub fn text_clipped(
        &mut self,
        buffer: impl Into<Arc<Buffer>>,
        left: f32,
        top: f32,
        color: Color,
        bounds: TextBounds,
    ) {
        self.texts.push(TextItem {
            buffer: buffer.into(),
            left,
            top,
            color: text_color(color),
            bounds: Some(bounds),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clip_applies_only_while_set() {
        let mut layer = Layer::default();
        let c = Color::rgba(1.0, 1.0, 1.0, 1.0);
        layer.rect([0.0, 0.0], [10.0, 10.0], c, 0.0);
        layer.set_clip(Some(([1.0, 2.0, 30.0, 40.0], 9.0)));
        layer.rect([0.0, 0.0], [10.0, 10.0], c, 0.0);
        layer.line([0.0, 0.0], [5.0, 5.0], 1.5, c);
        layer.set_clip(None);
        layer.circle([5.0, 5.0], 4.0, c);
        let clips: Vec<_> = layer
            .quads
            .iter()
            .map(|q| (q.clip, q.clip_radius))
            .collect();
        assert_eq!(clips[0], ([0.0; 4], 0.0));
        assert_eq!(clips[1], ([1.0, 2.0, 30.0, 40.0], 9.0));
        assert_eq!(clips[2], ([1.0, 2.0, 30.0, 40.0], 9.0));
        assert_eq!(clips[3], ([0.0; 4], 0.0));
    }

    #[test]
    fn gradient_quad_carries_both_stops_and_its_axis() {
        let mut layer = Layer::default();
        let (from, to) = (
            Color::rgba(1.0, 0.0, 0.0, 1.0),
            Color::rgba(0.0, 0.0, 1.0, 1.0),
        );
        layer.gradient([0.0, 0.0], [10.0, 20.0], from, to, true, 0.0);
        layer.gradient([0.0, 0.0], [10.0, 20.0], from, to, false, 0.0);
        let (v, h) = (layer.quads[0], layer.quads[1]);
        assert_eq!(v.kind, 2.0, "kind 2: the shader's gradient box");
        assert_eq!(v.color, [1.0, 0.0, 0.0, 1.0]);
        assert_eq!(v.border_color, [0.0, 0.0, 1.0, 1.0]);
        assert_eq!(v.border_width, 0.0, "no ring over the gradient");
        assert_eq!(&v.line[..2], &[0.0, 1.0], "top to bottom");
        assert_eq!(&h.line[..2], &[1.0, 0.0], "left to right");
    }

    #[test]
    fn line_box_covers_both_ends_and_the_stroke() {
        let q = Quad::line([10.0, 20.0], [4.0, 8.0], 2.0, [1.0; 4]);
        assert_eq!(q.kind, 1.0);
        assert_eq!(q.line, [10.0, 20.0, 4.0, 8.0]);
        // half the stroke plus one pixel of anti-aliasing on every side
        assert_eq!(q.pos, [2.0, 6.0]);
        assert_eq!(q.size, [10.0, 16.0]);
    }
}
