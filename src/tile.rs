use image::{DynamicImage, GenericImageView};

use crate::{Vec2d, ZoomError};
use crate::dezoomer::{PostProcessFn, TileReference};
use crate::errors::BufferToImageError;
use crate::network::fetch_uri;

#[derive(Clone)]
pub struct Tile {
    pub image: image::DynamicImage,
    pub position: Vec2d,
}

impl Tile {
    pub fn size(&self) -> Vec2d { self.image.dimensions().into() }
    pub fn bottom_right(&self) -> Vec2d {
        self.size() + self.position
    }
    pub async fn download(
        post_process_fn: PostProcessFn,
        tile_reference: &TileReference,
        client: &reqwest::Client,
    ) -> Result<Tile, ZoomError> {
        let bytes = fetch_uri(&tile_reference.url, client).await?;
        let tile_reference = tile_reference.clone();

        let tile: Result<Tile, BufferToImageError> = tokio::spawn(async move {
            tokio::task::block_in_place(move || {
                let transformed_bytes =
                    if let PostProcessFn::Fn(post_process) = post_process_fn {
                        post_process(&tile_reference, bytes)
                            .map_err(|e| BufferToImageError::PostProcessing { e })?
                    } else {
                        bytes
                    };

                Ok(Tile {
                    image: image::load_from_memory(&transformed_bytes)?,
                    position: tile_reference.position,
                })
            })
        }).await?;
        Ok(tile?)
    }
    pub fn empty(position: Vec2d, size: Vec2d) -> Tile {
        Tile { image: DynamicImage::new_rgba8(size.x, size.y), position }
    }
    pub fn position(&self) -> Vec2d {
        self.position
    }
}

impl std::fmt::Debug for Tile {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.debug_struct("Tile")
            .field("x", &self.position.x)
            .field("y", &self.position.y)
            .field("width", &self.image.width())
            .field("height", &self.image.height())
            .finish()
    }
}

impl PartialEq for Tile {
    fn eq(&self, other: &Self) -> bool {
        self.position == other.position &&
            self.size() == other.size() &&
            self.image.pixels().all(|(x, y, pix)| {
                other.image.get_pixel(x, y) == pix
            })
    }
}