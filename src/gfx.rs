use wgpu::util::DeviceExt;
use winit::window::Window;

use std::{
    collections::HashMap,
    sync::{Arc, Mutex, RwLock},
};

use math::{Rect, V2, V4};

use crate::text::{
    AtlasEntry, GlyphKey, GlyphRender, GlyphRenderState, GlyphUploadEntry, TEXT_ATLAS_SIZE,
};
use crate::{FeatureDraw, LayerLabelDraw, RectExt, TileId};

pub const TILE_WGSL: &'static str = include_str!("../shaders/tile.wgsl");
pub const TEXT_WGSL: &'static str = include_str!("../shaders/text.wgsl");
pub const PUSH_CONSTANT_LIMIT: usize = 128;

pub struct Gfx {
    window: &'static Window,
    surface: wgpu::Surface<'static>,
    device: Arc<wgpu::Device>,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    render_pipeline: wgpu::RenderPipeline,
    multisampled_framebuffer: wgpu::TextureView,
    instance: wgpu::Instance,
    size: V2<u32>,
    tile_cache: GpuTileCache<TileGeometry>,
    samples: u32,
    glyph_pipeline: GlyphPipeline,
    tile_size: V2<f32>,
}

impl Gfx {
    pub fn new(window: &'static Window, tile_size: u32) -> Self {
        let size = window.inner_size();
        let size = V2::new(size.width, size.height);
        let tile_size = V2::fill(tile_size).as_f32();
        let samples = 4;

        let instance_desc = wgpu::InstanceDescriptor::default();

        let instance = wgpu::Instance::new(instance_desc);
        let surface = instance.create_surface(window).unwrap();
        let adapter = instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
        });
        let adapter = pollster::block_on(adapter).unwrap();

        let device_request = adapter.request_device(
            &wgpu::DeviceDescriptor {
                required_features: wgpu::Features::PUSH_CONSTANTS,
                required_limits: wgpu::Limits {
                    max_push_constant_size: PUSH_CONSTANT_LIMIT as u32,
                    ..Default::default()
                },
                label: Some("device"),
            },
            None,
        );

        let (device, queue) = pollster::block_on(device_request).unwrap();

        let mut config = surface
            .get_default_config(&adapter, size.x, size.y)
            .unwrap();
        config.present_mode = wgpu::PresentMode::AutoVsync;

        surface.configure(&device, &config);

        let glyph_renderer = GlyphPipeline::new(&device, &config, samples);

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("tile-shader"),
            source: wgpu::ShaderSource::Wgsl(TILE_WGSL.into()),
        });

        let render_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("tile-pipeline-layout"),
                bind_group_layouts: &[],
                push_constant_ranges: &[wgpu::PushConstantRange {
                    stages: wgpu::ShaderStages::VERTEX,
                    range: 0..PUSH_CONSTANT_LIMIT as u32,
                }],
            });

        let render_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("tile-pipeline"),
            layout: Some(&render_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: "vs_main",
                buffers: &[GeoVertex::desc()],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: "fs_main",
                targets: &[Some(wgpu::ColorTargetState {
                    format: config.format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                polygon_mode: wgpu::PolygonMode::Fill,
                unclipped_depth: false,
                conservative: false,
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState {
                count: samples,
                mask: !0,
                alpha_to_coverage_enabled: false,
            },
            multiview: None,
        });

        let multisampled_framebuffer =
            Self::create_multisampled_framebuffer(&device, &config, samples);

        Self {
            window,
            instance,
            surface,
            device: Arc::new(device),
            queue,
            config,
            render_pipeline,
            multisampled_framebuffer,
            size,
            tile_cache: GpuTileCache::new(),
            samples,
            glyph_pipeline: glyph_renderer,
            tile_size,
        }
    }

    fn create_multisampled_framebuffer(
        device: &wgpu::Device,
        config: &wgpu::SurfaceConfiguration,
        sample_count: u32,
    ) -> wgpu::TextureView {
        let multisampled_texture_extent = wgpu::Extent3d {
            width: config.width,
            height: config.height,
            depth_or_array_layers: 1,
        };
        let view_formats = vec![config.format];
        let multisampled_frame_descriptor = wgpu::TextureDescriptor {
            size: multisampled_texture_extent,
            mip_level_count: 1,
            sample_count,
            dimension: wgpu::TextureDimension::D2,
            format: config.format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            label: None,
            view_formats: &view_formats,
        };

        device
            .create_texture(&multisampled_frame_descriptor)
            .create_view(&wgpu::TextureViewDescriptor::default())
    }

    pub fn handle(&self) -> GfxHandle {
        GfxHandle {
            device: self.device.clone(),
            glyph_render: self.glyph_pipeline.glyph_render(),
        }
    }

    pub fn store_tile(&mut self, tile_geo: TileGeometry) {
        if self.tile_cache.contains(tile_geo.tile_id) {
            return;
        }

        if !tile_geo.text.complete {
            return;
        }

        self.tile_cache.insert(tile_geo.tile_id, tile_geo);
    }

    pub fn resize(&mut self, size: V2<u32>) {
        if size.x > 0 && size.y > 0 {
            self.size = size;
            self.config.width = self.size.x;
            self.config.height = self.size.y;
            self.surface.configure(&self.device, &self.config);
            self.multisampled_framebuffer =
                Self::create_multisampled_framebuffer(&self.device, &self.config, self.samples);
        }
    }

    pub fn reconfigure(&mut self) {
        self.surface = self.instance.create_surface(self.window).unwrap();
        self.surface.configure(&self.device, &self.config);
        self.multisampled_framebuffer =
            Self::create_multisampled_framebuffer(&self.device, &self.config, self.samples);
    }

    pub fn has_tile(&self, tile_id: TileId) -> bool {
        self.tile_cache.contains(tile_id)
    }

    pub fn render<I: IntoIterator<Item = (TileId, Rect<i32>)> + Clone>(
        &self,
        tiles: I,
        zoom: f32,
        scale: f32,
    ) -> Result<(), wgpu::SurfaceError> {
        let output = self.surface.get_current_texture()?;
        let view = output
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("encoder"),
            });

        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("tile-render-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.multisampled_framebuffer,
                    resolve_target: Some(&view),
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.79 as f64,
                            g: 0.79 as f64,
                            b: 0.79 as f64,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            render_pass.set_pipeline(&self.render_pipeline);

            for (tile_id, rect) in tiles.clone() {
                let tile = if let Some(tile) = self.tile_cache.get(tile_id) {
                    tile
                } else {
                    continue;
                };

                if let Some(scissor) = rect.to_scissor(self.size) {
                    render_pass.set_scissor_rect(
                        scissor.min.x,
                        scissor.min.y,
                        scissor.width(),
                        scissor.height(),
                    );
                    render_pass.set_vertex_buffer(0, tile.vertex_buffer.slice(..));
                    render_pass
                        .set_index_buffer(tile.index_buffer.slice(..), wgpu::IndexFormat::Uint32);

                    for feature in tile.features.iter() {
                        let style = feature.paint.style(zoom);
                        let uniforms = TileUniforms::new(self.size, scale, rect, style);

                        render_pass.set_push_constants(
                            wgpu::ShaderStages::VERTEX,
                            0,
                            bytemuck::bytes_of(&uniforms),
                        );

                        let start = feature.elements.start as u32;
                        let end = feature.elements.end as u32;

                        render_pass.draw_indexed(start..end, 0, 0..1);
                    }
                }
            }
        }

        self.glyph_pipeline.upload(&self.queue);
        self.render_text(&mut encoder, &view, tiles, zoom, scale);

        self.queue.submit(Some(encoder.finish()));

        output.present();

        Ok(())
    }

    fn render_text<I: IntoIterator<Item = (TileId, Rect<i32>)>>(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        tiles: I,
        zoom: f32,
        scale: f32,
    ) {
        let mut text_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("text-render-pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &self.multisampled_framebuffer,
                resolve_target: Some(&view),
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });

        text_pass.set_pipeline(&self.glyph_pipeline.render_pipeline);
        text_pass.set_bind_group(0, &self.glyph_pipeline.atlas_bind_group, &[]);

        let scaled_dims = self.tile_size * scale;
        let mut label_bounds: Vec<Rect<f32>> = Vec::new();

        for (tile_id, rect) in tiles {
            let tile = if let Some(tile) = self.tile_cache.get(tile_id) {
                tile
            } else {
                continue;
            };

            text_pass.insert_debug_marker("new tile");
            text_pass.set_vertex_buffer(0, tile.text.vertex_buffer.slice(..));
            text_pass.set_index_buffer(tile.text.index_buffer.slice(..), wgpu::IndexFormat::Uint32);

            label_bounds.clear();

            for layer in tile.text.layers.iter().rev() {
                let style = layer.paint.style(zoom);

                let uniforms = TextUniforms::new(self.size, self.tile_size, scale, rect, &style);
                text_pass.set_push_constants(
                    wgpu::ShaderStages::VERTEX_FRAGMENT,
                    0,
                    bytemuck::bytes_of(&uniforms),
                );

                'outer: for label in layer.labels.iter() {
                    let scaled_point = scaled_dims * label.point * V2::new(1.0, -1.0);

                    let scaled_bounds = Rect::new(
                        label.bounds.min + scaled_point,
                        label.bounds.max + scaled_point,
                    );

                    for &placed_label in label_bounds.iter() {
                        if scaled_bounds.overlaps(placed_label) {
                            continue 'outer;
                        }
                    }

                    let start = if style.text_halo_width > 0.0 {
                        label.halo_elements.start as u32
                    } else {
                        label.elements.start as u32
                    };

                    let end = label.elements.end as u32;

                    text_pass.draw_indexed(start..end, 0, 0..1);

                    label_bounds.push(scaled_bounds);
                }
            }
        }
    }
}

pub struct GfxHandle {
    device: Arc<wgpu::Device>,
    glyph_render: GlyphRender,
}

impl GfxHandle {
    pub fn create_geometry(
        &mut self,
        tile_id: TileId,
        vertices: &[GeoVertex],
        indices: &[u32],
        features: Vec<FeatureDraw>,
        labels: &[LayerLabelDraw],
    ) -> TileGeometry {
        let vertex_buffer = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("tile-dynamic-vb"),
                contents: bytemuck::cast_slice(vertices),
                usage: wgpu::BufferUsages::VERTEX,
            });

        let index_buffer = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("tile-dynamic-index"),
                contents: bytemuck::cast_slice(indices),
                usage: wgpu::BufferUsages::INDEX,
            });

        for layer in labels.iter() {
            for label in layer.labels.iter() {
                for glyph in label.glyphs() {
                    self.glyph_render.prepare(label.text_size, glyph.glyph)
                }
            }
        }

        let text = self.create_text_geometry(labels);

        TileGeometry {
            tile_id,
            vertex_buffer,
            index_buffer,
            features,
            text,
        }
    }

    pub fn create_text_geometry(&self, tile_layers: &[LayerLabelDraw]) -> TextBuffers {
        let mut vertices: Vec<TextVertex> = Vec::new();
        let mut indices: Vec<u32> = Vec::new();
        let mut layers = Vec::new();
        let mut labels = Vec::new();
        let mut complete = true;

        {
            let cache = self.glyph_render.atlas_contents.read().unwrap();
            for layer in tile_layers {
                for label in layer.labels.iter() {
                    let halo_start = indices.len();
                    let line_height = label.bounds.height() / label.lines.len() as f32;
                    let anchor = V2::new(
                        label.bounds.width() / 2.0,
                        (label.bounds.height() / -2.0) + line_height,
                    );
                    let label_offset = label.offset;

                    for glyph in label.glyphs() {
                        if let Some(raster) = cache.get(&glyph.glyph.with_size(label.text_size)) {
                            let [p0, p1, p2, p3] = glyph.bounds.corners();
                            let [uv0, uv1, uv2, uv3] = raster.uv();

                            for n in 0..4 {
                                let halo = n + 1;
                                let h0 = TextVertex {
                                    position: p0 - anchor,
                                    uv: uv0,
                                    label_offset,
                                    halo,
                                };

                                let h1 = TextVertex {
                                    position: p1 - anchor,
                                    uv: uv1,
                                    label_offset,
                                    halo,
                                };

                                let h2 = TextVertex {
                                    position: p2 - anchor,
                                    uv: uv2,
                                    label_offset,
                                    halo,
                                };

                                let h3 = TextVertex {
                                    position: p3 - anchor,
                                    uv: uv3,
                                    label_offset,
                                    halo,
                                };

                                let idx = vertices.len() as u32;

                                vertices.push(h0);
                                vertices.push(h1);
                                vertices.push(h2);
                                vertices.push(h3);

                                indices.push(idx + 2);
                                indices.push(idx + 1);
                                indices.push(idx);

                                indices.push(idx + 1);
                                indices.push(idx + 2);
                                indices.push(idx + 3);
                            }
                        } else {
                            if glyph.glyph.1 != ' ' {
                                complete = false;
                            }
                        }
                    }

                    let element_start = indices.len();

                    for glyph in label.glyphs() {
                        if let Some(raster) = cache.get(&glyph.glyph.with_size(label.text_size)) {
                            let [p0, p1, p2, p3] = glyph.bounds.corners();
                            let [uv0, uv1, uv2, uv3] = raster.uv();

                            let v0 = TextVertex {
                                position: p0 - anchor,
                                uv: uv0,
                                label_offset,
                                halo: 0,
                            };

                            let v1 = TextVertex {
                                position: p1 - anchor,
                                uv: uv1,
                                label_offset,
                                halo: 0,
                            };

                            let v2 = TextVertex {
                                position: p2 - anchor,
                                uv: uv2,
                                label_offset,
                                halo: 0,
                            };

                            let v3 = TextVertex {
                                position: p3 - anchor,
                                uv: uv3,
                                label_offset,
                                halo: 0,
                            };

                            let idx = vertices.len() as u32;

                            vertices.push(v0);
                            vertices.push(v1);
                            vertices.push(v2);
                            vertices.push(v3);

                            indices.push(idx + 2);
                            indices.push(idx + 1);
                            indices.push(idx);

                            indices.push(idx + 1);
                            indices.push(idx + 2);
                            indices.push(idx + 3);
                        }
                    }

                    labels.push(LabelGeometry {
                        elements: element_start..indices.len(),
                        halo_elements: halo_start..element_start,
                        bounds: Rect::new(label.bounds.min - anchor, label.bounds.max - anchor),
                        point: label.offset,
                    });
                }

                layers.push(LabelLayerGeometry {
                    paint: layer.paint.clone(),
                    labels: labels.clone(),
                });

                labels.clear();
            }
        }

        let vertex_buffer = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("text-dynamic-vb"),
                contents: &bytemuck::cast_slice(vertices.as_slice()),
                usage: wgpu::BufferUsages::VERTEX,
            });

        let index_buffer = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("text-dynamic-index"),
                contents: &bytemuck::cast_slice(indices.as_slice()),
                usage: wgpu::BufferUsages::INDEX,
            });

        TextBuffers {
            vertex_buffer,
            index_buffer,
            layers,
            complete,
        }
    }
}

pub struct TileGeometry {
    pub tile_id: TileId,
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    features: Vec<FeatureDraw>,
    text: TextBuffers,
}

struct GlyphPipeline {
    atlas_texture: Arc<wgpu::Texture>,
    atlas_bind_group: wgpu::BindGroup,
    render_pipeline: wgpu::RenderPipeline,
    atlas_size: V2<u32>,
    atlas_contents: Arc<RwLock<HashMap<GlyphKey, AtlasEntry>>>,
    state: Arc<Mutex<GlyphRenderState>>,
    glyph_upload: Arc<RwLock<HashMap<GlyphKey, GlyphUploadEntry>>>,
}

impl GlyphPipeline {
    fn new(device: &wgpu::Device, config: &wgpu::SurfaceConfiguration, samples: u32) -> Self {
        let atlas_size = V2::new(TEXT_ATLAS_SIZE, TEXT_ATLAS_SIZE);
        let atlas_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("glyph-atlas-texture"),
            size: wgpu::Extent3d {
                width: atlas_size.x,
                height: atlas_size.y,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        let atlas_view = atlas_texture.create_view(&wgpu::TextureViewDescriptor::default());

        let atlas_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });

        let atlas_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            multisampled: false,
                            view_dimension: wgpu::TextureViewDimension::D2,
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                ],
                label: Some("glyph-atlas-bind-group-layout"),
            });

        let atlas_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            layout: &atlas_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&atlas_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&atlas_sampler),
                },
            ],
            label: Some("glyph-atlas-bind-group"),
        });

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("glyph-atlas-shader"),
            source: wgpu::ShaderSource::Wgsl(TEXT_WGSL.into()),
        });

        let render_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("glyph-atlas-pipeline-layout"),
                bind_group_layouts: &[&atlas_bind_group_layout],
                push_constant_ranges: &[wgpu::PushConstantRange {
                    stages: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    range: 0..PUSH_CONSTANT_LIMIT as u32,
                }],
            });

        let render_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("glyph-atlas-pipeline"),
            layout: Some(&render_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: "vs_main",
                buffers: &[TextVertex::desc()],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: "fs_main",
                targets: &[Some(wgpu::ColorTargetState {
                    format: config.format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                polygon_mode: wgpu::PolygonMode::Fill,
                unclipped_depth: false,
                conservative: false,
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState {
                count: samples,
                mask: !0,
                alpha_to_coverage_enabled: false,
            },
            multiview: None,
        });

        Self {
            atlas_texture: Arc::new(atlas_texture),
            atlas_bind_group,
            atlas_size,
            render_pipeline,
            atlas_contents: Arc::new(RwLock::new(HashMap::new())),
            state: Arc::new(Mutex::new(GlyphRenderState::default())),
            glyph_upload: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    fn glyph_render(&self) -> GlyphRender {
        GlyphRender {
            atlas_texture: self.atlas_texture.clone(),
            atlas_size: self.atlas_size,
            atlas_contents: self.atlas_contents.clone(),
            fonts: crate::FontCollection::new(),
            state: self.state.clone(),
            glyph_upload: self.glyph_upload.clone(),
        }
    }

    fn upload(&self, queue: &wgpu::Queue) {
        let mut upload = self.glyph_upload.write().unwrap();
        let mut atlas_contents = self.atlas_contents.write().unwrap();
        let mut atlas_state = self.state.lock().unwrap();

        let mut pending_set = HashMap::new();

        for (glyph_key, entry) in upload.drain() {
            if let GlyphUploadEntry::Prepared(metrics, bitmap) = entry {
                if atlas_contents.contains_key(&glyph_key) {
                    continue;
                }

                let width = metrics.width as u32;
                let height = metrics.height as u32;

                if atlas_state.cursor.x + width >= self.atlas_size.x {
                    atlas_state.cursor.y += atlas_state.row_height + 1;
                    atlas_state.cursor.x = 0;
                    atlas_state.row_height = 0;
                }

                atlas_state.row_height = atlas_state.row_height.max(height);

                if atlas_state.cursor.y + height >= self.atlas_size.y {
                    eprintln!("ATLAS FULL");
                    atlas_contents.clear();
                    atlas_state.cursor = V2::zero();
                    continue;
                }

                queue.write_texture(
                    wgpu::ImageCopyTexture {
                        texture: &self.atlas_texture,
                        mip_level: 0,
                        origin: wgpu::Origin3d {
                            x: atlas_state.cursor.x,
                            y: atlas_state.cursor.y,
                            z: 0,
                        },
                        aspect: wgpu::TextureAspect::All,
                    },
                    bitmap.as_slice(),
                    wgpu::ImageDataLayout {
                        offset: 0,
                        bytes_per_row: Some(width),
                        rows_per_image: None,
                    },
                    wgpu::Extent3d {
                        width,
                        height,
                        depth_or_array_layers: 1,
                    },
                );

                let entry = AtlasEntry {
                    offset: atlas_state.cursor,
                    dimensions: V2::new(width, height),
                };

                atlas_contents.insert(glyph_key, entry);

                atlas_state.cursor.x += width + 1;
            } else {
                pending_set.insert(glyph_key, entry);
            }
        }

        upload.extend(pending_set.drain());
    }
}

pub struct TextBuffers {
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    layers: Vec<LabelLayerGeometry>,
    complete: bool,
}

pub struct GpuTileCache<T> {
    entries: HashMap<TileId, TileEntry<T>>,
    generation_two: HashMap<TileId, TileEntry<T>>,
}

impl<T> GpuTileCache<T> {
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
            generation_two: HashMap::new(),
        }
    }

    pub fn clean(&mut self) {
        if self.entries.len() > 1000 {
            std::mem::swap(&mut self.entries, &mut self.generation_two);
            self.entries.clear();
        }
    }

    pub fn insert(&mut self, tile_id: TileId, entry: T) {
        self.clean();
        self.entries.insert(tile_id, TileEntry { entry });
    }

    pub fn get(&self, tile_id: TileId) -> Option<&T> {
        if let Some(entry) = self
            .entries
            .get(&tile_id)
            .or_else(|| self.generation_two.get(&tile_id))
        {
            Some(&entry.entry)
        } else {
            None
        }
    }

    pub fn contains(&self, tile_id: TileId) -> bool {
        self.entries.contains_key(&tile_id) || self.generation_two.contains_key(&tile_id)
    }
}

struct TileEntry<T> {
    entry: T,
}

#[repr(C)]
#[derive(Debug, Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct TextVertex {
    position: V2<f32>,
    uv: V2<f32>,
    label_offset: V2<f32>,
    halo: u32,
}

impl TextVertex {
    const ATTRIBS: [wgpu::VertexAttribute; 4] =
        wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2, 2 => Float32x2, 3 => Uint32];

    fn desc<'a>() -> wgpu::VertexBufferLayout<'a> {
        use std::mem;

        wgpu::VertexBufferLayout {
            array_stride: mem::size_of::<Self>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &Self::ATTRIBS,
        }
    }
}

#[repr(C)]
#[derive(Debug, Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
// Sorted fields in a weird order to fix possibly padding issue with shader
struct TileUniforms {
    scale: f32,
    line_width: f32,
    offset: V2<f32>,
    tile_dims: V2<f32>,
    window_dims: V2<f32>,
    fill_translate: V2<f32>,
    line_translate: V2<f32>,
    fill_color: V4<f32>,
    line_color: V4<f32>,
}

const _: () = assert!(
    std::mem::size_of::<TileUniforms>() <= PUSH_CONSTANT_LIMIT,
    "TileUniforms must fit within push constant limit"
);

impl TileUniforms {
    fn new(window_size: V2<u32>, scale: f32, rect: Rect<i32>, style: super::FeatureStyle) -> Self {
        Self {
            offset: rect.min.as_f32(),
            tile_dims: rect.dimensions().as_f32(),
            window_dims: window_size.as_f32(),
            scale,
            fill_color: style.fill_color().as_v4(),
            fill_translate: style.fill_translate(),
            line_color: style.line_color().as_v4(),
            line_translate: style.line_translate(),
            line_width: style.line_width(),
        }
    }
}

#[repr(C)]
#[derive(Debug, Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
// Sorted fields in a weird order to fix possibly padding issue with shader
struct TextUniforms {
    scale: f32,
    halo_width: f32,
    offset: V2<f32>,
    tile_dims: V2<f32>,
    window_dims: V2<f32>,
    text_color: V4<f32>,
    halo_color: V4<f32>,
}

impl TextUniforms {
    fn new(
        window_size: V2<u32>,
        tile_dims: V2<f32>,
        scale: f32,
        rect: Rect<i32>,
        style: &super::FeatureStyle,
    ) -> Self {
        let tile_dims = tile_dims * scale;
        Self {
            halo_width: style.text_halo_width,
            offset: rect.max.as_f32() - tile_dims,
            tile_dims,
            window_dims: window_size.as_f32(),
            scale,
            text_color: style.text_color().as_v4(),
            halo_color: style.text_halo_color().as_v4(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct LabelGeometry {
    pub elements: std::ops::Range<usize>,
    pub halo_elements: std::ops::Range<usize>,
    bounds: Rect<f32>,
    point: V2<f32>,
}
#[derive(Debug, Clone)]
pub struct LabelLayerGeometry {
    pub paint: super::FeaturePaint,
    pub labels: Vec<LabelGeometry>,
}

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GeoVertex {
    pub position: V2<f32>,
    pub normal: V2<f32>,
    pub fill: u32,
}

impl GeoVertex {}

impl GeoVertex {
    const ATTRIBS: [wgpu::VertexAttribute; 3] =
        wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2, 2 => Uint32];

    fn desc<'a>() -> wgpu::VertexBufferLayout<'a> {
        use std::mem;

        wgpu::VertexBufferLayout {
            array_stride: mem::size_of::<Self>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &Self::ATTRIBS,
        }
    }

    pub const BACKGROUND_VERTICES: &'static [GeoVertex] = &[
        GeoVertex {
            position: V2::new(-0.1, -0.1),
            normal: V2::zero(),
            fill: 1,
        },
        GeoVertex {
            position: V2::new(1.1, -0.1),
            normal: V2::zero(),
            fill: 1,
        },
        GeoVertex {
            position: V2::new(1.1, 1.1),
            normal: V2::zero(),
            fill: 1,
        },
        GeoVertex {
            position: V2::new(-0.1, 1.1),
            normal: V2::zero(),
            fill: 1,
        },
    ];

    pub const BACKGROUND_INDICES: &'static [u32] = &[0, 3, 1, 1, 3, 2];
}
