use fontdue::{Font, Metrics};

use std::{
    collections::HashMap,
    sync::{Arc, Mutex, RwLock},
};

use math::V2;

pub const TEXT_ATLAS_SIZE: u32 = 2048;

pub struct GlyphRender {
    pub atlas_contents: Arc<RwLock<HashMap<GlyphKey, AtlasEntry>>>,
    pub atlas_texture: Arc<wgpu::Texture>,
    pub atlas_size: V2<u32>,
    pub state: Arc<Mutex<GlyphRenderState>>,
    pub fonts: FontCollection,
    pub glyph_upload: Arc<RwLock<HashMap<GlyphKey, GlyphUploadEntry>>>,
}

pub struct GlyphRenderState {
    pub cursor: V2<u32>,
    pub row_height: u32,
}

impl Default for GlyphRenderState {
    fn default() -> Self {
        Self {
            cursor: V2::zero(),
            row_height: 0,
        }
    }
}

impl GlyphRender {
    pub fn prepare(&mut self, text_size: f32, glyph_id: GlyphId) -> bool {
        let glyph_key = glyph_id.with_size(text_size);

        if self.atlas_contents.read().unwrap().contains_key(&glyph_key) {
            return true;
        }

        {
            let mut upload = self.glyph_upload.write().unwrap();
            if upload.contains_key(&glyph_key) {
                return false;
            } else {
                upload.insert(glyph_key, GlyphUploadEntry::Pending);
            }
        }

        let GlyphId(font_id, glyph) = glyph_id;

        let font = self.fonts.font_id(font_id);
        let (metrics, bitmap) = font.rasterize(glyph, text_size);

        {
            let mut upload = self.glyph_upload.write().unwrap();
            upload.insert(glyph_key, GlyphUploadEntry::Prepared(metrics, bitmap));
        }

        false
    }
}

pub struct FontCollection {
    fonts: HashMap<FontId, Font>,
}

impl FontCollection {
    pub fn new() -> FontCollection {
        let fonts = HashMap::new();

        let mut fonts = Self { fonts };

        for i in 1..4 {
            let id = FontId(i);
            fonts.fonts.insert(
                id,
                Font::from_bytes(Self::font_data(id), Default::default()).unwrap(),
            );
        }

        fonts
    }

    pub fn font<I: IntoIterator<Item = S>, S: AsRef<str>>(&mut self, names: I) -> (FontId, &Font) {
        let mut font_id = None;

        for name in names {
            let name = name.as_ref();
            let found_face = match name {
                "Noto Sans" | "Noto Sans Regular" => 1,
                "Noto Sans Bold" => 2,
                "Noto Sans Italic" => 3,
                _ => continue,
            };

            font_id = Some(found_face);
            break;
        }

        let font_id = font_id.unwrap_or_else(|| {
            eprintln!("unable to find matching font face");
            1
        });

        let face = self.font_id(FontId(font_id));

        (FontId(font_id), face)
    }

    pub fn font_id(&mut self, font_id: FontId) -> &Font {
        self.fonts.get(&font_id).unwrap()
    }

    fn font_data(FontId(font_id): FontId) -> &'static [u8] {
        match font_id {
            1 => notosans::REGULAR_TTF,
            2 => notosans::BOLD_TTF,
            3 => notosans::ITALIC_TTF,
            _ => notosans::REGULAR_TTF,
        }
    }
}

pub enum GlyphUploadEntry {
    Pending,
    Prepared(Metrics, Vec<u8>),
}

pub struct AtlasEntry {
    pub offset: V2<u32>,
    pub dimensions: V2<u32>,
}

impl AtlasEntry {
    pub fn uv(&self) -> [V2<f32>; 4] {
        let offset = self.offset.as_f32();
        let dimensions = self.dimensions.as_f32();
        let atlas_dims = V2::fill(TEXT_ATLAS_SIZE).as_f32();

        let min = offset / atlas_dims;
        let max = (offset + dimensions) / atlas_dims;

        [V2::new(min.x, max.y), min, max, V2::new(max.x, min.y)]
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct FontId(u8);

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct GlyphId(pub FontId, pub char);

impl GlyphId {
    pub fn with_size(&self, size: f32) -> GlyphKey {
        GlyphKey(*self, size.floor() as i32)
    }

    pub fn _sdf(&self) -> GlyphKey {
        GlyphKey(*self, 0)
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct GlyphKey(GlyphId, i32);
