use std::path::PathBuf;

/**
Used to receive tiles asynchronously and provide them to the encoder
*/
use log::debug;
use tokio::sync::mpsc;

use crate::encoder::{Encoder, SourceLevel, encoder_for_name};
use crate::tile::{EncodedTile, Tile};
use crate::{Vec2d, ZoomError};
use log::warn;

/// Data structure used to store tiles until the final image size is known
pub enum TileBuffer {
    Buffering {
        destination: PathBuf,
        buffer: Vec<Tile>,
        encoded_buffer: Vec<EncodedTile>,
        compression: u8,
    },
    Writing {
        destination: PathBuf,
        tile_sender: mpsc::Sender<TileBufferMsg>,
        error_receiver: mpsc::Receiver<std::io::Error>,
    },
}

impl TileBuffer {
    /// Create an encoder for an image of the given size at the path
    /// Errors out if the encoder cannot create files with the given extension
    /// or at the given size
    pub async fn new(destination: PathBuf, compression: u8) -> Result<Self, ZoomError> {
        Ok(TileBuffer::Buffering {
            destination,
            buffer: vec![],
            encoded_buffer: vec![],
            compression,
        })
    }

    pub async fn set_size(&mut self, size: Vec2d) -> Result<(), ZoomError> {
        let next_state = match self {
            TileBuffer::Buffering {
                buffer,
                encoded_buffer,
                destination,
                compression,
            } => {
                let destination = std::mem::take(destination);
                debug!("Creating a tile writer for an image of size {size}");
                let mut encoder = encoder_for_name(destination.clone(), size, *compression)?;
                debug!("Adding buffered tiles: {buffer:?}");
                for tile in buffer.drain(..) {
                    encoder.add_tile(tile)?;
                }
                for tile in encoded_buffer.drain(..) {
                    encoder.add_encoded_tile(tile)?;
                }
                buffer_tiles(encoder, destination).await
            }
            TileBuffer::Writing { .. } => {
                unreachable!("The size of the image can be set only once")
            }
        };
        *self = next_state;
        Ok(())
    }

    pub fn prefers_encoded_tiles(&self) -> bool {
        let extension = self.destination().extension().unwrap_or_default();
        extension == "tiff" || extension == "tif"
    }

    /// Start writing a source pyramid level.
    pub async fn begin_level(&mut self, level: SourceLevel) -> Result<(), ZoomError> {
        match self {
            TileBuffer::Buffering { .. } => {
                self.set_size(level.size).await?;
                if let TileBuffer::Writing { tile_sender, .. } = self {
                    tile_sender.send(TileBufferMsg::BeginLevel(level)).await?;
                    Ok(())
                } else {
                    unreachable!("set_size transitions buffering to writing")
                }
            }
            TileBuffer::Writing { tile_sender, .. } => {
                tile_sender.send(TileBufferMsg::BeginLevel(level)).await?;
                Ok(())
            }
        }
    }

    /// Add a tile to the image
    pub async fn add_tile(&mut self, tile: Tile) {
        match self {
            TileBuffer::Buffering { buffer, .. } => buffer.push(tile),
            TileBuffer::Writing { tile_sender, .. } => {
                tile_sender
                    .send(TileBufferMsg::AddTile(tile))
                    .await
                    .expect("The tile writer ended unexpectedly");
            }
        }
    }

    /// Add an encoded tile to the image without decoding it.
    pub async fn add_encoded_tile(&mut self, tile: EncodedTile) {
        match self {
            TileBuffer::Buffering { encoded_buffer, .. } => encoded_buffer.push(tile),
            TileBuffer::Writing { tile_sender, .. } => {
                tile_sender
                    .send(TileBufferMsg::AddEncodedTile(tile))
                    .await
                    .expect("The tile writer ended unexpectedly");
            }
        }
    }

    /// To be called when no more tile will be added
    pub async fn finalize(&mut self) -> Result<(), ZoomError> {
        if let TileBuffer::Buffering {
            buffer,
            encoded_buffer,
            ..
        } = self
        {
            let decoded_size = buffer
                .iter()
                .map(|t| t.position + t.size())
                .fold(Vec2d { x: 0, y: 0 }, Vec2d::max);
            let encoded_size = encoded_buffer
                .iter()
                .map(|t| t.position + t.size)
                .fold(Vec2d { x: 0, y: 0 }, Vec2d::max);
            let size = decoded_size.max(encoded_size);
            self.set_size(size).await?;
        }
        let (tile_sender, error_receiver) = match self {
            TileBuffer::Buffering { .. } => unreachable!("Just set the size"),
            TileBuffer::Writing {
                tile_sender,
                error_receiver,
                ..
            } => (tile_sender, error_receiver),
        };
        tile_sender.send(TileBufferMsg::Close).await?;
        debug!("Waiting for the image encoding task to finish");
        let mut result = Ok(());
        // Wait for the encoder to terminate even if some tiles raised errors
        while let Some(err) = error_receiver.recv().await {
            result = Err(err.into())
        }
        result
    }

    pub fn destination(&self) -> &PathBuf {
        match self {
            TileBuffer::Buffering { destination, .. } => destination,
            TileBuffer::Writing { destination, .. } => destination,
        }
    }
}

#[derive(Debug)]
pub enum TileBufferMsg {
    BeginLevel(SourceLevel),
    AddTile(Tile),
    AddEncodedTile(EncodedTile),
    Close,
}

async fn buffer_tiles(mut encoder: Box<dyn Encoder>, destination: PathBuf) -> TileBuffer {
    let (tile_sender, mut tile_receiver) = mpsc::channel(1024);
    let (error_sender, error_receiver) = mpsc::channel(1);
    tokio::spawn(async move {
        while let Some(msg) = tile_receiver.recv().await {
            match msg {
                TileBufferMsg::BeginLevel(level) => {
                    debug!("Starting output source level: {level:?}");
                    let result = tokio::task::block_in_place(|| encoder.begin_level(level));
                    if let Err(err) = result {
                        warn!("Error when starting output source level: {err}");
                        error_sender.send(err).await.expect("could not send error");
                    }
                }
                TileBufferMsg::AddTile(tile) => {
                    debug!("Sending tile to encoder: {tile:?}");
                    let result = tokio::task::block_in_place(|| encoder.add_tile(tile));
                    if let Err(err) = result {
                        warn!("Error when adding tile: {err}");
                        error_sender.send(err).await.expect("could not send error");
                    }
                }
                TileBufferMsg::AddEncodedTile(tile) => {
                    debug!("Sending encoded tile to encoder: {tile:?}");
                    let result = tokio::task::block_in_place(|| encoder.add_encoded_tile(tile));
                    if let Err(err) = result {
                        warn!("Error when adding encoded tile: {err}");
                        error_sender.send(err).await.expect("could not send error");
                    }
                }
                TileBufferMsg::Close => {
                    break;
                }
            }
        }
        debug!("Finalizing the encoder");
        if let Err(err) = encoder.finalize() {
            warn!("Error when finalizing image: {err}");
            error_sender.send(err).await.expect("could not send error");
        }
    });
    TileBuffer::Writing {
        tile_sender,
        error_receiver,
        destination,
    }
}
