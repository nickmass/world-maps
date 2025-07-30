use std::{io::Read, path::PathBuf};

use anyhow::Result;
use libflate::gzip;
use prost::Message;
use rusqlite::{Connection, OptionalExtension};

use crate::{Tile, TileId};

pub struct MbTilesSource {
    path: PathBuf,
    connection: Connection,
    decompress_buf: Vec<u8>,
}

impl MbTilesSource {
    pub fn new<P: Into<PathBuf>>(database: P) -> Result<Self> {
        let path = database.into();
        let connection =
            Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;

        Ok(Self {
            path,
            connection,
            decompress_buf: Vec::new(),
        })
    }

    pub fn query_tile(&mut self, tile: TileId) -> Option<Tile> {
        let mut query = self.connection
        .prepare_cached(
            "SELECT tile_data FROM tiles WHERE zoom_level = ?1 AND tile_column = ?2 AND tile_row = ?3",
        )
        .unwrap();

        query
            .query_row((tile.zoom, tile.column, tile.row), |row| {
                let compressed_bytes = row.get_ref(0)?.as_blob()?;

                let mut decoder = gzip::Decoder::new(compressed_bytes).unwrap();
                self.decompress_buf.clear();
                decoder.read_to_end(&mut self.decompress_buf).unwrap();

                // Changed protobuf def. to use `bytes` instead of `string` for labels, to avoid some non utf-8 data
                let tile = Tile::decode(self.decompress_buf.as_slice()).unwrap();
                Ok(tile)
            })
            // Should return parent tile if tile not found - mbtiles de-dedupe method when tiles are identical to parent
            .optional()
            .unwrap()
    }

    pub fn try_clone(&self) -> Result<Self> {
        let connection =
            Connection::open_with_flags(&self.path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;

        Ok(Self {
            path: self.path.clone(),
            connection,
            decompress_buf: Vec::new(),
        })
    }
}
