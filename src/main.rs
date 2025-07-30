use bstr::{BStr, ByteSlice};
use clap::Parser;
use lyon::{
    math::point,
    tessellation::{
        BuffersBuilder, FillOptions, FillTessellator, FillVertex, StrokeOptions, StrokeTessellator,
        StrokeVertex, VertexBuffers,
    },
};
use proto::{Tile, tile::GeomType};
use smallvec::SmallVec;
use winit::{
    event::{MouseButton, WindowEvent},
    event_loop::{EventLoop, EventLoopProxy},
    window::Window,
};

use math::{Rect, V2, V4};

use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::{Arc, Mutex, atomic::AtomicUsize, mpsc},
    thread::JoinHandle,
    time::{Duration, Instant},
};

use crate::{
    gfx::GeoVertex,
    tile_source::{TileRect, TileSourceCollection},
};
use crate::{
    style::SourceId,
    text::{FontCollection, GlyphId},
};

mod gfx;
mod mbtiles;
mod proto {
    include!(concat!(env!("OUT_DIR"), "/vector_tile.rs"));
}
mod style;
mod text;
mod tile_source;
mod versatiles;

const TILE_SCALE: f32 = 2.0;
const TILE_SIZE: f32 = 256.0 * TILE_SCALE;

/// Navigate OSM Vector tilesets
#[derive(Parser, Debug)]
#[command(author, version, about)]
struct Args {
    /// Path to a MapLibre style document
    style: std::path::PathBuf,
}

fn main() {
    env_logger::init();
    let event_loop: EventLoop<UserEvent> = EventLoop::with_user_event().build().unwrap();
    let proxy = event_loop.create_proxy();
    let mut application = Application::new(proxy);

    event_loop.run_app(&mut application).unwrap();
}

struct Application {
    event_loop_proxy: EventLoopProxy<UserEvent>,
    state: Option<ApplicationState>,
}

impl Application {
    fn new(event_loop_proxy: EventLoopProxy<UserEvent>) -> Self {
        Self {
            event_loop_proxy,
            state: None,
        }
    }
}

impl winit::application::ApplicationHandler<UserEvent> for Application {
    fn resumed(&mut self, active_event_loop: &winit::event_loop::ActiveEventLoop) {
        if self.state.is_none() {
            let state = ApplicationState::new(self.event_loop_proxy.clone(), active_event_loop);

            self.state = Some(state);
        }
    }

    fn window_event(
        &mut self,
        event_loop: &winit::event_loop::ActiveEventLoop,
        window_id: winit::window::WindowId,
        event: WindowEvent,
    ) {
        if let Some(state) = self.state.as_mut() {
            state.window_event(event_loop, window_id, event);
        }
    }

    fn user_event(&mut self, event_loop: &winit::event_loop::ActiveEventLoop, event: UserEvent) {
        if let Some(state) = self.state.as_mut() {
            state.user_event(event_loop, event);
        }
    }
}

struct GfxWindow {
    // Safety: must drop gfx before window
    gfx: gfx::Gfx,
    window: Box<Window>,
}

impl GfxWindow {
    fn new(window: Window) -> Self {
        let window = Box::new(window);
        let window_ref = unsafe { std::mem::transmute(window.as_ref()) };
        let gfx = gfx::Gfx::new(window_ref, TILE_SIZE);

        Self { gfx, window }
    }

    fn gfx(&mut self) -> &mut gfx::Gfx {
        &mut self.gfx
    }

    fn window(&mut self) -> &Window {
        &self.window
    }

    fn request_redraw(&self) {
        self.window.request_redraw()
    }
}

struct ApplicationState {
    window: GfxWindow,
    input_state: InputState,
    slippy: SlippyMap,
    tile_loader: TileLoader,
    target_zoom: f64,
    frame_times: VecDeque<Duration>,
    frame_time: Instant,
    tiles_pending_glyphs: HashSet<TileId>,
}

impl ApplicationState {
    fn new(
        proxy: EventLoopProxy<UserEvent>,
        active_event_loop: &winit::event_loop::ActiveEventLoop,
    ) -> Self {
        let args = Args::parse();
        let style_json = std::fs::File::open(&args.style).unwrap();
        let style = style::Style::load(style_json).unwrap();
        let data_dir = args.style.parent().unwrap();
        let tile_source = TileSourceCollection::load(data_dir, &style).unwrap();

        let window = active_event_loop
            .create_window(
                Window::default_attributes()
                    .with_title("World Map")
                    .with_inner_size(winit::dpi::PhysicalSize {
                        width: 1920 * 2,
                        height: 1080 * 2,
                    }),
            )
            .unwrap();

        let mut window = GfxWindow::new(window);

        let input_state = InputState::new();

        let slippy = SlippyMap::new(
            V2::new(TILE_SIZE, TILE_SIZE).as_f64(),
            V2::new(1920 * 2, 1080 * 2),
            Camera {
                zoom: 13.0,
                position: V2::new(53.5461853, -113.5083185),
            },
        );

        let (tile_loader, tile_handle) = TileLoader::new(tile_source, style, window.gfx());
        let _t = std::thread::Builder::new()
            .name("tile-dispatch".into())
            .spawn({
                move || {
                    tile_handle.process_tiles(proxy);
                }
            });

        let target_zoom = slippy.current_zoom();

        Self {
            window,
            input_state,
            slippy,
            tile_loader,
            target_zoom,
            frame_times: VecDeque::new(),
            frame_time: Instant::now(),
            tiles_pending_glyphs: HashSet::new(),
        }
    }

    fn window_event(
        &mut self,
        event_loop: &winit::event_loop::ActiveEventLoop,
        _window_id: winit::window::WindowId,
        event: WindowEvent,
    ) {
        match event {
            WindowEvent::RedrawRequested => {
                let zoom = self.slippy.current_zoom();
                if zoom != self.target_zoom {
                    let diff = zoom - self.target_zoom;
                    let scroll = (0.02 * diff.abs()).max(0.01);

                    if diff.abs() < scroll || diff.abs() < 0.015 {
                        self.slippy.set_zoom(self.target_zoom);
                    } else {
                        let factor = scroll * diff.signum();
                        self.slippy.zoom(factor, self.input_state.mouse_position);
                    }
                }

                for (tile, _rect) in self.slippy.screen_tiles() {
                    if !self.window.gfx().has_tile(tile) {
                        self.tile_loader.prepare_tile(tile);
                    }
                }

                if zoom != self.target_zoom {
                    for tile in self.slippy.nearby_tiles(self.target_zoom) {
                        if !self.window.gfx().has_tile(tile) {
                            self.tile_loader.prepare_tile(tile);
                        }
                    }
                }

                if self.target_zoom != self.slippy.current_zoom() {
                    self.window.request_redraw();
                }

                let r = self.window.gfx().render(
                    self.slippy.screen_tiles(),
                    self.slippy.current_zoom() as f32,
                    self.slippy.scale() as f32,
                );

                for tile_id in self.tiles_pending_glyphs.drain() {
                    if !self.window.gfx().has_tile(tile_id) {
                        self.tile_loader.prepare_tile(tile_id);
                    }
                }

                self.frame_times.push_back(self.frame_time.elapsed());
                self.frame_time = Instant::now();

                if self.frame_times.len() > 100 {
                    let _ = self.frame_times.pop_front();
                    let avg = self
                        .frame_times
                        .iter()
                        .sum::<std::time::Duration>()
                        .as_secs_f64()
                        / self.frame_times.len() as f64;
                    let fps = 1.0 / avg;
                    let max = self.frame_times.iter().max().unwrap().as_secs_f64() * 1000.0;
                    //eprintln!("{:.1}fps {:.1}avg {:.1}max", fps, avg * 1000.0, max);
                }

                if let Err(e) = r {
                    eprintln!("{:?}", e);
                    if e == wgpu::SurfaceError::Outdated {
                        self.window.gfx().reconfigure();
                    }
                }
            }
            WindowEvent::Resized(size) => {
                let size = V2::new(size.width, size.height);
                self.window.gfx().resize(size);
                self.slippy.resize(size);
                self.window.request_redraw();
            }
            WindowEvent::CursorMoved { position, .. } => {
                let delta = self
                    .input_state
                    .set_mouse_position(V2::new(position.x, position.y));

                if self.input_state.get_mouse_button(MouseButton::Left) {
                    self.slippy.pan(delta);
                    self.window.request_redraw();
                }
            }
            WindowEvent::MouseInput { state, button, .. } => {
                let pressed = state == winit::event::ElementState::Pressed;

                if let winit::event::MouseButton::Left = button {
                    let cursor_icon = if pressed {
                        winit::window::CursorIcon::Grabbing
                    } else {
                        winit::window::CursorIcon::Default
                    };

                    self.window.window().set_cursor(cursor_icon);
                }
                if let winit::event::MouseButton::Right = button {
                    if pressed && !self.input_state.get_mouse_button(MouseButton::Right) {
                        self.window.request_redraw();
                    }
                }
                self.input_state.set_mouse_button(button, pressed);
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let y = match delta {
                    winit::event::MouseScrollDelta::LineDelta(_, y) => y as f64,
                    winit::event::MouseScrollDelta::PixelDelta(y) => y.y as f64,
                };

                let zoom_amount = y * 0.25;
                self.target_zoom += zoom_amount;
                self.target_zoom = self.target_zoom.max(0.0).min(23.0);

                self.window.request_redraw();
            }
            WindowEvent::CloseRequested => {
                event_loop.exit();
            }
            _ => (),
        }
    }

    fn user_event(&mut self, _event_loop: &winit::event_loop::ActiveEventLoop, event: UserEvent) {
        match event {
            UserEvent::TilePrepared(tile) => {
                self.window.gfx().store_tile(tile);
                self.window.request_redraw();
            }
            UserEvent::TilePendingGlyphs(tile_id) => {
                self.tiles_pending_glyphs.insert(tile_id);
            }
        }
    }
}

struct InputState {
    mouse: HashSet<MouseButton>,
    mouse_position: V2<f64>,
}

impl InputState {
    fn new() -> Self {
        InputState {
            mouse: HashSet::new(),
            mouse_position: V2::zero(),
        }
    }

    fn set_mouse_button(&mut self, button: MouseButton, pressed: bool) {
        if pressed {
            self.mouse.insert(button);
        } else {
            self.mouse.remove(&button);
        }
    }

    fn set_mouse_position(&mut self, xy: V2<f64>) -> V2<f64> {
        let delta = self.mouse_position - xy;

        self.mouse_position = xy;

        delta
    }

    fn get_mouse_button(&mut self, button: MouseButton) -> bool {
        self.mouse.contains(&button)
    }
}

#[derive(Debug, Copy, Clone)]
struct Camera {
    zoom: f64,
    position: V2<f64>,
}

trait RectExt<T> {
    fn left(&self) -> Rect<T>;
    fn right(&self) -> Rect<T>;
    fn up(&self) -> Rect<T>;
    fn down(&self) -> Rect<T>;
    fn to_scissor(&self, window: V2<u32>) -> Option<Rect<u32>>;
}

impl RectExt<i32> for Rect<i32> {
    fn left(&self) -> Rect<i32> {
        *self - V2::new(self.width(), 0)
    }

    fn right(&self) -> Rect<i32> {
        *self + V2::new(self.width(), 0)
    }

    fn up(&self) -> Rect<i32> {
        *self - V2::new(0, self.height())
    }

    fn down(&self) -> Rect<i32> {
        *self + V2::new(0, self.height())
    }

    fn to_scissor(&self, window: V2<u32>) -> Option<Rect<u32>> {
        let dims = self.dimensions();

        let (left, width) = if self.min.x < 0 {
            (0, dims.x + self.min.x)
        } else {
            (self.min.x, dims.x)
        };

        let (top, height) = if self.min.y < 0 {
            (0, dims.y + self.min.y)
        } else {
            (self.min.y, dims.y)
        };

        if width <= 0 || height <= 0 {
            return None;
        }

        if self.min.y > window.y as i32 || self.min.x > window.x as i32 {
            return None;
        }

        let left = left as u32;
        let top = top as u32;
        let width = width as u32;
        let height = height as u32;

        let width = width.min(window.x - left);
        let height = height.min(window.y - top);

        if width == 0 || height == 0 {
            return None;
        }

        let min = V2::new(left, top);
        let max = V2::new(width, height) + min;

        Some(Rect::new(min, max))
    }
}

#[derive(Debug, Clone)]
struct SlipTilesIter {
    window_dims: V2<u32>,
    zoom: u16,
    tile: V2<i32>,
    tile_region: Rect<i32>,
    direction: usize,
    direction_count: usize,
    direction_limit: usize,
    tile_limit: usize,
    counter: usize,
    first: bool,
}

impl SlipTilesIter {
    fn new(tile: TileId, tile_region: Rect<i32>, window_dims: V2<u32>) -> Self {
        let tile_count = (window_dims.as_f64() / tile_region.dimensions().as_f64())
            .ceil()
            .as_u32();

        let large_dim = tile_count.x.max(tile_count.y) + 5;
        let tile_limit = (large_dim * large_dim) as usize;

        Self {
            window_dims,
            zoom: tile.zoom,
            tile: V2::new(tile.row as i32, tile.column as i32),
            tile_region,
            direction: 0,
            direction_count: 0,
            direction_limit: 0,
            tile_limit,
            counter: 0,
            first: true,
        }
    }

    fn current_tile(&self) -> TileId {
        TileId::normalize(self.zoom, self.tile.y, self.tile.x)
    }
}

impl Iterator for SlipTilesIter {
    type Item = (TileId, Rect<i32>);

    fn next(&mut self) -> Option<Self::Item> {
        if self.first {
            self.first = false;
            self.counter += 1;
            self.direction_count += 1;

            Some((self.current_tile(), self.tile_region))
        } else {
            while self.counter < self.tile_limit {
                let (tile, region) = match self.direction & 3 {
                    0 => (self.tile + V2::new(0, 1), self.tile_region.right()),
                    1 => (self.tile - V2::new(1, 0), self.tile_region.down()),
                    2 => (self.tile - V2::new(0, 1), self.tile_region.left()),
                    3 => (self.tile + V2::new(1, 0), self.tile_region.up()),
                    _ => unreachable!(),
                };

                self.tile = tile;
                self.tile_region = region;

                self.counter += 1;
                self.direction_count += 1;

                if self.direction_count > self.direction_limit / 2 {
                    self.direction += 1;
                    self.direction_count = 0;
                    self.direction_limit += 1;
                }

                let tile = self.current_tile();
                if tile.is_valid() {
                    return Some((tile, self.tile_region));
                }
                /*
                if self.tile_region.to_scissor(self.window_dims).is_some() && tile.is_valid() {
                }
                */
            }

            None
        }
    }
}

#[derive(Debug, Clone)]
struct SlippyMap {
    camera: Camera,
    tile_dims: V2<f64>,
    window_dims: V2<u32>,
}

impl SlippyMap {
    fn new(tile_dims: V2<f64>, window_dims: V2<u32>, camera: Camera) -> Self {
        SlippyMap {
            camera,
            tile_dims,
            window_dims,
        }
    }

    fn screen_tiles(&self) -> SlipTilesIter {
        let (center_tile, offset) = self.center_tile();
        let scaled_tile_dims = self.scaled_tile_dims();

        let min = (((self.window_dims.as_f64() * V2::fill(0.5)) + (scaled_tile_dims * 0.0))
            - (offset * scaled_tile_dims))
            .as_i32();
        let max = min + scaled_tile_dims.as_i32();

        let rect = Rect::new(min, max);

        SlipTilesIter::new(center_tile, rect, self.window_dims)
    }

    fn nearby_tiles(&self, target_zoom: f64) -> impl IntoIterator<Item = TileId> {
        let nearest_zoom = self.nearest_zoom();

        let out_tiles = if nearest_zoom > 0 && target_zoom <= nearest_zoom as f64 {
            let mut zoom_out = self.clone();
            zoom_out.camera.zoom -= 1.0;

            Some(zoom_out.screen_tiles().into_iter().map(|(tile, _)| tile))
        } else {
            None
        };

        let in_tiles = if nearest_zoom < 23 && target_zoom > nearest_zoom as f64 {
            let mut zoom_in = self.clone();
            zoom_in.camera.zoom += 1.0;

            Some(zoom_in.screen_tiles().into_iter().map(|(tile, _)| tile))
        } else {
            None
        };

        out_tiles
            .into_iter()
            .flatten()
            .chain(in_tiles.into_iter().flatten())
    }

    fn scaled_tile_dims(&self) -> V2<f64> {
        self.tile_dims * self.scale()
    }

    fn resize(&mut self, size: V2<u32>) {
        self.window_dims = size;
    }

    fn scale(&self) -> f64 {
        let zoom = self.camera.zoom;

        if zoom < 24.0 {
            zoom.fract() + 1.0
        } else {
            2.0f64.powf(zoom - 23.0)
        }
    }

    fn current_zoom(&self) -> f64 {
        self.camera.zoom
    }

    fn center_tile(&self) -> (TileId, V2<f64>) {
        let zoom = self.nearest_zoom();
        let map_tiles = self.map_tile_dims();

        let lat = (1.0
            + ((self.camera.position.x * std::f64::consts::PI / 180.0)
                .tan()
                .asinh()
                / std::f64::consts::PI))
            / 2.0;

        let long = (self.camera.position.y + 180.0) / 360.0;

        let lat_long = V2::new(lat, long);

        let tile_row_col = map_tiles.as_f64() * lat_long;

        let tile_pos = tile_row_col.fract();
        let tile_pos = V2::new(tile_pos.y, 1.0 - tile_pos.x);

        let tile_row_col = tile_row_col.as_i32();

        (
            TileId::normalize(zoom as u16, tile_row_col.y, tile_row_col.x),
            tile_pos,
        )
    }

    fn map_tile_dims(&self) -> V2<u32> {
        let zoom = self.nearest_zoom();
        V2::fill(2u32.pow(zoom))
    }

    fn nearest_zoom(&self) -> u32 {
        (self.camera.zoom.floor() as u32).min(23).max(0)
    }

    fn pan(&mut self, offset: V2<f64>) {
        let offset = V2::new(-offset.y, offset.x) / self.scaled_tile_dims();

        let (center, current_offset) = self.center_tile();

        let offset = V2::new(center.row, center.column).as_f64()
            + V2::new(1.0 - current_offset.y, current_offset.x)
            + offset;

        let map_offset = offset / self.map_tile_dims().as_f64();

        let lat_rad = (std::f64::consts::PI * ((2.0 * map_offset.x as f64) - 1.0))
            .sinh()
            .atan();

        let lat = lat_rad * 180.0 / std::f64::consts::PI;
        let long = (map_offset.y * 360.0) - 180.0;

        if lat > -85.0 && lat < 85.0 {
            self.camera.position = V2::new(lat, long);
        }
    }

    fn zoom(&mut self, factor: f64, offset: V2<f64>) {
        if factor < 0.0 {
            let offset = offset - (self.window_dims.as_f64() / 2.0);
            self.pan(offset);
            self.camera.zoom = (self.current_zoom() - factor).max(0.0).min(23.0);
            self.pan(-offset);
        } else {
            self.camera.zoom = (self.current_zoom() - factor).max(0.0).min(23.0);
        }
    }

    fn set_zoom(&mut self, zoom: f64) {
        self.camera.zoom = zoom;
    }
}

struct TileLoaderHandle {
    data_receiver: mpsc::Receiver<TilePrepare>,
    pending_tiles: Arc<Mutex<HashSet<TileId>>>,
}

impl TileLoaderHandle {
    fn process_tiles(&self, proxy: winit::event_loop::EventLoopProxy<UserEvent>) {
        for tile in self.data_receiver.iter() {
            let mut pending_tiles = self.pending_tiles.lock().unwrap();
            match tile {
                TilePrepare::Ready(tile) => {
                    let tile_id = tile.tile_id;
                    let _ = proxy.send_event(UserEvent::TilePrepared(tile));
                    pending_tiles.remove(&tile_id);
                }
                TilePrepare::PendingGlyphs(tile_id) => {
                    let _ = proxy.send_event(UserEvent::TilePendingGlyphs(tile_id));
                    pending_tiles.remove(&tile_id);
                }
            }
        }
    }
}

enum TilePrepare {
    Ready(gfx::TileGeometry),
    PendingGlyphs(TileId),
}

struct TileLoader {
    pending_tiles: Arc<Mutex<HashSet<TileId>>>,
    workers: Arc<Vec<TileWorker>>,
    next_worker: Arc<AtomicUsize>,
}

impl TileLoader {
    fn new(
        tile_source: TileSourceCollection,
        style: style::Style,
        gfx: &gfx::Gfx,
    ) -> (Self, TileLoaderHandle) {
        let mut workers = Vec::new();

        let (data_sender, data_receiver) = mpsc::channel();

        for id in 0..std::thread::available_parallelism()
            .map(usize::from)
            .unwrap_or(1)
        {
            let handle = gfx.handle();
            workers.push(TileWorker::new(
                &tile_source,
                id,
                style.clone(),
                handle,
                data_sender.clone(),
            ));
        }

        let pending_tiles = Arc::new(Mutex::new(HashSet::new()));
        let handle = TileLoaderHandle {
            data_receiver,
            pending_tiles: pending_tiles.clone(),
        };

        let loader = TileLoader {
            workers: Arc::new(workers),
            next_worker: Arc::new(AtomicUsize::new(0)),
            pending_tiles,
        };

        (loader, handle)
    }

    fn prepare_tile(&self, tile_id: TileId) {
        use std::sync::atomic::Ordering;
        {
            let mut pending_tiles = self.pending_tiles.lock().unwrap();
            if pending_tiles.contains(&tile_id) {
                return;
            }

            pending_tiles.insert(tile_id);
        }

        let worker_id = self.next_worker.fetch_add(1, Ordering::SeqCst) % self.workers.len();

        self.workers[worker_id].send(tile_id)
    }
}

enum UserEvent {
    TilePrepared(gfx::TileGeometry),
    TilePendingGlyphs(TileId),
}

struct TileWorker {
    _handle: JoinHandle<()>,
    sender: mpsc::Sender<TileId>,
}
impl TileWorker {
    fn new(
        tile_source: &TileSourceCollection,
        id: usize,
        style: style::Style,
        gfx: gfx::GfxHandle,
        data_sender: mpsc::Sender<TilePrepare>,
    ) -> Self {
        let (sender, receiver) = mpsc::channel();

        let builder = std::thread::Builder::new();
        let handle = builder
            .name(format!("tesselator-{}", id))
            .spawn({
                let tile_source = tile_source.try_clone().unwrap();
                move || TileWorker::run(tile_source, style, gfx, data_sender, receiver)
            })
            .expect("unable to spawn worker");

        TileWorker {
            sender,
            _handle: handle,
        }
    }

    fn send(&self, tile_id: TileId) {
        let _ = self.sender.send(tile_id);
    }

    fn run(
        mut tile_source: TileSourceCollection,
        style: style::Style,
        mut gfx: gfx::GfxHandle,
        data_sender: mpsc::Sender<TilePrepare>,
        receiver: mpsc::Receiver<TileId>,
    ) {
        let mut tesselator = VectorTileTesselator::new(style, V2::fill(TILE_SIZE));

        for tile_id in receiver.iter() {
            tesselator.tesselate_tile(tile_id, &mut tile_source);

            if gfx.prepare_glyphs(tesselator.labels()) {
                let geo = gfx.create_geometry(
                    tile_id,
                    tesselator.vertices(),
                    tesselator.indices(),
                    tesselator.features().to_vec(),
                    tesselator.labels(),
                );

                if let Err(_) = data_sender.send(TilePrepare::Ready(geo)) {
                    break;
                }
            } else {
                if let Err(_) = data_sender.send(TilePrepare::PendingGlyphs(tile_id)) {
                    break;
                }
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct FeatureDraw {
    pub paint: FeaturePaint,
    pub elements: std::ops::Range<usize>,
}

#[derive(Debug, Copy, Clone)]
enum Value<'a> {
    String(&'a bstr::BStr),
    Number(f64),
    Bool(bool),
}

impl<'a> Value<'a> {
    fn as_str(&self) -> Option<&'a bstr::BStr> {
        match self {
            Value::String(s) => Some(s),
            Value::Number(_) => None,
            Value::Bool(_) => None,
        }
    }
}

impl std::fmt::Display for Value<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Value::String(s) => s.fmt(f),
            Value::Number(n) => n.fmt(f),
            Value::Bool(b) => b.fmt(f),
        }
    }
}

impl From<&'static str> for Value<'static> {
    fn from(value: &'static str) -> Self {
        Value::String(value.as_bytes().as_bstr())
    }
}

impl<'a> From<&'a proto::tile::Value> for Value<'a> {
    fn from(value: &'a proto::tile::Value) -> Self {
        if let Some(s) = value.string_value.as_ref() {
            Value::String(s.as_bstr())
        } else if let Some(n) = value.float_value {
            Value::Number(n as f64)
        } else if let Some(n) = value.double_value {
            Value::Number(n)
        } else if let Some(n) = value.int_value {
            Value::Number(n as f64)
        } else if let Some(n) = value.uint_value {
            Value::Number(n as f64)
        } else if let Some(n) = value.sint_value {
            Value::Number(n as f64)
        } else if let Some(n) = value.bool_value {
            Value::Bool(n)
        } else {
            unreachable!()
        }
    }
}

pub struct FeatureView<'a> {
    layer: &'a proto::tile::Layer,
    feature: &'a proto::tile::Feature,
}

static EMPTY_LAYER: proto::tile::Layer = proto::tile::Layer {
    version: 0,
    name: String::new(),
    features: Vec::new(),
    keys: Vec::new(),
    values: Vec::new(),
    extent: None,
};

static EMPTY_FEATURE: proto::tile::Feature = proto::tile::Feature {
    id: None,
    tags: Vec::new(),
    r#type: None,
    geometry: Vec::new(),
};

impl FeatureView<'static> {
    fn empty() -> Self {
        FeatureView {
            layer: &EMPTY_LAYER,
            feature: &EMPTY_FEATURE,
        }
    }
}

impl<'a> FeatureView<'a> {
    fn key<B: AsRef<BStr>>(&self, key: B) -> Option<Value<'_>> {
        let key = key.as_ref();
        if key == "$type" {
            return Some(match self.shape() {
                GeomType::Polygon => "Polygon".into(),
                GeomType::Linestring => "LineString".into(),
                GeomType::Point => "Point".into(),
                GeomType::Unknown => "Unknown".into(),
            });
        }

        for tag in self.feature.tags.chunks(2) {
            let t_key = tag[0] as usize;
            let t_value = tag[1] as usize;

            let key: Option<Value> = self
                .layer
                .keys
                .get(t_key)
                .filter(|k| k.as_slice() == key)
                .and_then(|_| self.layer.values.get(t_value).map(Value::from));

            if key.is_some() {
                return key;
            }
        }

        None
    }

    fn shape(&self) -> GeomType {
        self.feature.r#type()
    }
}

struct FeatureLayout<'a> {
    kind: style::LayerType,
    line_cap: lyon::path::LineCap,
    line_join: lyon::path::LineJoin,
    view: &'a FeatureView<'a>,
    style: &'a style::Layer,
    zoom: f32,
}

impl<'a> FeatureLayout<'a> {
    fn new(view: &'a FeatureView<'a>, style: &'a style::Layer, zoom: f32) -> Self {
        let kind = style.kind;

        let line_cap = match style.layout.line_cap {
            style::LineCap::Round => lyon::path::LineCap::Round,
            style::LineCap::Butt => lyon::path::LineCap::Butt,
            style::LineCap::Square => lyon::path::LineCap::Square,
        };

        let line_join = match style.layout.line_join {
            style::LineJoin::Round => lyon::path::LineJoin::Round,
            style::LineJoin::Miter => lyon::path::LineJoin::Miter,
            style::LineJoin::Bevel => lyon::path::LineJoin::Bevel,
        };

        FeatureLayout {
            kind,
            line_cap,
            line_join,
            view,
            zoom,
            style,
        }
    }

    fn visible(&self) -> bool {
        let supported = !self.style.paint.unsupported();
        let visible = self.style.layout.visibility == style::Visibility::Visible;
        let in_zoom = self.style.minzoom.map(|z| self.zoom >= z).unwrap_or(true)
            && self.style.maxzoom.map(|z| self.zoom <= z).unwrap_or(true);
        let valid_type = match self.kind {
            style::LayerType::Raster => false,
            style::LayerType::FillExtrusion => false,
            _ => true,
        };

        let visible =
            valid_type && supported && visible && in_zoom && self.style.filter(&self.view);

        visible
    }

    fn text(&self) -> Option<smartstring::alias::String> {
        self.style.layout.text(self.view)
    }

    fn text_font(&self) -> impl Iterator<Item = &str> {
        self.style.layout.text_font.iter().map(String::as_ref)
    }

    fn text_size(&self) -> f32 {
        self.style.layout.text_size(self.view, self.zoom) * TILE_SCALE
    }

    fn text_max_width(&self) -> f32 {
        self.style.layout.text_max_width() * TILE_SCALE
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct FeaturePaint {
    paint: style::Paint,
    kind: style::LayerType,
}

impl FeaturePaint {
    pub fn new(style_layer: &style::Layer, features: &FeatureView<'_>) -> Self {
        FeaturePaint {
            paint: style_layer.paint.eval(features),
            kind: style_layer.kind,
        }
    }

    pub fn style(&self, zoom: f32) -> FeatureStyle {
        let line_width = self.paint.line_width(zoom);
        let line_color = self.paint.line_color(zoom).into();

        let fill_translate = self.paint.fill_translate(zoom).into();
        let fill_color = self.paint.fill_color(zoom).into();
        let fill_outline_color = self.paint.fill_outline_color(zoom).map(Color::from);

        let background_color = self.paint.background_color(zoom).into();

        let text_color = self.paint.text_color(zoom).into();
        let text_halo_width = self.paint.text_halo_width(zoom).into();
        let text_halo_color = self.paint.text_halo_color(zoom).into();

        let line_dasharray = self.paint.line_dasharray();

        FeatureStyle {
            background_color,
            line_color,
            fill_color,
            line_width,
            fill_translate,
            fill_outline_color,
            text_color,
            text_halo_width,
            text_halo_color,
            line_dasharray,
            kind: self.kind,
        }
    }
}

#[derive(Debug)]
pub struct FeatureStyle {
    background_color: Color,
    line_color: Color,
    fill_color: Color,
    fill_outline_color: Option<Color>,
    line_width: f32,
    kind: style::LayerType,
    fill_translate: V2<f32>,
    text_color: Color,
    text_halo_width: f32,
    text_halo_color: Color,
    line_dasharray: SmallVec<[f32; 8]>,
}

impl FeatureStyle {
    pub fn fill_color(&self) -> Color {
        match self.kind {
            style::LayerType::Background => self.background_color,
            _ => self.fill_color,
        }
    }

    pub fn fill_outline_color(&self) -> Option<Color> {
        self.fill_outline_color
    }

    pub fn line_color(&self) -> Color {
        match self.kind {
            style::LayerType::Fill => self.fill_outline_color().unwrap_or(self.line_color),
            _ => self.line_color,
        }
    }

    pub fn kind(&self) -> style::LayerType {
        self.kind
    }

    pub fn line_width(&self) -> f32 {
        self.line_width * TILE_SCALE
    }

    pub fn fill_translate(&self) -> V2<f32> {
        self.fill_translate * TILE_SCALE
    }

    pub fn line_translate(&self) -> V2<f32> {
        match self.kind {
            style::LayerType::Fill => self.fill_translate(),
            _ => (0.0, 0.0).into(),
        }
    }

    pub fn text_color(&self) -> Color {
        self.text_color
    }

    pub fn text_halo_color(&self) -> Color {
        self.text_halo_color
    }

    pub fn text_halo_width(&self) -> f32 {
        self.text_halo_width * TILE_SCALE
    }

    pub fn line_dasharray(&self) -> SmallVec<[f32; 8]> {
        self.line_dasharray.clone()
    }
}

struct TileContainer {
    names: HashMap<String, usize>,
    tiles: Vec<Option<Option<(Tile, TileRect)>>>,
}

impl TileContainer {
    fn new(style: &style::Style) -> Self {
        let mut names = HashMap::new();
        let mut tiles = Vec::new();

        for (name, _) in style.sources.iter() {
            names.insert(name.to_string(), tiles.len());
            tiles.push(None);
        }

        Self { names, tiles }
    }

    fn query_tile<'a>(
        &'a mut self,
        tile_source: &mut TileSourceCollection,
        source_id: &SourceId,
        tile_id: TileId,
    ) -> Option<&'a (Tile, TileRect)> {
        let idx = match source_id {
            SourceId::Name(n) => *self.names.get(n)?,
            SourceId::Index(idx) => *idx,
        };

        let slot = self.tiles.get_mut(idx)?;

        if let Some(tile) = slot {
            tile.as_ref()
        } else {
            let tile = tile_source.query_tile(source_id, tile_id);
            *slot = Some(tile);

            if let Some(tile) = slot {
                tile.as_ref()
            } else {
                None
            }
        }
    }

    fn clear(&mut self) {
        for tile in self.tiles.iter_mut() {
            *tile = None;
        }
    }
}

struct VectorTileTesselator {
    style: style::Style,
    fill_tessellator: FillTessellator,
    fill_options: FillOptions,
    stroke_tessellator: StrokeTessellator,
    stroke_options: StrokeOptions,
    geometry: VertexBuffers<GeoVertex, u32>,
    tile_dims: V2<f32>,
    fonts: FontCollection,
    tile_container: TileContainer,
    draw_commands: DrawCommands,
}

impl VectorTileTesselator {
    fn new(style: style::Style, tile_dims: V2<f32>) -> Self {
        let fill_options = FillOptions::default().with_tolerance(0.001);
        let fill_tessellator = FillTessellator::new();
        let stroke_options = StrokeOptions::default()
            .with_tolerance(0.0001)
            .with_line_width(0.01); // These values are very sensitive and can cause very different issues
        let stroke_tessellator = StrokeTessellator::new();
        let geometry: VertexBuffers<GeoVertex, u32> = VertexBuffers::new();
        let fonts = FontCollection::new();
        let tile_container = TileContainer::new(&style);
        let draw_commands = DrawCommands::new();

        VectorTileTesselator {
            style,
            fill_options,
            fill_tessellator,
            stroke_tessellator,
            stroke_options,
            geometry,
            tile_dims,
            fonts,
            tile_container,
            draw_commands,
        }
    }

    fn tesselate_tile(&mut self, id: TileId, tile_source: &mut TileSourceCollection) -> () {
        let zoom = id.zoom();

        self.geometry.vertices.clear();
        self.geometry.indices.clear();
        self.tile_container.clear();
        self.draw_commands.clear();

        for style_layer in self.style.layers.iter() {
            if style_layer.kind == style::LayerType::Background {
                let range_start = self.geometry.indices.len();
                self.draw_commands.add_draw_cmds(None, range_start);
                self.geometry
                    .vertices
                    .extend_from_slice(GeoVertex::BACKGROUND_VERTICES);
                self.geometry
                    .indices
                    .extend_from_slice(GeoVertex::BACKGROUND_INDICES);

                let range_end = self.geometry.indices.len();
                let draw = FeatureDraw {
                    paint: FeaturePaint::new(style_layer, &FeatureView::empty()),
                    elements: range_start..range_end,
                };

                self.draw_commands.feature_draw.push(draw);
                continue;
            }

            let Some(target_layer) = style_layer.layer.as_ref() else {
                continue;
            };

            let Some(source_id) = style_layer.source.as_ref() else {
                continue;
            };

            let Some((tile, tile_rect)) =
                self.tile_container.query_tile(tile_source, source_id, id)
            else {
                continue;
            };

            let Some(layer) = tile.layers.iter().find(|layer| &layer.name == target_layer) else {
                continue;
            };

            self.draw_commands.layer_labels.clear();
            self.draw_commands.draw_range_start = self.geometry.indices.len();

            for feature in layer.features.iter() {
                let view = FeatureView { layer, feature };
                let layout = FeatureLayout::new(&view, style_layer, zoom);

                if !layout.visible() {
                    continue;
                }

                let paint = FeaturePaint::new(&style_layer, &view);
                let style = paint.style(zoom);

                self.draw_commands
                    .add_draw_cmds(Some(&paint), self.geometry.indices.len());

                self.stroke_options = self
                    .stroke_options
                    .with_line_cap(layout.line_cap)
                    .with_line_join(layout.line_join);

                match feature.r#type() {
                    GeomType::Polygon => {
                        if layout.kind == style::LayerType::Fill {
                            let polygon =
                                PolygonIter::new(feature.geometry.iter().copied(), *tile_rect);

                            let mut fill_builder =
                                BuffersBuilder::new(&mut self.geometry, |vertex: FillVertex| {
                                    GeoVertex {
                                        position: vertex.position().to_tuple().into(),
                                        normal: V2::fill(0.0),
                                        advancement: 0.0,
                                        fill: gfx::FillMode::Polygon,
                                    }
                                });

                            let result = self.fill_tessellator.tessellate(
                                polygon,
                                &self.fill_options,
                                &mut fill_builder,
                            );

                            match result {
                                Err(e) => eprintln!("polygon {:?}", e),
                                _ => (),
                            }
                        }

                        let stroke = match layout.kind {
                            style::LayerType::Line => true,
                            style::LayerType::Fill => style.fill_outline_color().is_some(),
                            _ => false,
                        };

                        if stroke {
                            let polygon =
                                PolygonIter::new(feature.geometry.iter().copied(), *tile_rect);

                            let mut stroke_builder =
                                BuffersBuilder::new(&mut self.geometry, |vertex: StrokeVertex| {
                                    GeoVertex {
                                        position: vertex.position_on_path().to_tuple().into(),
                                        normal: vertex.normal().to_tuple().into(),
                                        advancement: vertex.advancement(),
                                        fill: gfx::FillMode::Line,
                                    }
                                });

                            let result = self.stroke_tessellator.tessellate(
                                polygon,
                                &self.stroke_options,
                                &mut stroke_builder,
                            );

                            match result {
                                Err(e) => eprintln!("polygon stroke {:?}", e),
                                _ => (),
                            }
                        }
                    }
                    GeomType::Linestring if layout.kind == style::LayerType::Line => {
                        let polygon =
                            LineStringIter::new(feature.geometry.iter().copied(), *tile_rect);

                        let mut stroke_builder =
                            BuffersBuilder::new(&mut self.geometry, |vertex: StrokeVertex| {
                                GeoVertex {
                                    position: vertex.position_on_path().to_tuple().into(),
                                    normal: vertex.normal().to_tuple().into(),
                                    advancement: vertex.advancement(),
                                    fill: gfx::FillMode::Line,
                                }
                            });

                        let result = self.stroke_tessellator.tessellate(
                            polygon,
                            &self.stroke_options,
                            &mut stroke_builder,
                        );

                        match result {
                            Err(e) => eprintln!("line string {:?}", e),
                            _ => (),
                        }
                    }
                    GeomType::Point => {
                        if layout.kind == style::LayerType::Symbol {
                            if let Some(text) = layout.text() {
                                let (font_id, font) = self.fonts.font(layout.text_font());
                                let font_size = layout.text_size();
                                let max_text_width = layout.text_max_width() * font_size;
                                let v_advance = font
                                    .horizontal_line_metrics(font_size)
                                    .map(|m| m.new_line_size)
                                    .unwrap_or_default();
                                let mut h_offset = 0.0;
                                let mut v_offset = 0.0;
                                let mut glyphs = SmallVec::new();
                                let mut lines: SmallVec<[LineDraw; 3]> = SmallVec::new();
                                let mut widest_line: f32 = 0.0;

                                let mut bounds_min = V2::fill(f32::MAX);
                                let mut bounds_max = V2::fill(f32::MIN);

                                let mut last_glyph = None;

                                for (idx, c) in text.chars().enumerate() {
                                    if ((h_offset > max_text_width && c == ' ') || c == '\n')
                                        && text.len() > idx + 2
                                    {
                                        widest_line = widest_line.max(h_offset);
                                        let line = LineDraw {
                                            width: h_offset,
                                            glyphs: glyphs.clone(),
                                        };
                                        lines.push(line);
                                        glyphs.clear();
                                        last_glyph = None;
                                        h_offset = 0.0;
                                        v_offset -= v_advance;
                                        continue;
                                    }

                                    if c.is_control() {
                                        last_glyph = None;
                                        continue;
                                    }

                                    if font.lookup_glyph_index(c) == 0 {
                                        last_glyph = None;
                                        continue;
                                    }

                                    let kern = last_glyph
                                        .and_then(|g| font.horizontal_kern(g, c, font_size))
                                        .unwrap_or_default();

                                    last_glyph = Some(c);

                                    let metrics = font.metrics(c, font_size);

                                    let min = V2::new(metrics.xmin, metrics.ymin).as_f32()
                                        + V2::new(h_offset + kern, v_offset);
                                    let dims = V2::new(metrics.width, metrics.height).as_f32();

                                    let bounds = Rect::new(min, min + dims);
                                    let glyph_id = GlyphId(font_id, c);

                                    bounds_min = bounds_min.min(bounds.min).min(bounds.max);
                                    bounds_max = bounds_max.max(bounds.max).max(bounds.min);

                                    if !c.is_whitespace() {
                                        glyphs.push(GlyphDraw {
                                            bounds,
                                            glyph: glyph_id,
                                        });
                                    }

                                    h_offset += metrics.advance_width;
                                }

                                let points =
                                    PointIter::new(feature.geometry.iter().copied(), *tile_rect);

                                if glyphs.len() > 0 {
                                    widest_line = widest_line.max(h_offset);
                                    let line = LineDraw {
                                        width: h_offset,
                                        glyphs,
                                    };
                                    lines.push(line);
                                }

                                if lines.len() > 0 {
                                    if lines.len() > 1 {
                                        for line in lines.iter_mut() {
                                            let adj_width = (widest_line - line.width) / 2.0;
                                            for glyph in line.glyphs.iter_mut() {
                                                glyph.bounds.min.x += adj_width;
                                                glyph.bounds.max.x += adj_width;
                                            }
                                        }
                                    }

                                    let bounds = Rect::new(bounds_min, bounds_max);

                                    for point in points {
                                        if point.x > 1.0
                                            || point.y > 1.0
                                            || point.x < 0.0
                                            || point.y < 0.0
                                        {
                                            continue;
                                        }

                                        let label = LabelDraw {
                                            text_size: layout.text_size(),
                                            offset: point,
                                            bounds,
                                            lines: lines.clone(),
                                        };

                                        self.draw_commands.layer_labels.push(label);
                                    }
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }

            self.draw_commands
                .add_draw_cmds(None, self.geometry.indices.len());
        }
    }

    fn features(&self) -> &[FeatureDraw] {
        self.draw_commands.feature_draw.as_slice()
    }

    fn vertices(&self) -> &[GeoVertex] {
        self.geometry.vertices.as_slice()
    }

    fn indices(&self) -> &[u32] {
        self.geometry.indices.as_slice()
    }

    fn labels(&self) -> &[LayerLabelDraw] {
        self.draw_commands.labels.as_slice()
    }
}

struct DrawCommands {
    feature_draw: Vec<FeatureDraw>,
    labels: Vec<LayerLabelDraw>,
    layer_labels: Vec<LabelDraw>,
    last_paint: Option<FeaturePaint>,
    draw_range_start: usize,
}

impl DrawCommands {
    fn new() -> Self {
        Self {
            feature_draw: Vec::new(),
            labels: Vec::new(),
            layer_labels: Vec::new(),
            last_paint: None,
            draw_range_start: 0,
        }
    }

    fn clear(&mut self) {
        self.feature_draw.clear();
        self.labels.clear();
        self.layer_labels.clear();
        self.draw_range_start = 0;
        self.last_paint = None;
    }

    fn add_draw_cmds(&mut self, next_paint: Option<&FeaturePaint>, indices: usize) {
        if (next_paint.is_none() || next_paint != self.last_paint.as_ref())
            && let Some(last) = self.last_paint.take()
        {
            if self.layer_labels.len() > 0 {
                let draw = LayerLabelDraw {
                    paint: last.clone(),
                    labels: self.layer_labels.clone(),
                };

                self.layer_labels.clear();
                self.labels.push(draw);
            }

            let range_end = indices;
            if range_end > self.draw_range_start {
                let draw = FeatureDraw {
                    paint: last,
                    elements: self.draw_range_start..range_end,
                };
                self.draw_range_start = range_end;

                self.feature_draw.push(draw);
            }
            self.last_paint = next_paint.cloned();
        } else if self.last_paint.is_none() {
            self.last_paint = next_paint.cloned();
        }
    }
}

#[derive(Clone, Debug)]
pub struct LayerLabelDraw {
    pub paint: FeaturePaint,
    pub labels: Vec<LabelDraw>,
}

#[derive(Clone, Debug)]
pub struct LabelDraw {
    pub offset: V2<f32>,
    pub bounds: Rect<f32>,
    pub lines: SmallVec<[LineDraw; 3]>,
    pub text_size: f32,
}

impl LabelDraw {
    fn glyphs(&self) -> impl Iterator<Item = &GlyphDraw> {
        self.lines.iter().flat_map(|l| l.glyphs.iter())
    }
}

#[derive(Clone, Debug)]
pub struct LineDraw {
    pub glyphs: SmallVec<[GlyphDraw; 20]>,
    width: f32,
}

#[derive(Clone, Debug)]
pub struct GlyphDraw {
    pub bounds: Rect<f32>,
    pub glyph: GlyphId,
}

struct PolygonIter<I: Iterator<Item = u32>> {
    inner: std::iter::Fuse<I>,
    cursor: GeoCursor,
    command: GeoCommand,
    count: u32,
    begin: lyon::math::Point,
    previous: lyon::math::Point,
    open: bool,
}

impl<I: Iterator<Item = u32>> PolygonIter<I> {
    fn new(inner: I, rect: TileRect) -> Self {
        PolygonIter {
            inner: inner.fuse(),
            cursor: GeoCursor::new(rect),
            command: GeoCommand::Unknown,
            count: 0,
            begin: lyon::math::Point::zero(),
            previous: lyon::math::Point::zero(),
            open: false,
        }
    }
}

impl<I: Iterator<Item = u32>> Iterator for PolygonIter<I> {
    type Item = lyon::path::PathEvent;

    fn next(&mut self) -> Option<Self::Item> {
        use lyon::path::PathEvent;

        loop {
            if self.count == 0 {
                if let Some(next) = self.inner.next() {
                    self.command = next.into();
                    self.count = next >> 3;
                } else {
                    if self.open {
                        self.open = false;
                        return Some(PathEvent::End {
                            last: self.previous,
                            first: self.begin,
                            close: true,
                        });
                    } else {
                        return None;
                    }
                }
            }

            if self.count == 0 {
                continue;
            }

            self.count -= 1;

            match self.command {
                GeoCommand::MoveTo => {
                    let dx = self.inner.next();
                    let dy = self.inner.next();

                    if let Some((dx, dy)) = dx.zip(dy) {
                        self.cursor.update(dx, dy);
                        let at = self.cursor.point();
                        self.begin = at;
                        self.previous = at;
                        self.open = true;
                        return Some(PathEvent::Begin { at });
                    }
                }
                GeoCommand::LineTo => {
                    let dx = self.inner.next();
                    let dy = self.inner.next();

                    if let Some((dx, dy)) = dx.zip(dy) {
                        self.cursor.update(dx, dy);
                        let from = self.previous;
                        let to = self.cursor.point();
                        self.previous = to;

                        return Some(PathEvent::Line { from, to });
                    }
                }
                GeoCommand::ClosePath => {
                    self.open = false;
                    return Some(PathEvent::End {
                        last: self.previous,
                        first: self.begin,
                        close: true,
                    });
                }
                GeoCommand::Unknown => (),
            }
        }
    }
}

struct LineStringIter<I: Iterator<Item = u32>> {
    inner: std::iter::Fuse<I>,
    cursor: GeoCursor,
    command: GeoCommand,
    count: u32,
    begin: lyon::math::Point,
    previous: lyon::math::Point,
    open: bool,
}

impl<I: Iterator<Item = u32>> LineStringIter<I> {
    fn new(inner: I, rect: TileRect) -> Self {
        LineStringIter {
            inner: inner.fuse(),
            cursor: GeoCursor::new(rect),
            command: GeoCommand::Unknown,
            count: 0,
            begin: lyon::math::Point::zero(),
            previous: lyon::math::Point::zero(),
            open: false,
        }
    }
}

impl<I: Iterator<Item = u32>> Iterator for LineStringIter<I> {
    type Item = lyon::path::PathEvent;

    fn next(&mut self) -> Option<Self::Item> {
        use lyon::path::PathEvent;

        loop {
            if self.count == 0 {
                if let Some(next) = self.inner.next() {
                    self.command = next.into();
                    self.count = next >> 3;
                } else {
                    if self.open {
                        self.open = false;
                        return Some(PathEvent::End {
                            last: self.previous,
                            first: self.begin,
                            close: false,
                        });
                    } else {
                        return None;
                    }
                }
            }

            if self.count == 0 {
                continue;
            }

            self.count -= 1;

            match self.command {
                GeoCommand::MoveTo => {
                    if self.open {
                        self.count += 1;
                        self.open = false;
                        return Some(PathEvent::End {
                            last: self.previous,
                            first: self.begin,
                            close: false,
                        });
                    }

                    let dx = self.inner.next();
                    let dy = self.inner.next();

                    if let Some((dx, dy)) = dx.zip(dy) {
                        self.cursor.update(dx, dy);
                        let at = self.cursor.point();
                        self.begin = at;
                        self.previous = at;
                        self.open = true;
                        return Some(PathEvent::Begin { at });
                    }
                }
                GeoCommand::LineTo => {
                    let dx = self.inner.next();
                    let dy = self.inner.next();

                    if let Some((dx, dy)) = dx.zip(dy) {
                        self.cursor.update(dx, dy);
                        let from = self.previous;
                        let to = self.cursor.point();
                        self.previous = to;

                        return Some(PathEvent::Line { from, to });
                    }
                }
                GeoCommand::ClosePath => (),
                GeoCommand::Unknown => (),
            }
        }
    }
}

struct PointIter<I: Iterator<Item = u32>> {
    inner: std::iter::Fuse<I>,
    cursor: GeoCursor,
    command: GeoCommand,
    count: u32,
}

impl<I: Iterator<Item = u32>> PointIter<I> {
    fn new(inner: I, rect: TileRect) -> Self {
        Self {
            inner: inner.fuse(),
            cursor: GeoCursor::new(rect),
            command: GeoCommand::Unknown,
            count: 0,
        }
    }
}

impl<I: Iterator<Item = u32>> Iterator for PointIter<I> {
    type Item = V2<f32>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if self.count == 0 {
                if let Some(next) = self.inner.next() {
                    self.command = next.into();
                    self.count = next >> 3;
                } else {
                    return None;
                }
            }

            if self.count == 0 {
                continue;
            }

            self.count -= 1;

            match self.command {
                GeoCommand::MoveTo => {
                    let dx = self.inner.next();
                    let dy = self.inner.next();
                    if let Some((dx, dy)) = dx.zip(dy) {
                        self.cursor.update(dx, dy);
                        return Some(self.cursor.v2());
                    }
                }
                _ => (),
            }
        }
    }
}

#[derive(Copy, Clone)]
struct GeoCursor {
    x: i64,
    y: i64,
    rect: TileRect,
}

impl GeoCursor {
    fn new(rect: TileRect) -> Self {
        GeoCursor { x: 0, y: 0, rect }
    }

    fn update(&mut self, dx: u32, dy: u32) {
        let dx = dx as i64;
        let dy = dy as i64;

        let dx = (dx >> 1) ^ (-(dx & 1));
        let dy = (dy >> 1) ^ (-(dy & 1));

        self.x += dx;
        self.y += dy;
    }

    fn point(&self) -> lyon::math::Point {
        let p = self.v2();
        point(p.x, p.y)
    }

    fn v2(&self) -> V2<f32> {
        let p = V2::new(self.x as f32 / 4096.0, self.y as f32 / 4096.0);
        self.rect.offset(p)
    }
}

#[derive(Debug, Copy, Clone)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Color {
    pub fn from_rgb(r: u8, g: u8, b: u8) -> Self {
        Color {
            r: (r as f32) / 255.0,
            g: (g as f32) / 255.0,
            b: (b as f32) / 255.0,
            a: 1.0,
        }
    }

    pub fn as_v4(&self) -> V4<f32> {
        V4::new(self.r, self.g, self.b, self.a)
    }

    pub fn as_srgb(&self) -> Self {
        Color {
            r: self.r.powf(2.2),
            g: self.r.powf(2.2),
            b: self.r.powf(2.2),
            a: self.a,
        }
    }
}

impl From<style::color::Color> for Color {
    fn from(style: style::color::Color) -> Self {
        let rgba = style.to_rgba();

        Color {
            r: rgba.r,
            g: rgba.g,
            b: rgba.b,
            a: rgba.a,
        }
    }
}

enum GeoCommand {
    MoveTo,
    LineTo,
    ClosePath,
    Unknown,
}

impl From<u32> for GeoCommand {
    fn from(n: u32) -> GeoCommand {
        match n & 7 {
            1 => GeoCommand::MoveTo,
            2 => GeoCommand::LineTo,
            7 => GeoCommand::ClosePath,
            _ => GeoCommand::Unknown,
        }
    }
}

#[derive(Debug, Copy, Clone, Hash, PartialEq, Eq)]
pub struct TileId {
    zoom: u16,
    column: u32,
    row: u32,
}

impl TileId {
    fn normalize(zoom: u16, column: i32, row: i32) -> Self {
        let limit = 2i32.pow(zoom as u32);

        let column = column.rem_euclid(limit);

        TileId {
            zoom,
            row: row as u32,
            column: column as u32,
        }
    }

    fn is_valid(&self) -> bool {
        self.row < self.limit() && self.column < self.limit()
    }

    pub fn parent(&self) -> Option<Self> {
        if self.zoom == 0 {
            None
        } else {
            Some(TileId {
                zoom: self.zoom - 1,
                column: self.column / 2,
                row: self.row / 2,
            })
        }
    }

    fn zoom(&self) -> f32 {
        self.zoom as f32
    }

    fn limit(&self) -> u32 {
        2u32.pow(self.zoom as u32)
    }
}
