//! GPU textures for Kitty graphics placements.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use chda_term::{Image, ImagePlacement};
use gpui::RenderImage;

/// Image id, image generation and source rectangle: one texture each.
pub type ImageKey = (u32, u64, (u32, u32, u32, u32));

/// Textures of the images a terminal shows, built once per image version.
#[derive(Default)]
pub struct ImageTextures {
    textures: HashMap<ImageKey, Arc<RenderImage>>,
}

pub fn key(p: &ImagePlacement) -> ImageKey {
    (p.image.id, p.image.generation, p.source)
}

impl ImageTextures {
    /// The texture for a placement's part of its image.
    pub fn get(&mut self, p: &ImagePlacement) -> Option<Arc<RenderImage>> {
        let key = key(p);
        if let Some(texture) = self.textures.get(&key) {
            return Some(Arc::clone(texture));
        }
        let texture = Arc::new(texture(&p.image, p.source)?);
        self.textures.insert(key, Arc::clone(&texture));
        Some(texture)
    }

    /// Forget textures no placement uses any more and return them, so the
    /// caller can free them from the window's atlas.
    pub fn retain(&mut self, used: &HashSet<ImageKey>) -> Vec<Arc<RenderImage>> {
        let unused: Vec<ImageKey> = self
            .textures
            .keys()
            .filter(|k| !used.contains(k))
            .copied()
            .collect();
        unused
            .into_iter()
            .filter_map(|k| self.textures.remove(&k))
            .collect()
    }
}

/// The source rectangle of an RGBA image as a BGRA texture, the layout
/// GPUI's sprite atlas expects.
fn texture(image: &Image, (x, y, width, height): (u32, u32, u32, u32)) -> Option<RenderImage> {
    if width == 0
        || height == 0
        || x.checked_add(width)? > image.width
        || y.checked_add(height)? > image.height
    {
        return None;
    }
    let stride = image.width as usize * 4;
    let mut bgra = Vec::with_capacity(width as usize * height as usize * 4);
    for row in y..y + height {
        let start = row as usize * stride + x as usize * 4;
        let line = image.rgba.get(start..start + width as usize * 4)?;
        bgra.extend(
            line.as_chunks::<4>()
                .0
                .iter()
                .flat_map(|p| [p[2], p[1], p[0], p[3]]),
        );
    }
    let buffer = image::RgbaImage::from_raw(width, height, bgra)?;
    Some(RenderImage::new(vec![image::Frame::new(buffer)]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crops_the_source_rect_and_swaps_to_bgra() {
        // 2x2: red, green / blue, white.
        let image = Image {
            id: 1,
            generation: 1,
            width: 2,
            height: 2,
            rgba: vec![
                255, 0, 0, 255, 0, 255, 0, 255, //
                0, 0, 255, 255, 255, 255, 255, 128,
            ]
            .into(),
        };
        let t = texture(&image, (1, 0, 1, 2)).unwrap();
        assert_eq!(
            t.as_bytes(0).unwrap(),
            &[0, 255, 0, 255, 255, 255, 255, 128]
        );
        assert!(texture(&image, (1, 1, 2, 1)).is_none());
    }
}
