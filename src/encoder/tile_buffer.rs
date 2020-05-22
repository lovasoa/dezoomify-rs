/**
Used to receive tiles asynchronously and provide them to the encoder
*/
use log::debug;
use tokio::sync::mpsc;
use crate::{Vec2d, ZoomError};
use std::path::PathBuf;
use crate::tile::Tile;
use crate::encoder::{encoder_for_name, Encoder};


/// Data structure used to store tiles until the final image size is known
pub enum TileBuffer {
    Buffering { destination: PathBuf, buffer: Vec<Tile> },
    Writing {
        tile_sender: mpsc::Sender<TileBufferMsg>,
        error_receiver: mpsc::Receiver<std::io::Error>,
    },
}

impl TileBuffer {
    /// Create an encoder for an image of the given size at the path
    /// Errors out if the encoder cannot create files with the given extension
    /// or at the given size
    pub async fn new(destination: PathBuf) -> Result<Self, ZoomError> {
        Ok(TileBuffer::Buffering {
            destination,
            buffer: vec![],
        })
    }

    pub async fn set_size(&mut self, size: Vec2d) -> Result<(), ZoomError> {
        let next_state = match self {
            TileBuffer::Buffering { buffer, destination } => {
                debug!("Creating a tile writer for an image of size {}", size);
                let mut e = encoder_for_name(destination.clone(), size)?;
                debug!("Adding buffered tiles: {:?}", buffer);
                for tile in buffer.drain(..) { e.add_tile(tile)?; }
                buffer_tiles(e).await
            }
            TileBuffer::Writing { .. } => unreachable!("The size of the image can be set only once")
        };
        std::mem::replace(self, next_state);
        Ok(())
    }

    /// Add a tile to the image
    pub async fn add_tile(&mut self, tile: Tile) -> Result<(), ZoomError> {
        match self {
            TileBuffer::Buffering { buffer, .. } => {
                buffer.push(tile)
            }
            TileBuffer::Writing { tile_sender, error_receiver } => {
                if let Ok(e) = error_receiver.try_recv() { return Err(e.into()) }
                tile_sender.send(TileBufferMsg::AddTile(tile)).await?;
            }
        }
        Ok(())
    }

    /// To be called when no more tile will be added
    pub async fn finalize(&mut self) -> Result<(), ZoomError> {
        if let TileBuffer::Buffering { buffer, .. } = self {
            let size = buffer.iter().map(|t| t.position + t.size()).fold(
                Vec2d { x: 0, y: 0 },
                Vec2d::max,
            );
            self.set_size(size).await?;
        }
        let (tile_sender, error_receiver) = match self {
            TileBuffer::Buffering { .. } => unreachable!("Just set the size"),
            TileBuffer::Writing { tile_sender, error_receiver } => (tile_sender, error_receiver)
        };
        tile_sender.send(TileBufferMsg::Close).await?;
        debug!("Waiting for the image encoding task to finish");
        if let Some(err) = error_receiver.recv().await { return Err(err.into()) }
        Ok(())
    }
}

#[derive(Debug)]
pub enum TileBufferMsg {
    AddTile(Tile),
    Close,
}

async fn buffer_tiles(mut encoder: Box<dyn Encoder>) -> TileBuffer {
    let (tile_sender, mut tile_receiver) = mpsc::channel(1024);
    let (mut error_sender, error_receiver) = mpsc::channel(1);
    tokio::spawn(async move {
        while let Some(msg) = tile_receiver.recv().await {
            match msg {
                TileBufferMsg::AddTile(tile) => {
                    debug!("Sending tile to encoder: {:?}", tile);
                    let result = tokio::task::block_in_place(|| encoder.add_tile(tile));
                    if let Err(err) = result {
                        error_sender.send(err).await.expect("could not send error");
                    }
                }
                TileBufferMsg::Close => { break; }
            }
        }
        debug!("Finalizing the encoder");
        if let Err(err) = encoder.finalize() {
            error_sender.send(err).await.expect("could not send error");
        }
    });
    TileBuffer::Writing {
        tile_sender,
        error_receiver,
    }
}