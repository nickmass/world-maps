use std::collections::HashMap;

use serde::Deserialize;

#[derive(Debug, Clone)]
pub struct SourceCollection {
    sources: Vec<Source>,
    names: Vec<String>,
}

impl<'de> serde::Deserialize<'de> for SourceCollection {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let source_map = HashMap::<String, Source>::deserialize(deserializer)?;
        let mut sources = Vec::new();
        let mut names = Vec::new();

        for (name, source) in source_map {
            names.push(name);
            sources.push(source);
        }

        Ok(Self { sources, names })
    }
}

impl SourceCollection {
    pub fn get(&self, index: &SourceId) -> Option<&Source> {
        let idx = match index {
            SourceId::Name(n) => self.names.iter().position(|name| n == name)?,
            SourceId::Index(idx) => *idx,
        };

        self.sources.get(idx)
    }

    pub fn remap(&self, index: SourceId) -> Option<SourceId> {
        match index {
            SourceId::Name(n) => self
                .names
                .iter()
                .position(|name| &n == name)
                .map(|idx| SourceId::Index(idx)),
            SourceId::Index(idx) => (self.sources.len() > idx).then_some(index),
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &'_ Source)> {
        self.names
            .iter()
            .map(String::as_str)
            .zip(self.sources.iter())
    }
}

#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "kebab-case")]
pub struct Source {
    #[serde(rename = "type")]
    pub kind: SourceType,
    #[serde(default)]
    pub tiles: Vec<url::Url>,
    pub attribution: Option<String>,
}

#[derive(Deserialize, Debug, Copy, Clone, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum SourceType {
    Vector,
    Raster,
    RasterDem,
    Geojson,
    Video,
    Image,
}

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub enum SourceId {
    Name(String),
    Index(usize),
}

impl<'de> serde::Deserialize<'de> for SourceId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let name = String::deserialize(deserializer)?;
        Ok(SourceId::Name(name))
    }
}
