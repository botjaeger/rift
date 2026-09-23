use crate::color::to_vec4;
use crate::quad::Quad;
use glyphon::{Buffer, TextBounds};
use runway_core::Color;
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
    pub fn rect(&mut self, pos: [f32; 2], size: [f32; 2], color: Color, radius: f32) {
        self.quads
            .push(Quad::rect(pos, size, to_vec4(color), radius));
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
        self.quads.push(Quad::bordered(
            pos,
            size,
            to_vec4(fill),
            to_vec4(border),
            radius,
            border_width,
        ));
    }

    pub fn circle(&mut self, center: [f32; 2], diameter: f32, color: Color) {
        self.quads
            .push(Quad::circle(center, diameter, to_vec4(color)));
    }

    pub fn hline(&mut self, x: f32, y: f32, w: f32, thickness: f32, color: Color) {
        self.rect([x, y], [w, thickness], color, 0.0);
    }

    pub fn shadow(&mut self, pos: [f32; 2], size: [f32; 2], color: Color, radius: f32, sigma: f32) {
        self.quads
            .push(Quad::shadow(pos, size, to_vec4(color), radius, sigma));
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
