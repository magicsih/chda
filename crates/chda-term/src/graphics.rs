//! Images placed with the Kitty graphics protocol, as plain data.
//!
//! libghostty-vt parses the protocol and keeps images and placements; this
//! module turns them into [`ImagePlacement`]s for the frame and decodes PNG
//! payloads, which libghostty leaves to the embedder.

use std::fmt;
use std::io::Cursor;
use std::sync::Arc;

use libghostty_vt::alloc::{Allocator, Bytes};
use libghostty_vt::kitty::graphics::{DecodePng, DecodedImage, ImageFormat};

/// Image storage per terminal screen, as Ghostty's `image-storage-limit`
/// default.
pub(crate) const STORAGE_LIMIT: u64 = 320_000_000;

/// Decoded pixels of one stored image, 8-bit RGBA, row-major.
pub struct Image {
    pub id: u32,
    /// Changes whenever the image's pixels may have changed, so renderers
    /// can key texture caches on `(id, generation)`.
    pub generation: u64,
    pub width: u32,
    pub height: u32,
    pub rgba: Arc<[u8]>,
}

impl PartialEq for Image {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id && self.generation == other.generation
    }
}

impl Eq for Image {}

impl fmt::Debug for Image {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Image")
            .field("id", &self.id)
            .field("generation", &self.generation)
            .field("width", &self.width)
            .field("height", &self.height)
            .finish_non_exhaustive()
    }
}

/// Where an image is drawn relative to the cell contents.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ImageLayer {
    /// Under the cell backgrounds (`z < i32::MIN / 2`).
    BelowBackground,
    /// Over backgrounds, under text (negative `z`).
    BelowText,
    /// Over text (`z >= 0`).
    AboveText,
}

/// One visible placement of an image, in device pixels relative to the
/// viewport's cell grid. Placements come sorted by `z`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImagePlacement {
    pub image: Arc<Image>,
    /// Top-left cell; `row` is negative when the image has partly scrolled
    /// out above the viewport.
    pub col: i32,
    pub row: i32,
    /// Offset inside the top-left cell.
    pub x_offset: u32,
    pub y_offset: u32,
    /// Rendered size.
    pub width: u32,
    pub height: u32,
    /// The part of the image to draw, in image pixels.
    pub source: (u32, u32, u32, u32),
    pub layer: ImageLayer,
    pub z: i32,
}

impl ImageLayer {
    pub(crate) fn of(z: i32) -> Self {
        if z < i32::MIN / 2 {
            Self::BelowBackground
        } else if z < 0 {
            Self::BelowText
        } else {
            Self::AboveText
        }
    }
}

/// Convert stored pixel data to RGBA. `None` for data that does not match
/// the image size.
pub(crate) fn to_rgba(
    format: ImageFormat,
    data: &[u8],
    width: u32,
    height: u32,
) -> Option<Vec<u8>> {
    let pixels = usize::try_from(width).ok()? * usize::try_from(height).ok()?;
    let channels = match format {
        ImageFormat::Rgba | ImageFormat::Png => 4,
        ImageFormat::Rgb => 3,
        ImageFormat::GrayAlpha => 2,
        ImageFormat::Gray => 1,
        _ => return None,
    };
    let data = data.get(..pixels.checked_mul(channels)?)?;
    Some(match channels {
        4 => data.to_vec(),
        _ => data
            .chunks_exact(channels)
            .flat_map(|p| match p {
                [r, g, b] => [*r, *g, *b, 255],
                [v, a] => [*v, *v, *v, *a],
                [v] => [*v, *v, *v, 255],
                _ => unreachable!(),
            })
            .collect(),
    })
}

/// Decodes PNG payloads (`f=100`) to RGBA with the `png` crate.
pub(crate) struct PngDecoder;

/// Largest PNG accepted, in decoded bytes: one image may not take more
/// than the whole storage.
const MAX_DECODED: usize = STORAGE_LIMIT as usize;

impl DecodePng for PngDecoder {
    fn decode_png<'alloc>(
        &mut self,
        alloc: &'alloc Allocator<'_>,
        data: &[u8],
    ) -> Option<DecodedImage<'alloc>> {
        let (width, height, rgba) = decode_png(data)?;
        let mut bytes = Bytes::new_with_alloc(alloc, rgba.len()).ok()?;
        bytes.copy_from_slice(&rgba);
        Some(DecodedImage {
            width,
            height,
            data: bytes,
        })
    }
}

fn decode_png(data: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    let mut decoder =
        png::Decoder::new_with_limits(Cursor::new(data), png::Limits { bytes: MAX_DECODED });
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buf).ok()?;
    buf.truncate(info.buffer_size());
    let format = match info.color_type {
        png::ColorType::Rgba => ImageFormat::Rgba,
        png::ColorType::Rgb => ImageFormat::Rgb,
        png::ColorType::GrayscaleAlpha => ImageFormat::GrayAlpha,
        png::ColorType::Grayscale => ImageFormat::Gray,
        png::ColorType::Indexed => return None,
    };
    let rgba = to_rgba(format, &buf, info.width, info.height)?;
    Some((info.width, info.height, rgba))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layers_follow_kittys_z_ranges() {
        assert_eq!(ImageLayer::of(i32::MIN), ImageLayer::BelowBackground);
        assert_eq!(ImageLayer::of(-1), ImageLayer::BelowText);
        assert_eq!(ImageLayer::of(0), ImageLayer::AboveText);
    }

    #[test]
    fn pixel_formats_become_rgba() {
        assert_eq!(
            to_rgba(ImageFormat::Rgb, &[1, 2, 3, 4, 5, 6], 2, 1),
            Some(vec![1, 2, 3, 255, 4, 5, 6, 255])
        );
        assert_eq!(
            to_rgba(ImageFormat::GrayAlpha, &[7, 8], 1, 1),
            Some(vec![7, 7, 7, 8])
        );
        assert_eq!(
            to_rgba(ImageFormat::Gray, &[9], 1, 1),
            Some(vec![9, 9, 9, 255])
        );
        assert_eq!(to_rgba(ImageFormat::Rgb, &[1, 2], 1, 1), None);
    }

    #[test]
    fn decodes_png() {
        // 1x1 red pixel.
        let png = [
            0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48,
            0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00,
            0x00, 0x1f, 0x15, 0xc4, 0x89, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x44, 0x41, 0x54, 0x78,
            0x9c, 0x63, 0xf8, 0xcf, 0xc0, 0xf0, 0x1f, 0x00, 0x05, 0x00, 0x01, 0xff, 0x89, 0x99,
            0x3d, 0x1d, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
        ];
        assert_eq!(decode_png(&png), Some((1, 1, vec![255, 0, 0, 255])));
        assert_eq!(decode_png(b"not a png"), None);
    }
}
