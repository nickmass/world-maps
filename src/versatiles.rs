use super::{Tile, TileId};

use ahash::AHashMap as HashMap;
use anyhow::Result;
use byteorder::{BigEndian, ReadBytesExt};
use prost::Message;

use std::{
    fs::File,
    io::{BufReader, Read, Seek, SeekFrom},
    path::PathBuf,
    sync::Arc,
};

pub struct VersatilesSource {
    path: PathBuf,
    db: BufReader<File>,
    header: Arc<Header>,
    metadata: Arc<Option<Metadata>>,
    block_idx: Arc<BlockIndex>,
    decompression_buf: Vec<u8>,
}

impl VersatilesSource {
    pub fn new<P: Into<PathBuf>>(database: P) -> Result<Self> {
        let path = database.into();
        let mut db = BufReader::new(File::open(&path)?);
        let header = RawHeader::read(&mut db)?.validate()?;
        //println!("{:?}", header);
        let metadata = Metadata::read(&header, &mut db)?;
        //println!("{:?}", metadata);
        let block_idx = BlockIndex::read(&header, &mut db)?;

        Ok(Self {
            path,
            db,
            header: Arc::new(header),
            metadata: Arc::new(metadata),
            block_idx: Arc::new(block_idx),
            decompression_buf: Vec::new(),
        })
    }

    pub fn try_clone(&self) -> Result<Self> {
        let db = BufReader::new(File::open(&self.path)?);

        Ok(Self {
            path: self.path.clone(),
            db,
            header: self.header.clone(),
            metadata: self.metadata.clone(),
            block_idx: self.block_idx.clone(),
            decompression_buf: Vec::new(),
        })
    }

    pub fn query_tile(&mut self, tile_id: TileId) -> Option<Tile> {
        if !matches!(self.header.tile_format, TileFormat::Pbf) {
            return None;
        }

        let tile_id = TileId {
            zoom: tile_id.zoom,
            column: tile_id.column,
            row: (tile_id.limit() - tile_id.row) - 1,
        };

        let block_entry = self.block_idx.lookup(tile_id)?;
        let tile_index_offset = block_entry.block_offset + block_entry.tile_blob_len;
        self.db.seek(SeekFrom::Start(tile_index_offset)).ok()?;
        let mut tile_index_reader = self.db.by_ref().take(block_entry.tile_idx_len as u64);
        self.decompression_buf.clear();
        brotli::BrotliDecompress(&mut tile_index_reader, &mut self.decompression_buf).ok()?;

        let column = (tile_id.column % 256) as usize;
        let row = (tile_id.row % 256) as usize;
        let row_min = block_entry.row_min as usize;
        let _row_max = block_entry.row_max as usize;
        let col_min = block_entry.col_min as usize;
        let col_max = block_entry.col_max as usize;
        let tile_entry_idx = (row - row_min) * (col_max - col_min + 1) + (column - col_min);

        if self.decompression_buf.len() < (tile_entry_idx + 1) * 12 {
            return None;
        }

        let tile_entry_idx = tile_entry_idx * 12;
        let mut tile_entry = &self.decompression_buf[tile_entry_idx..(tile_entry_idx + 12)];
        let block_offset = tile_entry.read_u64::<BigEndian>().ok()?;
        let tile_len = tile_entry.read_u32::<BigEndian>().ok()?;

        if tile_len == 0 {
            return None;
        }

        self.db
            .seek(SeekFrom::Start(block_entry.block_offset + block_offset))
            .ok()?;
        let mut tile_reader = self.db.by_ref().take(tile_len as u64);
        self.decompression_buf.clear();

        match self.header.precompression_format {
            PrecompressionFormat::Uncompressed => {
                std::io::copy(&mut tile_reader, &mut self.decompression_buf).ok()?;
            }
            PrecompressionFormat::Gzip => {
                let mut decoder = libflate::gzip::Decoder::new(tile_reader).ok()?;
                decoder.read_to_end(&mut self.decompression_buf).ok()?;
            }
            PrecompressionFormat::Brotli => {
                brotli::BrotliDecompress(&mut tile_reader, &mut self.decompression_buf).ok()?;
            }
        };

        Some(Tile::decode(self.decompression_buf.as_slice()).unwrap())
    }
}

struct RawHeader {
    file_ident: [u8; 14],
    tile_format: u8,
    precompression_format: u8,
    min_zoom: u8,
    max_zoom: u8,
    bbox_min_x: i32,
    bbox_min_y: i32,
    bbox_max_x: i32,
    bbox_max_y: i32,
    metadata_offset: u64,
    metadata_len: u64,
    block_idx_offset: u64,
    block_idx_len: u64,
}

impl RawHeader {
    fn read<R: Read>(mut reader: R) -> std::io::Result<Self> {
        let mut file_ident = [0; 14];
        reader.read_exact(&mut file_ident)?;
        let tile_format = reader.read_u8()?;
        let precompression_format = reader.read_u8()?;
        let min_zoom = reader.read_u8()?;
        let max_zoom = reader.read_u8()?;
        let bbox_min_x = reader.read_i32::<BigEndian>()?;
        let bbox_min_y = reader.read_i32::<BigEndian>()?;
        let bbox_max_x = reader.read_i32::<BigEndian>()?;
        let bbox_max_y = reader.read_i32::<BigEndian>()?;
        let metadata_offset = reader.read_u64::<BigEndian>()?;
        let metadata_len = reader.read_u64::<BigEndian>()?;
        let block_idx_offset = reader.read_u64::<BigEndian>()?;
        let block_idx_len = reader.read_u64::<BigEndian>()?;

        Ok(Self {
            file_ident,
            tile_format,
            precompression_format,
            min_zoom,
            max_zoom,
            bbox_min_x,
            bbox_min_y,
            bbox_max_x,
            bbox_max_y,
            metadata_offset,
            metadata_len,
            block_idx_offset,
            block_idx_len,
        })
    }

    fn validate(self) -> Result<Header> {
        if &self.file_ident != b"versatiles_v02" {
            anyhow::bail!("Unexpected file ident");
        }

        let tile_format = self.tile_format.try_into()?;
        let precompression_format = self.precompression_format.try_into()?;

        Ok(Header {
            tile_format,
            precompression_format,
            min_zoom: self.min_zoom,
            max_zoom: self.max_zoom,
            bbox_min_x: self.bbox_min_x,
            bbox_min_y: self.bbox_min_y,
            bbox_max_x: self.bbox_max_x,
            bbox_max_y: self.bbox_max_y,
            metadata_offset: self.metadata_offset,
            metadata_len: self.metadata_len,
            block_idx_offset: self.block_idx_offset,
            block_idx_len: self.block_idx_len,
        })
    }
}

#[derive(Debug, Clone)]
#[allow(unused)]
struct Header {
    tile_format: TileFormat,
    precompression_format: PrecompressionFormat,
    min_zoom: u8,
    max_zoom: u8,
    bbox_min_x: i32,
    bbox_min_y: i32,
    bbox_max_x: i32,
    bbox_max_y: i32,
    metadata_offset: u64,
    metadata_len: u64,
    block_idx_offset: u64,
    block_idx_len: u64,
}

#[derive(Debug, Copy, Clone)]
enum TileFormat {
    Bin,
    Png,
    Jpg,
    Webp,
    Avif,
    Svg,
    Pbf,
    GeoJson,
    TopoJson,
    Json,
}

#[derive(Debug, Copy, Clone)]
struct UnexpectedTileFormatErr(u8);

impl std::error::Error for UnexpectedTileFormatErr {}

impl std::fmt::Display for UnexpectedTileFormatErr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Unexpected Tile Format: {:#2x}", self.0)
    }
}

impl TryFrom<u8> for TileFormat {
    type Error = UnexpectedTileFormatErr;

    fn try_from(value: u8) -> std::result::Result<Self, Self::Error> {
        let value = match value {
            0x00 => TileFormat::Bin,
            0x10 => TileFormat::Png,
            0x11 => TileFormat::Jpg,
            0x12 => TileFormat::Webp,
            0x13 => TileFormat::Avif,
            0x14 => TileFormat::Svg,
            0x20 => TileFormat::Pbf,
            0x21 => TileFormat::GeoJson,
            0x22 => TileFormat::TopoJson,
            0x23 => TileFormat::Json,
            _ => return Err(UnexpectedTileFormatErr(value)),
        };

        Ok(value)
    }
}

#[derive(Debug, Copy, Clone)]
enum PrecompressionFormat {
    Uncompressed,
    Gzip,
    Brotli,
}

#[derive(Debug, Copy, Clone)]
struct UnexpectedPrecompressionFormatErr(u8);

impl std::error::Error for UnexpectedPrecompressionFormatErr {}

impl std::fmt::Display for UnexpectedPrecompressionFormatErr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Unexpected Precompression Format: {:#2x}", self.0)
    }
}

impl TryFrom<u8> for PrecompressionFormat {
    type Error = UnexpectedPrecompressionFormatErr;

    fn try_from(value: u8) -> std::result::Result<Self, Self::Error> {
        let value = match value {
            0x00 => PrecompressionFormat::Uncompressed,
            0x01 => PrecompressionFormat::Gzip,
            0x02 => PrecompressionFormat::Brotli,
            _ => return Err(UnexpectedPrecompressionFormatErr(value)),
        };

        Ok(value)
    }
}

#[derive(Debug, Clone)]
struct Metadata(#[allow(unused)] serde_json::Value);

impl Metadata {
    fn read<R: Read + Seek>(header: &Header, mut db: R) -> Result<Option<Self>> {
        if header.metadata_len <= 2 || header.metadata_offset == 0 {
            return Ok(None);
        }
        db.seek(SeekFrom::Start(header.metadata_offset))?;
        let mut metadata_buf = vec![0; header.metadata_len as usize];
        db.read_exact(&mut metadata_buf)?;

        let metadata_buf = match header.precompression_format {
            PrecompressionFormat::Uncompressed => metadata_buf,
            PrecompressionFormat::Gzip => {
                let in_buf = std::io::Cursor::new(metadata_buf);
                let mut out_buf = Vec::new();
                let mut decoder = libflate::gzip::Decoder::new(in_buf)?;
                decoder.read_to_end(&mut out_buf)?;
                out_buf
            }
            PrecompressionFormat::Brotli => {
                let mut in_buf = std::io::Cursor::new(metadata_buf);
                let mut out_buf = Vec::new();
                brotli::BrotliDecompress(&mut in_buf, &mut out_buf)?;
                out_buf
            }
        };

        let metadata: serde_json::Value = serde_json::from_slice(&metadata_buf)?;

        Ok(Some(Metadata(metadata)))
    }
}

struct BlockIndex {
    index: HashMap<BlockId, BlockIdxEntry>,
}

impl BlockIndex {
    fn read<R: Read + Seek>(header: &Header, db: &mut R) -> Result<Self> {
        db.seek(SeekFrom::Start(header.block_idx_offset))?;
        let mut compressed_reader = db.take(header.block_idx_len);

        let mut block_idx_bytes = Vec::new();
        brotli::BrotliDecompress(&mut compressed_reader, &mut block_idx_bytes)?;

        let mut index = HashMap::with_capacity(block_idx_bytes.len() / 33);

        let mut reader = block_idx_bytes.as_slice();
        while !reader.is_empty() {
            let level = reader.read_u8()?;
            let column_base = reader.read_u32::<BigEndian>()?;
            let row_base = reader.read_u32::<BigEndian>()?;

            let col_min = reader.read_u8()?;
            let row_min = reader.read_u8()?;
            let col_max = reader.read_u8()?;
            let row_max = reader.read_u8()?;
            let block_offset = reader.read_u64::<BigEndian>()?;
            let tile_blob_len = reader.read_u64::<BigEndian>()?;
            let tile_idx_len = reader.read_u32::<BigEndian>()?;

            let entry = BlockIdxEntry {
                col_min,
                row_min,
                col_max,
                row_max,
                block_offset,
                tile_blob_len,
                tile_idx_len,
            };

            let key = BlockId {
                level,
                row_base,
                column_base,
            };

            index.insert(key, entry);
        }

        Ok(Self { index })
    }

    fn lookup(&self, tile: TileId) -> Option<&BlockIdxEntry> {
        let key = tile.into();
        self.index.get(&key)
    }
}

#[derive(Debug, Copy, Clone, Hash, PartialEq, Eq)]
struct BlockId {
    level: u8,
    row_base: u32,
    column_base: u32,
}

impl From<TileId> for BlockId {
    fn from(value: TileId) -> Self {
        let TileId { zoom, column, row } = value;
        let level = u8::try_from(zoom).unwrap();
        let column_base = column / 256;
        let row_base = row / 256;

        Self {
            level,
            row_base,
            column_base,
        }
    }
}

#[derive(Debug, Clone)]
struct BlockIdxEntry {
    col_min: u8,
    row_min: u8,
    col_max: u8,
    row_max: u8,
    block_offset: u64,
    tile_blob_len: u64,
    tile_idx_len: u32,
}
