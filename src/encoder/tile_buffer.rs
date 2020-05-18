/**
Used to receive tiles asynchronously and provide them to the encoder
*/
use tokio::sync::mpsc;
use crate::{Vec2d, ZoomError};
use std::path::PathBuf;
use crate::tile::Tile;
use crate::encoder::{encoder_for_name, Encoder};


/// Data structure used to store tiles until the final image size is known
pub enum TileBuffer {
    Buffer { destination: PathBuf, buffer: Vec<Tile> },
    Sender(mpsc::Sender<TileBufferMsg>),
}

impl TileBuffer {
    /// Create an encoder for an image of the given size at the path
    /// Errors out if the encoder cannot create files with the given extension
    /// or at the given size
    pub async fn new(destination: PathBuf) -> Result<Self, ZoomError> {
        Ok(TileBuffer::Buffer {
            destination,
            buffer: vec![],
        })
    }

    pub async fn set_size(&mut self, size: Vec2d) -> Result<(), ZoomError> {
        let sender = match self {
            TileBuffer::Buffer { buffer, destination } => {
                let mut e = encoder_for_name(destination.clone(), size)?;
                for tile in buffer.drain(..) { e.add_tile(tile)?; }
                let sender = buffer_tiles(e).await;
                sender
            }
            TileBuffer::Sender(..) => unreachable!("The size of the image can be set only once")
        };
        std::mem::replace(self, TileBuffer::Sender(sender));
        Ok(())
    }

    /// Add a tile to the image
    pub async fn add_tile(&mut self, tile: Tile) -> Result<(), ZoomError> {
        match self {
            TileBuffer::Buffer { buffer, .. } => {
                buffer.push(tile)
            }
            TileBuffer::Sender(s) => {
                s.send(TileBufferMsg::AddTile(tile)).await?;
            }
        }
        Ok(())
    }

    /// To be called when no more tile will be added
    pub async fn finalize(&mut self) -> Result<(), ZoomError> {
        if let TileBuffer::Buffer { buffer, .. } = self {
            let size = buffer.iter().map(|t| t.position + t.size()).fold(
                Vec2d { x: 0, y: 0 },
                Vec2d::max,
            );
            self.set_size(size).await?;
        }
        let sender = match self {
            TileBuffer::Buffer { .. } => unreachable!("Just set the size"),
            TileBuffer::Sender(s) => s
        };
        sender.send(TileBufferMsg::Close).await?;
        Ok(())
    }
}

#[derive(Debug)]
pub enum TileBufferMsg {
    AddTile(Tile),
    Close,
}

async fn buffer_tiles(mut encoder: Box<dyn Encoder>) -> mpsc::Sender<TileBufferMsg> {
    let (sender, mut receiver) = mpsc::channel(128);
    tokio::spawn(async move {
        while let Some(msg) = receiver.recv().await {
            match msg {
                TileBufferMsg::AddTile(tile) => {
                    encoder.add_tile(tile).expect("Failed to add tile");
                }
                TileBufferMsg::Close => { break; }
            }
        }
        encoder.finalize().expect("Unable to finalize the image")
    });
    sender
}