use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
};

use crate::{
    TileId,
    mbtiles::MbTilesSource,
    proto::Tile,
    style::{SourceId, Style},
    versatiles::VersatilesSource,
};

use anyhow::Result;
use math::V2;

pub struct TileSourceCollection {
    names: Arc<HashMap<String, usize>>,
    sources: Vec<Option<TileSource>>,
}

impl TileSourceCollection {
    pub fn load<P: AsRef<Path>>(data_dir: P, style: &Style) -> Result<Self> {
        let mut names = HashMap::new();
        let mut sources = Vec::new();
        for (name, source) in style.sources.iter() {
            let mut tile_source = None;

            for uri in source.tiles.iter() {
                match TileSource::load(data_dir.as_ref(), uri) {
                    Ok(source) => {
                        tile_source = Some(source);
                        break;
                    }
                    Err(e) => {
                        eprintln!("unable to load tile source '{name}': {e}");
                    }
                }
            }

            if tile_source.is_none() {}

            names.insert(name.to_string(), sources.len());
            sources.push(tile_source);
        }

        Ok(Self {
            names: Arc::new(names),
            sources,
        })
    }

    pub fn try_clone(&self) -> Result<Self> {
        let mut sources = Vec::new();

        for source in self.sources.iter() {
            if let Some(s) = source {
                sources.push(Some(s.try_clone()?));
            } else {
                sources.push(None);
            }
        }

        Ok(Self {
            names: self.names.clone(),
            sources,
        })
    }

    pub fn query_tile(
        &mut self,
        source_id: &SourceId,
        mut tile_id: TileId,
    ) -> Option<(Tile, TileRect)> {
        let idx = match source_id {
            SourceId::Name(n) => *self.names.get(n)?,
            SourceId::Index(idx) => *idx,
        };

        let source = (*self.sources.get_mut(idx)?).as_mut()?;

        let mut rect_builder = TileRectBuilder::default();
        loop {
            let tile = source.query_tile(tile_id);

            let None = tile else {
                return tile.zip(Some(rect_builder.rect()));
            };

            tile_id = rect_builder.parent(tile_id)?;
        }
    }
}

pub enum TileSource {
    Versatiles(VersatilesSource),
    MbTiles(MbTilesSource),
}

impl TileSource {
    fn load<P: Into<PathBuf>>(data_dir: P, uri: &url::Url) -> Result<Self> {
        let res = match uri.scheme() {
            "versatiles" => {
                let mut path = data_dir.into();
                for seg in uri.path_segments().unwrap() {
                    path.push(seg);
                }

                TileSource::Versatiles(VersatilesSource::new(path)?)
            }
            "mbtiles" => {
                let mut path = data_dir.into();
                for seg in uri.path_segments().unwrap() {
                    path.push(seg);
                }

                TileSource::MbTiles(MbTilesSource::new(path)?)
            }
            scheme => {
                anyhow::bail!("unsupported tile source scheme: {scheme}")
            }
        };

        Ok(res)
    }

    fn try_clone(&self) -> Result<Self> {
        let res = match self {
            TileSource::Versatiles(source) => TileSource::Versatiles(source.try_clone()?),
            TileSource::MbTiles(source) => TileSource::MbTiles(source.try_clone()?),
        };

        Ok(res)
    }

    fn query_tile(&mut self, tile_id: TileId) -> Option<Tile> {
        match self {
            TileSource::Versatiles(versatiles_source) => versatiles_source.query_tile(tile_id),
            TileSource::MbTiles(mb_tiles_source) => mb_tiles_source.query_tile(tile_id),
        }
    }
}

#[derive(Debug, Copy, Clone)]
pub struct TileRectBuilder {
    column: u32,
    row: u32,
    parents: u16,
}

impl Default for TileRectBuilder {
    fn default() -> Self {
        Self {
            column: 0,
            row: 0,
            parents: 0,
        }
    }
}

impl TileRectBuilder {
    pub fn parent(&mut self, tile_id: TileId) -> Option<TileId> {
        if tile_id.zoom == 0 {
            return None;
        }

        self.parents += 1;
        self.column <<= 1;
        self.row <<= 1;
        self.column |= tile_id.column & 1;
        self.row |= tile_id.row & 1;

        tile_id.parent()
    }

    pub fn rect(mut self) -> TileRect {
        let mut offset = V2::new(0.0, 0.0);
        let mut scale = 1.0;

        while self.parents > 0 {
            scale *= 2.0;
            let dim = 1.0 / scale;

            let column_off = if self.column & 1 == 0 { 0.0 } else { dim };
            let row_off = if self.row & 1 != 0 { 0.0 } else { dim };

            offset += V2::new(column_off, row_off);
            self.column >>= 1;
            self.row >>= 1;
            self.parents -= 1;
        }

        TileRect { offset, scale }
    }
}

#[derive(Debug, Copy, Clone)]
pub struct TileRect {
    pub offset: V2<f32>,
    pub scale: f32,
}

impl Default for TileRect {
    fn default() -> Self {
        Self {
            offset: V2::zero(),
            scale: 1.0,
        }
    }
}

impl TileRect {
    pub fn offset(&self, point: V2<f32>) -> V2<f32> {
        (point - self.offset) * self.scale
    }
}
