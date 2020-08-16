use std::collections::HashMap;
use std::convert::TryInto;
use std::io;
use std::path::PathBuf;
use std::sync::Arc;

use fixedbitset::FixedBitSet;
use image::DynamicImage;
use image::GenericImage;
use image::imageops::FilterType;
use log::debug;

use crate::{max_size_in_rect, Tile};
use crate::encoder::crop_tile;
use crate::errors::image_error_to_io_error;
use crate::Vec2d;

pub trait TileSaver {
    fn save_tile(&self, size: Vec2d, tile: Tile) -> io::Result<()>;
}

pub struct Retiler<T: TileSaver> {
    original_size: Vec2d,
    pub tile_size: Vec2d,
    scale_factor: u32,
    next_level: Option<Box<Retiler<T>>>,
    tiles: HashMap<Vec2d, TmpTile>,
    tile_saver: Arc<T>,
}

struct TmpTile {
    done_pixels: FixedBitSet,
}

impl<T: TileSaver> Retiler<T> {
    pub fn new(size: Vec2d, tile_size: Vec2d, tile_saver: Arc<T>, scale_factor: u32) -> Retiler<T> {
        let next_scale_factor = scale_factor * 2;
        let next_level =
            if tile_size.fits_inside(size / next_scale_factor) {
                let tile_saver = Arc::clone(&tile_saver);
                let level = Retiler::new(size, tile_size, tile_saver, next_scale_factor);
                Some(Box::new(level))
            } else { None };
        Retiler {
            original_size: size,
            tile_size: tile_size * scale_factor,
            next_level,
            tiles: HashMap::new(),
            tile_saver,
            scale_factor,
        }
    }

    pub fn size(&self) -> Vec2d {
        self.original_size / self.scale_factor
    }

    fn tile_positions(&self, position: Vec2d, size: Vec2d) -> impl Iterator<Item=Vec2d> {
        let top_left = (position / self.tile_size) * self.tile_size;
        let bottom_right = ((position + size).ceil_div(self.tile_size)) * self.tile_size;
        let dy = self.tile_size.y as usize;
        let dx = self.tile_size.x as usize;
        (top_left.y..bottom_right.y)
            .step_by(dy)
            .flat_map(move |y|
                (top_left.x..bottom_right.x)
                    .step_by(dx)
                    .map(move |x|
                        Vec2d { x, y }
                    )
            )
    }

    pub fn add_tile(&mut self, tile: Tile) -> io::Result<()> {
        let tile_size = self.tile_size;
        let scale_factor = self.scale_factor;
        let scaled_size = tile.size().ceil_div(scale_factor);
        let scaled_tile = Tile {
            position: tile.position / scale_factor,
            image: tile.image.resize_exact(scaled_size.x, scaled_size.y, FilterType::Gaussian),
        };
        for cur_pos in self.tile_positions(tile.position, tile.size()) {
            let cur_tile_size = max_size_in_rect(cur_pos, tile_size, self.original_size);
            let scaled_tile_size = cur_tile_size.ceil_div(scale_factor);

            let tmp_tile = self.tiles.entry(cur_pos)
                .or_insert_with(|| {
                    debug!("Creating a new partial tile at scale factor {} position {} size {}", scale_factor, cur_pos, cur_tile_size);
                    TmpTile::new(scaled_tile_size)
                });
            let finished = tmp_tile.add_tile(
                cur_pos,
                cur_tile_size,
                self.original_size,
                scale_factor,
                &scaled_tile)?;
            if let Some(tile_img) = finished {
                self.tile_save(cur_pos, cur_tile_size, tile_img)?;
                self.tiles.remove(&cur_pos);
            }
        }


        if let Some(next_level) = &mut self.next_level {
            next_level.add_tile(tile)?;
        }
        Ok(())
    }

    pub fn tile_save(&self, position: Vec2d, size: Vec2d, image: DynamicImage) -> io::Result<()> {
        self.tile_saver.save_tile(size, Tile { position, image })
    }

    pub fn level_count(&self) -> u32 {
        1 + self.next_level.as_ref()
            .map(|l| l.level_count())
            .unwrap_or(0)
    }
}

impl TmpTile {
    fn new(size: Vec2d) -> TmpTile {
        let bits = size.area().try_into().expect("Tile size too large");
        TmpTile {
            done_pixels: FixedBitSet::with_capacity(bits)
        }
    }

    fn set_done_pixels(&mut self, self_size: Vec2d, top_left: Vec2d, bottom_right: Vec2d) {
        for y in top_left.y..bottom_right.y {
            let start = (y * self_size.x + top_left.x) as usize;
            let end = (y * self_size.x + bottom_right.x) as usize;
            self.done_pixels.insert_range(start..end);
        }
    }

    fn add_tile(
        &mut self,
        self_position: Vec2d,
        self_size: Vec2d,
        level_size: Vec2d,
        scale_factor: u32,
        tile: &Tile,
    )
        -> io::Result<Option<DynamicImage>> {
        let scaled_self_position = self_position / scale_factor;
        let top_left = tile.position() - scaled_self_position;
        let scaled_level_size = level_size.ceil_div(scale_factor);
        let bottom_right = tile.bottom_right().min(scaled_level_size) - scaled_self_position;
        let scaled_size = self_size.ceil_div(scale_factor);

        self.set_done_pixels(scaled_size, top_left, bottom_right);
        let tmp_tile_path = Self::path(self_position, scale_factor);
        debug!("Opening partial tile of size {} at {:?}", scaled_size, &tmp_tile_path);
        let mut tile_img = image::open(&tmp_tile_path)
            .unwrap_or_else(|_| image::DynamicImage::new_rgb8(scaled_size.x, scaled_size.y));
        let sub_tile_img = crop_tile(
            &tile,
            (self_position + self_size).ceil_div(scale_factor));
        tile_img.copy_from(&sub_tile_img, top_left.x, top_left.y).map_err(|_err| {
            io::Error::new(io::ErrorKind::InvalidData, "tile too large for image")
        })?;

        if self.done_pixels.count_ones(..) == scaled_size.area() as usize {
            // The tile has been fully covered by pixels
            debug!("Removing completed tile of level {} at position {}: {:?}", level_size, self_position, &tmp_tile_path);
            let _ = std::fs::remove_file(&tmp_tile_path);
            Ok(Some(tile_img))
        } else {
            debug!("Writing partly-filled tile of level {} at position {}", level_size, self_position);
            tile_img.save(&tmp_tile_path).map_err(image_error_to_io_error)?;
            Ok(None)
        }
    }

    fn path(position: Vec2d, scale_factor: u32) -> PathBuf {
        let pid = std::process::id();
        let mut path = std::env::temp_dir();
        path.push(format!("dezoomify_{}_level_{}_position_{}x{}.bmp",
                          pid,
                          scale_factor,
                          position.x, position.y));
        path
    }
}