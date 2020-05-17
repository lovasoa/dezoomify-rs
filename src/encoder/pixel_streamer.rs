use std::collections::BTreeMap;
use std::convert::TryFrom;
use std::io::Write;

use image::{Pixel, Rgb};

use crate::{Vec2d, ZoomError};
use crate::tile::Tile;

/// A structure to which you write tiles, not necessarily in order,
/// and that itself writes RGB pixels to its writer, ordered from top left to bottom right
pub struct PixelStreamer<W: Write> {
    strips: BTreeMap<usize, Vec<u8>>,
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

    pub fn add_tile(&mut self, tile: Tile) -> Result<(), ZoomError> {
        let rgb_image = tile.image.to_rgb();
        for (y, row) in rgb_image.enumerate_rows() {
            let position = tile.position + Vec2d { x: 0, y };
            let pixel_index = (position.y as usize) * (self.size.x as usize) + (position.x as usize);
            let key = pixel_index * usize::from(Rgb::<u8>::CHANNEL_COUNT);
            let value = row
                .flat_map(|(_x, _y, pixel)| pixel.channels().iter().copied())
                .collect();
            self.strips.insert(key, value);
        }
        self.advance(false)
    }

    fn advance(&mut self, finalize: bool) -> Result<(), ZoomError> {
        while let Some(&start) = self.strips.keys().next() {
            if start <= self.current_index {
                let values = self.strips.remove(&start).expect("The key should exist");
                let start_strip_idx = usize::try_from(self.current_index - start).unwrap();
                // The strip may have already been written, in which case we just ignore it
                if start_strip_idx < values.len() {
                    let to_write = &values[start_strip_idx..];
                    self.writer.write_all(to_write)?;
                    self.current_index += to_write.len();
                }
            } else if finalize {
                // We are finalizing the image and missing data for a part of it
                let missing = usize::try_from(start - self.current_index).unwrap();
                let blank = vec![0; missing];
                self.writer.write_all(&blank)?;
                self.current_index += missing;
            } else {
                break;
            }
        }
        Ok(())
    }

    pub fn finalize(&mut self) -> Result<(), ZoomError> {
        self.advance(true)?;
        let image_size = (self.size.x as usize) * (self.size.y as usize) *
            usize::from(Rgb::<u8>::CHANNEL_COUNT);
        if self.current_index < image_size {
            let remaining = image_size - self.current_index;
            let blank = vec![0; remaining];
            self.writer.write_all(&blank)?;
        }
        self.writer.flush()?;
        Ok(())
    }

    pub fn into_writer(self) -> W { self.writer }
}

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
    use image::{DynamicImage, ImageBuffer};

    use super::*;

    fn tiles(i: usize) -> Tile {
        [
            Tile {
                position: Vec2d { x: 0, y: 0 },
                image: DynamicImage::ImageRgb8(ImageBuffer::from_raw(2, 2, vec![
                    /* pixel 0,0 */ 1, 2, 3, /* pixel 1,0 */ 4, 5, 6,
                    /* pixel 0,1 */ 7, 8, 9, /* pixel 1,1 */ 10, 11, 12,
                ]).unwrap()),
            },
            Tile {
                position: Vec2d { x: 2, y: 0 },
                image: DynamicImage::ImageRgb8(ImageBuffer::from_raw(2, 4, vec![
                    /* pixel 0,0 */ 00, 00, 00, /* pixel 1,0 */ 10, 10, 10,
                    /* pixel 0,1 */ 01, 01, 01, /* pixel 1,1 */ 11, 11, 11,
                    /* pixel 0,2 */ 02, 02, 02, /* pixel 1,2 */ 12, 12, 12,
                    /* pixel 0,3 */ 03, 03, 03, /* pixel 1,3 */ 13, 13, 13,
                ]).unwrap()),
            },
            Tile {
                position: Vec2d { x: 0, y: 2 },
                image: DynamicImage::ImageRgb8(ImageBuffer::from_raw(2, 2, vec![
                    /* pixel 0,0 */ 100, 100, 100, /* pixel 1,0 */ 200, 200, 200,
                    /* pixel 0,1 */ 200, 200, 200, /* pixel 1,1 */ 99, 99, 99,
                ]).unwrap()),
            }
        ][i].clone()
    }

    const WHOLE_IMAGE: &[u8] = &[
        1, 2, 3, 4, 5, 6, /*             | */ 00, 00, 00, 10, 10, 10,
        7, 8, 9, 10, 11, 12, /*          | */ 01, 01, 01, 11, 11, 11,
        /*-------------------------------+                              */
        100, 100, 100, 200, 200, 200, /* | */ 02, 02, 02, 12, 12, 12,
        200, 200, 200, 99, 99, 99, /*    | */ 03, 03, 03, 13, 13, 13,
    ];

    #[test]
    fn test_pixel_streamer_tile0() {
        assert_state_after_tiles(
            &[0], // Only the first line has been partially written
            vec![1, 2, 3, 4, 5, 6],
        );
    }

    #[test]
    fn test_pixel_streamer_tile1() {
        // Nothing has been written on the top left
        assert_state_after_tiles(&[1], vec![]);
    }


    #[test]
    fn test_pixel_streamer_tiles_0_and_1() {
        assert_state_after_tiles(
            &[0, 1], // The first two lines now are written (tile 1 and the upper part of tile 2)
            vec![
                1, 2, 3, 4, 5, 6, 00, 00, 00, 10, 10, 10,
                7, 8, 9, 10, 11, 12, 01, 01, 01, 11, 11, 11
            ],
        );
    }

    #[test]
    fn test_pixel_streamer_all_tiles() {
        assert_state_after_tiles(
            &[0, 1, 2], // The whole image is written, in order
            Vec::from(WHOLE_IMAGE),
        );
    }

    #[test]
    fn test_pixel_streamer_all_tiles_non_sorted() {
        // The whole image is written, but not starting at the top left corner
        assert_state_after_tiles(&[1, 2, 0], Vec::from(WHOLE_IMAGE));
        assert_state_after_tiles(&[2, 1, 0], Vec::from(WHOLE_IMAGE));
    }

    #[test]
    fn test_pixel_streamer_all_tiles_overlapping_tiles() {
        // The same tile is written multiple times
        assert_state_after_tiles(&[0, 1, 0, 2], Vec::from(WHOLE_IMAGE));
        assert_state_after_tiles(&[0, 0, 1, 1, 2, 2], Vec::from(WHOLE_IMAGE));
        assert_state_after_tiles(&[2, 1, 2, 0], Vec::from(WHOLE_IMAGE));
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
    fn test_pixel_streamer_finalize_empty() {
        let mut out = vec![];
        let mut streamer = PixelStreamer::new(&mut out, Vec2d { x: 2, y: 2 });
        streamer.finalize().unwrap();
        assert_eq!(&out, &[ // No tile, the image is completely black
            0, 0, 0, /**/0, 0, 0,
            0, 0, 0, /**/0, 0, 0, ]
        );
    }

    #[test]
    fn test_pixel_streamer_finalize_only_tile2() {
        let mut out = vec![];
        let mut streamer = PixelStreamer::new(&mut out, Vec2d { x: 2, y: 5 });
        streamer.add_tile(tiles(2)).unwrap();
        streamer.finalize().unwrap();
        assert_eq!(&out, &[ // No tile, the image is completely black
            0, 0, 0, /**/0, 0, 0,
            0, 0, 0, /**/0, 0, 0,
            /* pixel 0,0 */ 100, 100, 100, /* pixel 1,0 */ 200, 200, 200,
            /* pixel 0,1 */ 200, 200, 200, /* pixel 1,1 */ 99, 99, 99,
            0, 0, 0, 0, 0, 0
        ]
        );
    }
}