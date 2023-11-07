use std::collections::BTreeMap;
use std::convert::TryFrom;
use std::io::{self, Write};

use image::{DynamicImage, GenericImageView, Pixel, Rgb, SubImage};
use log::debug;

use crate::encoder::crop_tile;
use crate::tile::Tile;
use crate::{max_size_in_rect, Vec2d};
use std::sync::Arc;

const BYTES_PER_PIXEL: usize = Rgb::<u8>::CHANNEL_COUNT as usize;

/// A structure to which you write tiles, not necessarily in order,
/// and that itself writes RGB pixels to its writer, ordered from top left to bottom right
pub struct PixelStreamer<W: Write> {
    strips: BTreeMap<usize, ImageStrip>,
    writer: W,
    size: Vec2d,
    current_index: usize,
}

impl<W: Write> PixelStreamer<W> {
    pub fn new(writer: W, size: Vec2d) -> Self {
        PixelStreamer {
            strips: BTreeMap::new(),
            writer,
            size,
            current_index: 0,
        }
    }

    pub fn add_tile(&mut self, tile: Tile) -> io::Result<()> {
        for strip in ImageStrip::in_tile(tile, self.size) {
            let key = strip.pixel_index(self.size);
            self.strips.insert(key, strip);
        }
        self.advance(false)
    }

    fn advance(&mut self, finalize: bool) -> io::Result<()> {
        while let Some(&start) = self.strips.keys().next() {
            if start <= self.current_index {
                let strip = self.strips.remove(&start).expect("The key should exist");
                let strip_size = strip.size(self.size);
                let start_strip_idx = self.current_index - start;
                // The strip may have already been written, in which case we just ignore it
                if start_strip_idx < strip_size {
                    strip.write_pixels(self.size, start_strip_idx, &mut self.writer)?;
                    debug!(
                        "Wrote a strip at position {} of size {}, skipping {} pixels",
                        self.current_index, strip_size, start_strip_idx
                    );
                    self.current_index += strip_size - start_strip_idx;
                }
            } else if finalize {
                // We are finalizing the image and missing data for a part of it
                self.fill_blank(start)?;
            } else {
                break;
            }
        }
        Ok(())
    }

    pub fn finalize(&mut self) -> io::Result<()> {
        self.advance(true)?;
        let image_size = (self.size.x as usize) * (self.size.y as usize);
        self.fill_blank(image_size)?;
        self.writer.flush()?;
        Ok(())
    }

    /// Write blank pixels until the given pixel index
    pub fn fill_blank(&mut self, until: usize) -> io::Result<()> {
        if until > self.current_index {
            let remaining = until - self.current_index;
            debug!("Filling incomplete image with {} pixels", remaining);
            let blank = vec![0; remaining * BYTES_PER_PIXEL];
            self.writer.write_all(&blank)?;
            self.current_index = until;
        }
        Ok(())
    }

    // https://github.com/image-rs/image-png/issues/307
    // pub fn into_writer(self) -> W { self.writer }
}

struct ImageStrip {
    source: Arc<Tile>,
    line: u32,
}

impl ImageStrip {
    pub fn in_tile(tile: Tile, canvas_size: Vec2d) -> impl Iterator<Item = ImageStrip> {
        let height = max_size_in_rect(tile.position, tile.size(), canvas_size).y;
        std::iter::successors(Some(Arc::new(tile)), |s| Some(Arc::clone(s)))
            .zip(0..height)
            .map(|(source, line)| ImageStrip { source, line })
    }
    pub fn pixel_index(&self, image_size: Vec2d) -> usize {
        let position = self.source.position + Vec2d { x: 0, y: self.line };
        (position.y as usize) * (image_size.x as usize) + (position.x as usize)
    }
    pub fn cropped(&self, image_size: Vec2d) -> SubImage<&DynamicImage> {
        crop_tile(&self.source, image_size)
    }
    /// Length of the strip in pixels
    pub fn size(&self, canvas_size: Vec2d) -> usize {
        max_size_in_rect(self.source.position, self.source.size(), canvas_size).x as usize
    }
    pub fn write_pixels<W: Write>(
        &self,
        image_size: Vec2d,
        start_at: usize,
        writer: &mut W,
    ) -> io::Result<()> {
        let img = self.cropped(image_size);
        let x0 = u32::try_from(start_at).unwrap();
        for x in x0..img.width() {
            let rgb: Rgb<u8> = img.get_pixel(x, self.line).to_rgb();
            writer.write_all(&rgb.0)?;
        }
        Ok(())
    }
}

#[allow(clippy::zero_prefixed_literal)]
#[cfg(test)]
mod tests {
    // In these tests, we consider a 4x4 image made from three tiles like so:
    //   0   1   2   3
    // +---+---+---+---+
    // |  Tile |       | 0
    // |   0   |  Tile | 1
    // +---+---+   1   | 2
    // |  Tile |       | 3
    // |   2   |       | 4
    // +---+---+---+---|
    // Tiles 0 and 2 are 2x2 and tile 1 is 2x4
    // Additionally, we add a Tile 3 that slightly overlaps tiles 0 and 1, with the same pixels
    use image::{DynamicImage, ImageBuffer};

    use super::*;

    fn tiles(i: usize) -> Tile {
        [
            Tile {
                position: Vec2d { x: 0, y: 0 },
                image: DynamicImage::ImageRgb8(
                    ImageBuffer::from_raw(
                        2,
                        2,
                        vec![
                            /* pixel 0,0 */ 1, 2, 3, /* pixel 1,0 */ 4, 5, 6,
                            /* pixel 0,1 */ 7, 8, 9, /* pixel 1,1 */ 10, 11, 12,
                        ],
                    )
                    .unwrap(),
                ),
            },
            Tile {
                position: Vec2d { x: 2, y: 0 },
                image: DynamicImage::ImageRgb8(
                    ImageBuffer::from_raw(
                        2,
                        4,
                        vec![
                            /* pixel 2,0 */ 00, 00, 00, /* pixel 3,0 */ 10, 10, 10,
                            /* pixel 2,1 */ 01, 01, 01, /* pixel 3,1 */ 11, 11, 11,
                            /* pixel 2,2 */ 02, 02, 02, /* pixel 3,2 */ 12, 12, 12,
                            /* pixel 2,3 */ 03, 03, 03, /* pixel 3,3 */ 13, 13, 13,
                        ],
                    )
                    .unwrap(),
                ),
            },
            Tile {
                position: Vec2d { x: 0, y: 2 },
                image: DynamicImage::ImageRgb8(
                    ImageBuffer::from_raw(
                        2,
                        2,
                        vec![
                            /* pixel 0,2 */ 100, 100, 100, /* pixel 1,2 */ 200, 200, 200,
                            /* pixel 0,3 */ 200, 200, 200, /* pixel 1,3 */ 99, 99, 99,
                        ],
                    )
                    .unwrap(),
                ),
            },
            Tile {
                position: Vec2d { x: 1, y: 0 },
                image: DynamicImage::ImageRgb8(
                    ImageBuffer::from_raw(
                        2,
                        1,
                        vec![
                            /* pixel 1,0 */ 4, 5, 6, /* pixel 2,0 */ 00, 00, 00,
                        ],
                    )
                    .unwrap(),
                ),
            },
        ][i]
            .clone()
    }

    const WHOLE_IMAGE: &[u8] = &[
        1, 2, 3, 4, 5, 6, /*             | */ 00, 00, 00, 10, 10, 10, 7, 8, 9, 10, 11, 12,
        /*          | */ 01, 01, 01, 11, 11, 11,
        /*-------------------------------+                              */
        100, 100, 100, 200, 200, 200, /* | */ 02, 02, 02, 12, 12, 12, 200, 200, 200, 99, 99,
        99, /*    | */ 03, 03, 03, 13, 13, 13,
    ];

    #[test]
    fn tile0() {
        assert_state_after_tiles(
            &[0], // Only the first line has been partially written
            vec![1, 2, 3, 4, 5, 6],
        );
    }

    #[test]
    fn tile1() {
        // Nothing has been written on the top left
        assert_state_after_tiles(&[1], vec![]);
    }

    #[test]
    fn tiles_0_and_1() {
        assert_state_after_tiles(
            &[0, 1], // The first two lines now are written (tile 1 and the upper part of tile 2)
            vec![
                1, 2, 3, 4, 5, 6, 00, 00, 00, 10, 10, 10, 7, 8, 9, 10, 11, 12, 01, 01, 01, 11, 11,
                11,
            ],
        );
    }

    #[test]
    fn all_tiles() {
        assert_state_after_tiles(
            &[0, 1, 2], // The whole image is written, in order
            Vec::from(WHOLE_IMAGE),
        );
    }

    #[test]
    fn all_tiles_non_sorted() {
        // The whole image is written, but not starting at the top left corner
        assert_state_after_tiles(&[1, 2, 0], Vec::from(WHOLE_IMAGE));
        assert_state_after_tiles(&[2, 1, 0], Vec::from(WHOLE_IMAGE));
    }

    #[test]
    fn all_tiles_overlapping_tiles() {
        // The same tile is written multiple times
        assert_state_after_tiles(&[0, 1, 0, 2], Vec::from(WHOLE_IMAGE));
        assert_state_after_tiles(&[0, 0, 1, 1, 2, 2], Vec::from(WHOLE_IMAGE));
        assert_state_after_tiles(&[2, 1, 2, 0], Vec::from(WHOLE_IMAGE));
        assert_state_after_tiles(&[0, 1, 3, 2], Vec::from(WHOLE_IMAGE));
        assert_state_after_tiles(&[0, 3, 1, 2], Vec::from(WHOLE_IMAGE));
        assert_state_after_tiles(&[3, 0, 1, 2], Vec::from(WHOLE_IMAGE));
        assert_state_after_tiles(&[0, 3, 0, 1, 2, 3], Vec::from(WHOLE_IMAGE));
    }

    fn assert_state_after_tiles(tile_indices: &[usize], expected: Vec<u8>) {
        let mut out = vec![];
        let mut streamer = PixelStreamer::new(&mut out, Vec2d { x: 4, y: 4 });
        for &i in tile_indices {
            streamer.add_tile(tiles(i)).unwrap();
        }
        assert_eq!(&out, &expected); // Only the first line has been partially written
    }

    #[test]
    fn finalize_empty() {
        let mut out = vec![];
        let mut streamer = PixelStreamer::new(&mut out, Vec2d { x: 2, y: 2 });
        streamer.finalize().unwrap();
        assert_eq!(
            &out,
            &[
                // No tile, the image is completely black
                0, 0, 0, /**/ 0, 0, 0, 0, 0, 0, /**/ 0, 0, 0,
            ]
        );
    }

    #[test]
    fn finalize_only_tile2() {
        let mut out = vec![];
        let mut streamer = PixelStreamer::new(&mut out, Vec2d { x: 2, y: 5 });
        streamer.add_tile(tiles(2)).unwrap();
        streamer.finalize().unwrap();
        assert_eq!(
            &out,
            &[
                // No tile, the image is completely black
                0, 0, 0, /**/ 0, 0, 0, 0, 0, 0, /**/ 0, 0, 0, /* pixel 0,0 */ 100,
                100, 100, /* pixel 1,0 */ 200, 200, 200, /* pixel 0,1 */ 200, 200, 200,
                /* pixel 1,1 */ 99, 99, 99, 0, 0, 0, 0, 0, 0
            ]
        );
    }

    #[test]
    fn tile_too_large() {
        let mut out = vec![];
        // Creating a 1x3 image and adding a 2x2 tile at position (0,2)
        // Since the tile doesn't fit, it must be cropped
        let mut streamer = PixelStreamer::new(&mut out, Vec2d { x: 1, y: 3 });
        streamer.add_tile(tiles(2)).unwrap();
        streamer.finalize().unwrap();
        assert_eq!(
            &out,
            &[
                // No tile, the image is completely black
                0, 0, 0, 0, 0, 0, 100, 100, 100,
            ]
        );
    }
}
