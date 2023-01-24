use fontdue::Font;

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
    pub fn prepare(&mut self, queue: &wgpu::Queue, text_size: f32, glyph_id: GlyphId) {
        if self
            .atlas_contents
            .read()
            .unwrap()
            .contains_key(&glyph_id.with_size(text_size))
            || glyph_id.1 == ' '
        {
            return;
        }

        let mut atlas = self.atlas_contents.write().unwrap();

        let mut state = self.state.lock().unwrap();

        let GlyphId(font_id, glyph) = glyph_id;

        let font = self.fonts.font_id(font_id);
        let (metrics, bitmap) = font.rasterize(glyph, text_size);
        let width = metrics.width as u32;
        let height = metrics.height as u32;

        if state.cursor.x + width >= self.atlas_size.x {
            state.cursor.y += state.row_height + 1;
            state.cursor.x = 0;
            state.row_height = 0;
        }

        state.row_height = state.row_height.max(height);

        if state.cursor.y + height >= self.atlas_size.y {
            eprintln!("ATLAS FULL");
            self.atlas_contents.write().unwrap().clear();
            state.cursor = V2::zero();
            return;
        }

        if let Some(bytes) = std::num::NonZeroU32::new(width) {
            queue.write_texture(
                wgpu::ImageCopyTexture {
                    texture: &self.atlas_texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d {
                        x: state.cursor.x,
                        y: state.cursor.y,
                        z: 0,
                    },
                    aspect: wgpu::TextureAspect::All,
                },
                bitmap.as_slice(),
                wgpu::ImageDataLayout {
                    offset: 0,
                    bytes_per_row: Some(bytes),
                    rows_per_image: None,
                },
                wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
            );

            let entry = AtlasEntry {
                offset: state.cursor,
                dimensions: V2::new(width, height),
            };

            atlas.insert(glyph_id.with_size(text_size), entry);

            state.cursor.x += width + 1;
        } else {
            eprintln!("err");
        }
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

pub struct AtlasEntry {
    offset: V2<u32>,
    dimensions: V2<u32>,
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
