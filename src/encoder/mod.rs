use std::path::Path;

use crate::{Vec2d, ZoomError};
use crate::tile::Tile;

mod canvas;

trait Encoder {
    /// Create an encoder for an image of the given size at the path
    /// Errors out if the encoder cannot create files with the given extension
    /// or at the given size
    fn new(destination: &Path, size: Vec2d) -> Result<Self, ZoomError>;
    /// Add a tile to the image
    fn add_tile(self: &mut Self, tile: &Tile) -> Result<(), ZoomError>;
    /// To be called when no more tile will be added
    fn finalize(self: &mut Self) -> Result<(), ZoomError>;
}