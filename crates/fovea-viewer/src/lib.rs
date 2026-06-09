use std::{
    collections::{HashMap, HashSet},
    sync::Once,
};

use bytemuck::{Pod, Zeroable};
use js_sys::Date;
use serde::{Deserialize, Serialize};
use wasm_bindgen::prelude::*;
use web_sys::HtmlCanvasElement;
use wgpu::util::DeviceExt;

const WORLD_SIZE: f64 = 100_000.0;
const MAX_POINTS: u32 = 1_000_000;
const TILE_CACHE_SOFT_LIMIT_BYTES: usize = 256 * 1024 * 1024;
const TILE_CACHE_HARD_LIMIT_BYTES: usize = 384 * 1024 * 1024;
const OVERLAY_CACHE_SOFT_LIMIT_BYTES: usize = 128 * 1024 * 1024;
const OVERLAY_CACHE_HARD_LIMIT_BYTES: usize = 192 * 1024 * 1024;
const HEATMAP_CACHE_SOFT_LIMIT_BYTES: usize = 96 * 1024 * 1024;
const HEATMAP_CACHE_HARD_LIMIT_BYTES: usize = 160 * 1024 * 1024;
const POLYGON_OUTLINE_MIN_ZOOM: f64 = 0.05;
const OVERLAY_PICK_RADIUS_PX: f64 = 8.0;
const OVERLAY_HOVER_CLASS_ID: u32 = u32::MAX;
const OVERLAY_SELECTED_CLASS_ID: u32 = u32::MAX - 1;

static PANIC_HOOK: Once = Once::new();

#[cfg(target_arch = "wasm32")]
fn js_error(message: impl AsRef<str>) -> JsValue {
    JsValue::from_str(message.as_ref())
}

#[cfg(not(target_arch = "wasm32"))]
fn js_error(_message: impl AsRef<str>) -> JsValue {
    JsValue::NULL
}

#[wasm_bindgen]
pub struct FoveaViewer {
    renderer: Renderer,
    camera: Camera,
    events: Vec<ViewerEvent>,
}

#[wasm_bindgen]
impl FoveaViewer {
    #[wasm_bindgen(js_name = create)]
    pub async fn create(canvas: HtmlCanvasElement) -> Result<FoveaViewer, JsValue> {
        PANIC_HOOK.call_once(console_error_panic_hook::set_once);

        let mut renderer = Renderer::new(canvas).await?;
        let camera =
            Camera::fit_dimensions(renderer.width, renderer.height, WORLD_SIZE, WORLD_SIZE);
        renderer.write_camera(&camera);
        renderer.set_point_count(100_000)?;

        Ok(Self {
            renderer,
            camera,
            events: Vec::new(),
        })
    }

    pub fn resize(&mut self, width: u32, height: u32, device_pixel_ratio: f64) {
        self.renderer.resize(width, height);
        let device_pixel_ratio = device_pixel_ratio.max(0.00001);
        self.camera.viewport_width_px =
            ((f64::from(width) / device_pixel_ratio).round() as u32).max(1);
        self.camera.viewport_height_px =
            ((f64::from(height) / device_pixel_ratio).round() as u32).max(1);
        self.camera.device_pixel_ratio = device_pixel_ratio;
        self.renderer.write_camera(&self.camera);
    }

    #[wasm_bindgen(js_name = loadManifest)]
    pub fn load_manifest(&mut self, manifest_json: &str) -> Result<(), JsValue> {
        let manifest = SlideManifest::from_json(manifest_json)?;
        self.camera = Camera::fit_dimensions(
            self.renderer.width,
            self.renderer.height,
            manifest.width,
            manifest.height,
        );
        self.renderer.set_slide_manifest(manifest);
        self.renderer.write_camera(&self.camera);
        Ok(())
    }

    #[wasm_bindgen(js_name = resetCamera)]
    pub fn reset_camera(&mut self) {
        let (width, height) = self
            .renderer
            .slide_dimensions()
            .unwrap_or((WORLD_SIZE, WORLD_SIZE));
        self.camera =
            Camera::fit_dimensions(self.renderer.width, self.renderer.height, width, height);
        self.renderer.write_camera(&self.camera);
    }

    #[wasm_bindgen(js_name = panByScreenDelta)]
    pub fn pan_by_screen_delta(&mut self, delta_x: f64, delta_y: f64) {
        self.camera.pan_by_screen_delta(delta_x, delta_y);
        self.camera.clamp_to_world();
        self.renderer.write_camera(&self.camera);
        self.events.push(ViewerEvent::ViewportChanged {
            center_x: self.camera.center_x,
            center_y: self.camera.center_y,
            zoom: self.camera.zoom,
        });
    }

    #[wasm_bindgen(js_name = zoomAt)]
    pub fn zoom_at(&mut self, screen_x: f64, screen_y: f64, wheel_delta_y: f64) {
        self.camera.zoom_at(screen_x, screen_y, wheel_delta_y);
        self.camera.clamp_to_world();
        self.renderer.write_camera(&self.camera);
        self.events.push(ViewerEvent::ViewportChanged {
            center_x: self.camera.center_x,
            center_y: self.camera.center_y,
            zoom: self.camera.zoom,
        });
    }

    #[wasm_bindgen(js_name = setPointCount)]
    pub fn set_point_count(&mut self, count: u32) -> Result<(), JsValue> {
        self.renderer.set_point_count(count.min(MAX_POINTS))
    }

    #[wasm_bindgen(js_name = visibleTileRequests)]
    pub fn visible_tile_requests(&self, max_requests: u32) -> String {
        self.renderer
            .visible_tile_requests(&self.camera, max_requests as usize)
            .unwrap_or_else(|| "[]".to_string())
    }

    #[wasm_bindgen(js_name = uploadTileRgba)]
    pub fn upload_tile_rgba(
        &mut self,
        level: u32,
        x: u32,
        y: u32,
        width: u32,
        height: u32,
        rgba: &[u8],
    ) -> Result<(), JsValue> {
        self.renderer
            .upload_tile_rgba(TileId { level, x, y }, width, height, rgba)
    }

    #[wasm_bindgen(js_name = loadCellManifest)]
    pub fn load_cell_manifest(&mut self, manifest_json: &str) -> Result<(), JsValue> {
        let manifest = CellOverlayManifest::from_json(manifest_json)?;
        let should_fit_overlay = self.renderer.slide_dimensions().is_none();
        let dimensions = manifest.dimensions();
        self.renderer.set_cell_overlay_manifest(manifest);
        if should_fit_overlay {
            self.camera = Camera::fit_dimensions(
                self.renderer.width,
                self.renderer.height,
                dimensions.0,
                dimensions.1,
            );
            self.renderer.write_camera(&self.camera);
        }
        Ok(())
    }

    #[wasm_bindgen(js_name = loadHeatmapManifest)]
    pub fn load_heatmap_manifest(&mut self, manifest_json: &str) -> Result<(), JsValue> {
        let manifest = HeatmapManifest::from_json(manifest_json)?;
        let should_fit_heatmap = self.renderer.slide_dimensions().is_none();
        let dimensions = manifest.dimensions();
        self.renderer.set_heatmap_manifest(manifest);
        if should_fit_heatmap {
            self.camera = Camera::fit_dimensions(
                self.renderer.width,
                self.renderer.height,
                dimensions.0,
                dimensions.1,
            );
            self.renderer.write_camera(&self.camera);
        }
        Ok(())
    }

    #[wasm_bindgen(js_name = visibleCellChunkRequests)]
    pub fn visible_cell_chunk_requests(&self, max_requests: u32) -> String {
        self.renderer
            .visible_overlay_chunk_requests(&self.camera, max_requests as usize)
            .unwrap_or_else(|| "[]".to_string())
    }

    #[wasm_bindgen(js_name = uploadCellChunkBytes)]
    pub fn upload_cell_chunk_bytes(&mut self, x: u32, y: u32, bytes: &[u8]) -> Result<(), JsValue> {
        self.renderer
            .upload_overlay_chunk_bytes(OverlayChunkId { x, y }, bytes)
    }

    #[wasm_bindgen(js_name = visibleHeatmapTileRequests)]
    pub fn visible_heatmap_tile_requests(&self, max_requests: u32) -> String {
        self.renderer
            .visible_heatmap_tile_requests(&self.camera, max_requests as usize)
            .unwrap_or_else(|| "[]".to_string())
    }

    #[wasm_bindgen(js_name = uploadHeatmapTileBytes)]
    pub fn upload_heatmap_tile_bytes(
        &mut self,
        level: u32,
        x: u32,
        y: u32,
        width: u32,
        height: u32,
        bytes: &[u8],
    ) -> Result<(), JsValue> {
        self.renderer
            .upload_heatmap_tile_bytes(TileId { level, x, y }, width, height, bytes)
    }

    #[wasm_bindgen(js_name = setCellVisibility)]
    pub fn set_cell_visibility(&mut self, visible: bool) {
        self.renderer.set_overlay_visibility(visible);
        self.renderer.write_camera(&self.camera);

        if !visible {
            self.events.push(ViewerEvent::CellHover {
                cell_id: None,
                class_id: None,
                slide_x: None,
                slide_y: None,
            });
            self.events.push(ViewerEvent::SelectionChange { count: 0 });
        }
    }

    #[wasm_bindgen(js_name = setCellOpacity)]
    pub fn set_cell_opacity(&mut self, opacity: f64) {
        self.renderer.set_overlay_opacity(opacity);
        self.renderer.write_camera(&self.camera);
    }

    #[wasm_bindgen(js_name = setCellPointSize)]
    pub fn set_cell_point_size(&mut self, size_px: f64) {
        self.renderer.set_overlay_point_size(size_px);
        self.renderer.write_camera(&self.camera);
    }

    #[wasm_bindgen(js_name = setCellOutlineWidth)]
    pub fn set_cell_outline_width(&mut self, width_px: f64) {
        self.renderer.set_overlay_outline_width(width_px);
        self.renderer.write_camera(&self.camera);
    }

    #[wasm_bindgen(js_name = setHeatmapVisibility)]
    pub fn set_heatmap_visibility(&mut self, visible: bool) {
        self.renderer.set_heatmap_visibility(visible);
        self.renderer.write_camera(&self.camera);
    }

    #[wasm_bindgen(js_name = setHeatmapOpacity)]
    pub fn set_heatmap_opacity(&mut self, opacity: f64) {
        self.renderer.set_heatmap_opacity(opacity);
        self.renderer.write_camera(&self.camera);
    }

    #[wasm_bindgen(js_name = setHeatmapRange)]
    pub fn set_heatmap_range(&mut self, min: f64, max: f64) {
        self.renderer.set_heatmap_range(min, max);
        self.renderer.write_camera(&self.camera);
    }

    #[wasm_bindgen(js_name = setHeatmapColormap)]
    pub fn set_heatmap_colormap(&mut self, colormap: &str) {
        self.renderer.set_heatmap_colormap(colormap);
        self.renderer.write_camera(&self.camera);
    }

    #[wasm_bindgen(js_name = hoverAt)]
    pub fn hover_at(&mut self, screen_x: f64, screen_y: f64) {
        let hit = self.renderer.pick_cell(&self.camera, screen_x, screen_y);

        if self.renderer.hovered_cell_id() == hit.as_ref().map(|cell| cell.cell_id) {
            return;
        }

        self.renderer.set_hovered_cell(hit);
        self.events.push(ViewerEvent::CellHover {
            cell_id: self.renderer.hovered_cell_id(),
            class_id: self.renderer.hovered_class_id(),
            slide_x: self.renderer.hovered_slide_position().map(|p| p.0),
            slide_y: self.renderer.hovered_slide_position().map(|p| p.1),
        });
    }

    #[wasm_bindgen(js_name = clickAt)]
    pub fn click_at(&mut self, screen_x: f64, screen_y: f64) {
        let hit = self.renderer.pick_cell(&self.camera, screen_x, screen_y);
        self.renderer.set_selected_cell(hit.clone());
        self.events.push(ViewerEvent::CellClick {
            cell_id: hit.as_ref().map(|cell| cell.cell_id),
            class_id: hit.as_ref().map(|cell| cell.class_id),
            slide_x: hit.as_ref().map(|cell| f64::from(cell.centroid[0])),
            slide_y: hit.as_ref().map(|cell| f64::from(cell.centroid[1])),
        });
        self.events.push(ViewerEvent::SelectionChange {
            count: u32::from(hit.is_some()),
        });
    }

    #[wasm_bindgen(js_name = drainEvents)]
    pub fn drain_events(&mut self) -> String {
        let events = std::mem::take(&mut self.events);
        serde_json::to_string(&events).unwrap_or_else(|_| "[]".to_string())
    }

    pub fn render(&mut self) -> Result<FrameStats, JsValue> {
        self.renderer.render(&self.camera)
    }
}

#[derive(Serialize)]
#[serde(
    tag = "type",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
enum ViewerEvent {
    ViewportChanged {
        center_x: f64,
        center_y: f64,
        zoom: f64,
    },
    CellHover {
        cell_id: Option<u64>,
        class_id: Option<u32>,
        slide_x: Option<f64>,
        slide_y: Option<f64>,
    },
    CellClick {
        cell_id: Option<u64>,
        class_id: Option<u32>,
        slide_x: Option<f64>,
        slide_y: Option<f64>,
    },
    SelectionChange {
        count: u32,
    },
}

#[wasm_bindgen]
pub struct FrameStats {
    frame_time_ms: f64,
    upload_time_ms: f64,
    draw_call_count: u32,
    visible_object_count: u32,
    visible_tile_count: u32,
    loaded_tile_count: u32,
    visible_heatmap_tile_count: u32,
    loaded_heatmap_tile_count: u32,
    visible_cell_chunk_count: u32,
    loaded_cell_chunk_count: u32,
    visible_cell_count: u32,
    gpu_buffer_memory_bytes: u32,
    cpu_memory_bytes: u32,
}

#[wasm_bindgen]
impl FrameStats {
    #[wasm_bindgen(getter, js_name = frameTimeMs)]
    pub fn frame_time_ms(&self) -> f64 {
        self.frame_time_ms
    }

    #[wasm_bindgen(getter, js_name = uploadTimeMs)]
    pub fn upload_time_ms(&self) -> f64 {
        self.upload_time_ms
    }

    #[wasm_bindgen(getter, js_name = drawCallCount)]
    pub fn draw_call_count(&self) -> u32 {
        self.draw_call_count
    }

    #[wasm_bindgen(getter, js_name = visibleObjectCount)]
    pub fn visible_object_count(&self) -> u32 {
        self.visible_object_count
    }

    #[wasm_bindgen(getter, js_name = visibleTileCount)]
    pub fn visible_tile_count(&self) -> u32 {
        self.visible_tile_count
    }

    #[wasm_bindgen(getter, js_name = loadedTileCount)]
    pub fn loaded_tile_count(&self) -> u32 {
        self.loaded_tile_count
    }

    #[wasm_bindgen(getter, js_name = visibleHeatmapTileCount)]
    pub fn visible_heatmap_tile_count(&self) -> u32 {
        self.visible_heatmap_tile_count
    }

    #[wasm_bindgen(getter, js_name = loadedHeatmapTileCount)]
    pub fn loaded_heatmap_tile_count(&self) -> u32 {
        self.loaded_heatmap_tile_count
    }

    #[wasm_bindgen(getter, js_name = visibleCellChunkCount)]
    pub fn visible_cell_chunk_count(&self) -> u32 {
        self.visible_cell_chunk_count
    }

    #[wasm_bindgen(getter, js_name = loadedCellChunkCount)]
    pub fn loaded_cell_chunk_count(&self) -> u32 {
        self.loaded_cell_chunk_count
    }

    #[wasm_bindgen(getter, js_name = visibleCellCount)]
    pub fn visible_cell_count(&self) -> u32 {
        self.visible_cell_count
    }

    #[wasm_bindgen(getter, js_name = gpuBufferMemoryBytes)]
    pub fn gpu_buffer_memory_bytes(&self) -> u32 {
        self.gpu_buffer_memory_bytes
    }

    #[wasm_bindgen(getter, js_name = cpuMemoryBytes)]
    pub fn cpu_memory_bytes(&self) -> u32 {
        self.cpu_memory_bytes
    }
}

#[derive(Clone, Copy)]
struct Camera {
    center_x: f64,
    center_y: f64,
    zoom: f64,
    viewport_width_px: u32,
    viewport_height_px: u32,
    device_pixel_ratio: f64,
    world_width: f64,
    world_height: f64,
}

impl Camera {
    fn fit_dimensions(width: u32, height: u32, world_width: f64, world_height: f64) -> Self {
        let viewport_width = width.max(1) as f64;
        let viewport_height = height.max(1) as f64;
        let zoom_x = viewport_width * 0.92 / world_width.max(1.0);
        let zoom_y = viewport_height * 0.92 / world_height.max(1.0);
        let zoom = zoom_x.min(zoom_y).clamp(0.00001, 16.0);

        Self {
            center_x: world_width * 0.5,
            center_y: world_height * 0.5,
            zoom,
            viewport_width_px: width.max(1),
            viewport_height_px: height.max(1),
            device_pixel_ratio: 1.0,
            world_width,
            world_height,
        }
    }

    fn pan_by_screen_delta(&mut self, delta_x: f64, delta_y: f64) {
        self.center_x -= delta_x / self.zoom;
        self.center_y -= delta_y / self.zoom;
    }

    fn zoom_at(&mut self, screen_x: f64, screen_y: f64, wheel_delta_y: f64) {
        let before = self.screen_to_slide(screen_x, screen_y);
        let factor = (-wheel_delta_y * 0.001).exp();
        self.zoom = (self.zoom * factor).clamp(0.00001, 16.0);
        let after = self.screen_to_slide(screen_x, screen_y);

        self.center_x += before.0 - after.0;
        self.center_y += before.1 - after.1;
    }

    fn screen_to_slide(&self, screen_x: f64, screen_y: f64) -> (f64, f64) {
        let x = self.center_x + (screen_x - self.viewport_width_px as f64 * 0.5) / self.zoom;
        let y = self.center_y + (screen_y - self.viewport_height_px as f64 * 0.5) / self.zoom;
        (x, y)
    }

    fn slide_to_screen(&self, slide_x: f64, slide_y: f64) -> (f64, f64) {
        let x = (slide_x - self.center_x) * self.zoom + self.viewport_width_px as f64 * 0.5;
        let y = (slide_y - self.center_y) * self.zoom + self.viewport_height_px as f64 * 0.5;
        (x, y)
    }

    fn visible_rect(&self) -> Rect {
        let width = self.viewport_width_px as f64 / self.zoom;
        let height = self.viewport_height_px as f64 / self.zoom;
        Rect {
            x: self.center_x - width * 0.5,
            y: self.center_y - height * 0.5,
            width,
            height,
        }
        .clamped(self.world_width, self.world_height)
    }

    fn clamp_to_world(&mut self) {
        let half_width = self.viewport_width_px as f64 / self.zoom * 0.5;
        let half_height = self.viewport_height_px as f64 / self.zoom * 0.5;
        let min_x = half_width.min(self.world_width * 0.5);
        let max_x = (self.world_width - half_width).max(min_x);
        let min_y = half_height.min(self.world_height * 0.5);
        let max_y = (self.world_height - half_height).max(min_y);

        self.center_x = self.center_x.clamp(min_x, max_x);
        self.center_y = self.center_y.clamp(min_y, max_y);
    }

    fn as_uniform_with_heatmap(
        &self,
        overlay_style: OverlayStyle,
        heatmap_style: HeatmapStyle,
    ) -> CameraUniform {
        CameraUniform {
            center: [self.center_x as f32, self.center_y as f32],
            zoom: self.zoom as f32,
            _pad0: 0.0,
            viewport: [
                self.viewport_width_px as f32,
                self.viewport_height_px as f32,
            ],
            _pad1: [0.0, 0.0],
            overlay: [
                overlay_style.opacity,
                overlay_style.point_size_px,
                overlay_style.outline_width_px,
                if overlay_style.visible { 1.0 } else { 0.0 },
            ],
            heatmap: [
                if heatmap_style.visible {
                    heatmap_style.opacity
                } else {
                    0.0
                },
                heatmap_style.range_min,
                heatmap_style.range_max,
                heatmap_style.colormap_id,
            ],
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct Rect {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

impl Rect {
    fn right(self) -> f64 {
        self.x + self.width
    }

    fn bottom(self) -> f64 {
        self.y + self.height
    }

    fn center(self) -> (f64, f64) {
        (self.x + self.width * 0.5, self.y + self.height * 0.5)
    }

    fn clamped(self, max_width: f64, max_height: f64) -> Self {
        let x0 = self.x.clamp(0.0, max_width);
        let y0 = self.y.clamp(0.0, max_height);
        let x1 = self.right().clamp(0.0, max_width);
        let y1 = self.bottom().clamp(0.0, max_height);
        Self {
            x: x0,
            y: y0,
            width: (x1 - x0).max(0.0),
            height: (y1 - y0).max(0.0),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct TileId {
    level: u32,
    x: u32,
    y: u32,
}

#[derive(Clone, Debug, Deserialize)]
struct RawManifest {
    tile_size: u32,
    width: u32,
    height: u32,
    levels: Vec<LevelManifest>,
    tiles: Vec<TileManifest>,
}

#[derive(Clone, Debug, Deserialize)]
struct LevelManifest {
    index: u32,
    width: u32,
    height: u32,
    downsample: f64,
    tile_cols: u32,
    tile_rows: u32,
}

#[derive(Clone, Debug, Deserialize)]
struct TileManifest {
    level: u32,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    path: String,
    skipped: bool,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawHeatmapManifest {
    id: String,
    width: u32,
    height: u32,
    tile_size: u32,
    levels: Vec<HeatmapLevelManifest>,
    tiles: Vec<HeatmapTileManifest>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct HeatmapLevelManifest {
    index: u32,
    width: u32,
    height: u32,
    downsample: f64,
    tile_cols: u32,
    tile_rows: u32,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct HeatmapTileManifest {
    level: u32,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    path: String,
    byte_size: u64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct TileRequest {
    level: u32,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    path: String,
    priority: f64,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct OverlayChunkId {
    x: u32,
    y: u32,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawCellOverlayManifest {
    id: String,
    width: u32,
    height: u32,
    chunk_width: u32,
    chunk_height: u32,
    chunks: Vec<CellChunkManifest>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CellChunkManifest {
    x: u32,
    y: u32,
    path: String,
    cell_count: u32,
    #[allow(dead_code)]
    polygon_vertex_count: u32,
    byte_size: u64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct OverlayChunkRequest {
    x: u32,
    y: u32,
    path: String,
    cell_count: u32,
    byte_size: u64,
    priority: f64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct HeatmapTileRequest {
    level: u32,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    path: String,
    byte_size: u64,
    priority: f64,
}

#[derive(Clone)]
struct CellOverlayManifest {
    #[allow(dead_code)]
    id: String,
    width: f64,
    height: f64,
    chunk_width: u32,
    chunk_height: u32,
    chunks: HashMap<OverlayChunkId, CellChunkManifest>,
}

#[derive(Clone)]
struct HeatmapManifest {
    #[allow(dead_code)]
    id: String,
    width: f64,
    height: f64,
    tile_size: u32,
    levels: Vec<LevelManifest>,
    tiles: HashMap<TileId, HeatmapTileManifest>,
}

impl HeatmapManifest {
    fn from_json(manifest_json: &str) -> Result<Self, JsValue> {
        let raw: RawHeatmapManifest = serde_json::from_str(manifest_json)
            .map_err(|err| js_error(&format!("failed to parse heatmap manifest: {err}")))?;
        let mut tiles = HashMap::new();

        for tile in raw.tiles {
            tiles.insert(
                TileId {
                    level: tile.level,
                    x: tile.x,
                    y: tile.y,
                },
                tile,
            );
        }

        if raw.levels.is_empty() {
            return Err(js_error("heatmap manifest has no pyramid levels"));
        }

        let levels = raw
            .levels
            .into_iter()
            .map(|level| LevelManifest {
                index: level.index,
                width: level.width,
                height: level.height,
                downsample: level.downsample,
                tile_cols: level.tile_cols,
                tile_rows: level.tile_rows,
            })
            .collect();

        Ok(Self {
            id: raw.id,
            width: raw.width as f64,
            height: raw.height as f64,
            tile_size: raw.tile_size,
            levels,
            tiles,
        })
    }

    fn dimensions(&self) -> (f64, f64) {
        (self.width, self.height)
    }

    fn best_level(&self, camera: &Camera) -> &LevelManifest {
        self.levels
            .iter()
            .min_by(|left, right| {
                let left_error = (camera.zoom * left.downsample).log2().abs();
                let right_error = (camera.zoom * right.downsample).log2().abs();
                left_error
                    .partial_cmp(&right_error)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .unwrap_or(&self.levels[0])
    }

    fn visible_tiles(&self, camera: &Camera, level: &LevelManifest) -> Vec<VisibleTile> {
        let visible = camera.visible_rect();

        if visible.width <= 0.0 || visible.height <= 0.0 {
            return Vec::new();
        }

        let tile_size = f64::from(self.tile_size);
        let min_x = (visible.x / level.downsample).floor().max(0.0);
        let min_y = (visible.y / level.downsample).floor().max(0.0);
        let max_x = (visible.right() / level.downsample)
            .ceil()
            .min(f64::from(level.width));
        let max_y = (visible.bottom() / level.downsample)
            .ceil()
            .min(f64::from(level.height));

        if max_x <= min_x || max_y <= min_y {
            return Vec::new();
        }

        let min_tile_x = (min_x / tile_size).floor() as u32;
        let min_tile_y = (min_y / tile_size).floor() as u32;
        let max_tile_x = ((max_x - 1.0) / tile_size).floor() as u32;
        let max_tile_y = ((max_y - 1.0) / tile_size).floor() as u32;
        let center = visible.center();
        let mut tiles = Vec::new();

        for y in min_tile_y..=max_tile_y.min(level.tile_rows.saturating_sub(1)) {
            for x in min_tile_x..=max_tile_x.min(level.tile_cols.saturating_sub(1)) {
                let id = TileId {
                    level: level.index,
                    x,
                    y,
                };
                let Some(tile) = self.tiles.get(&id) else {
                    continue;
                };
                let rect = self.tile_world_rect(level, tile);
                let tile_center = rect.center();
                let distance =
                    (tile_center.0 - center.0).powi(2) + (tile_center.1 - center.1).powi(2);

                tiles.push(VisibleTile { id, rect, distance });
            }
        }

        tiles.sort_by(|left, right| {
            left.distance
                .partial_cmp(&right.distance)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        tiles
    }

    fn tile_world_rect(&self, level: &LevelManifest, tile: &HeatmapTileManifest) -> Rect {
        let x = f64::from(tile.x * self.tile_size) * level.downsample;
        let y = f64::from(tile.y * self.tile_size) * level.downsample;
        let width = f64::from(tile.width) * level.downsample;
        let height = f64::from(tile.height) * level.downsample;

        Rect {
            x,
            y,
            width,
            height,
        }
        .clamped(self.width, self.height)
    }
}

impl CellOverlayManifest {
    fn from_json(manifest_json: &str) -> Result<Self, JsValue> {
        let raw: RawCellOverlayManifest = serde_json::from_str(manifest_json)
            .map_err(|err| js_error(&format!("failed to parse cell overlay manifest: {err}")))?;

        if raw.chunk_width == 0 || raw.chunk_height == 0 {
            return Err(js_error("cell overlay chunk dimensions must be nonzero"));
        }

        let mut chunks = HashMap::new();

        for chunk in raw.chunks {
            chunks.insert(
                OverlayChunkId {
                    x: chunk.x,
                    y: chunk.y,
                },
                chunk,
            );
        }

        Ok(Self {
            id: raw.id,
            width: raw.width as f64,
            height: raw.height as f64,
            chunk_width: raw.chunk_width,
            chunk_height: raw.chunk_height,
            chunks,
        })
    }

    fn dimensions(&self) -> (f64, f64) {
        (self.width, self.height)
    }

    fn visible_chunks(&self, camera: &Camera) -> Vec<VisibleOverlayChunk> {
        let visible = camera.visible_rect();

        if visible.width <= 0.0 || visible.height <= 0.0 {
            return Vec::new();
        }

        let min_x = (visible.x / f64::from(self.chunk_width)).floor().max(0.0) as u32;
        let min_y = (visible.y / f64::from(self.chunk_height)).floor().max(0.0) as u32;
        let max_x = ((visible.right() - 1.0) / f64::from(self.chunk_width))
            .floor()
            .max(0.0) as u32;
        let max_y = ((visible.bottom() - 1.0) / f64::from(self.chunk_height))
            .floor()
            .max(0.0) as u32;
        let center = visible.center();
        let mut chunks = Vec::new();

        for y in min_y..=max_y {
            for x in min_x..=max_x {
                let id = OverlayChunkId { x, y };
                let Some(chunk) = self.chunks.get(&id) else {
                    continue;
                };
                let rect = self.chunk_world_rect(id);
                let chunk_center = rect.center();
                let distance =
                    (chunk_center.0 - center.0).powi(2) + (chunk_center.1 - center.1).powi(2);
                chunks.push(VisibleOverlayChunk {
                    id,
                    path: chunk.path.clone(),
                    cell_count: chunk.cell_count,
                    byte_size: chunk.byte_size,
                    distance,
                });
            }
        }

        chunks.sort_by(|left, right| {
            left.distance
                .partial_cmp(&right.distance)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        chunks
    }

    fn chunk_world_rect(&self, id: OverlayChunkId) -> Rect {
        Rect {
            x: f64::from(id.x * self.chunk_width),
            y: f64::from(id.y * self.chunk_height),
            width: f64::from(self.chunk_width),
            height: f64::from(self.chunk_height),
        }
        .clamped(self.width, self.height)
    }
}

#[derive(Clone)]
struct VisibleOverlayChunk {
    id: OverlayChunkId,
    path: String,
    cell_count: u32,
    byte_size: u64,
    distance: f64,
}

#[derive(Clone)]
struct SlideManifest {
    tile_size: u32,
    width: f64,
    height: f64,
    levels: Vec<LevelManifest>,
    tiles: HashMap<TileId, TileManifest>,
}

impl SlideManifest {
    fn from_json(manifest_json: &str) -> Result<Self, JsValue> {
        let raw: RawManifest = serde_json::from_str(manifest_json)
            .map_err(|err| js_error(&format!("failed to parse manifest: {err}")))?;
        let mut tiles = HashMap::new();

        for tile in raw.tiles {
            if tile.skipped {
                continue;
            }

            tiles.insert(
                TileId {
                    level: tile.level,
                    x: tile.x,
                    y: tile.y,
                },
                tile,
            );
        }

        if raw.levels.is_empty() {
            return Err(js_error("manifest has no pyramid levels"));
        }

        Ok(Self {
            tile_size: raw.tile_size,
            width: raw.width as f64,
            height: raw.height as f64,
            levels: raw.levels,
            tiles,
        })
    }

    fn level(&self, index: u32) -> Option<&LevelManifest> {
        self.levels.iter().find(|level| level.index == index)
    }

    fn best_level(&self, camera: &Camera) -> &LevelManifest {
        self.levels
            .iter()
            .min_by(|left, right| {
                let left_error = (camera.zoom * left.downsample).log2().abs();
                let right_error = (camera.zoom * right.downsample).log2().abs();
                left_error
                    .partial_cmp(&right_error)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .unwrap_or(&self.levels[0])
    }

    fn visible_tiles(&self, camera: &Camera, level: &LevelManifest) -> Vec<VisibleTile> {
        let visible = camera.visible_rect();

        if visible.width <= 0.0 || visible.height <= 0.0 {
            return Vec::new();
        }

        let tile_size = f64::from(self.tile_size);
        let min_x = (visible.x / level.downsample).floor().max(0.0);
        let min_y = (visible.y / level.downsample).floor().max(0.0);
        let max_x = (visible.right() / level.downsample)
            .ceil()
            .min(f64::from(level.width));
        let max_y = (visible.bottom() / level.downsample)
            .ceil()
            .min(f64::from(level.height));

        if max_x <= min_x || max_y <= min_y {
            return Vec::new();
        }

        let min_tile_x = (min_x / tile_size).floor() as u32;
        let min_tile_y = (min_y / tile_size).floor() as u32;
        let max_tile_x = ((max_x - 1.0) / tile_size).floor() as u32;
        let max_tile_y = ((max_y - 1.0) / tile_size).floor() as u32;
        let center = visible.center();
        let mut tiles = Vec::new();

        for y in min_tile_y..=max_tile_y.min(level.tile_rows.saturating_sub(1)) {
            for x in min_tile_x..=max_tile_x.min(level.tile_cols.saturating_sub(1)) {
                let id = TileId {
                    level: level.index,
                    x,
                    y,
                };
                let Some(tile) = self.tiles.get(&id) else {
                    continue;
                };
                let rect = self.tile_world_rect(level, tile);
                let tile_center = rect.center();
                let distance =
                    (tile_center.0 - center.0).powi(2) + (tile_center.1 - center.1).powi(2);

                tiles.push(VisibleTile { id, rect, distance });
            }
        }

        tiles.sort_by(|left, right| {
            left.distance
                .partial_cmp(&right.distance)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        tiles
    }

    fn tile_world_rect(&self, level: &LevelManifest, tile: &TileManifest) -> Rect {
        let x = f64::from(tile.x * self.tile_size) * level.downsample;
        let y = f64::from(tile.y * self.tile_size) * level.downsample;
        let width = f64::from(tile.width) * level.downsample;
        let height = f64::from(tile.height) * level.downsample;

        Rect {
            x,
            y,
            width,
            height,
        }
        .clamped(self.width, self.height)
    }
}

#[derive(Clone, Copy)]
struct VisibleTile {
    id: TileId,
    rect: Rect,
    distance: f64,
}

struct Renderer {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    width: u32,
    height: u32,
    triangle_pipeline: wgpu::RenderPipeline,
    tile_pipeline: wgpu::RenderPipeline,
    heatmap_pipeline: wgpu::RenderPipeline,
    point_pipeline: wgpu::RenderPipeline,
    overlay_point_pipeline: wgpu::RenderPipeline,
    overlay_line_pipeline: wgpu::RenderPipeline,
    camera_buffer: wgpu::Buffer,
    camera_bind_group: wgpu::BindGroup,
    texture_bind_group_layout: wgpu::BindGroupLayout,
    point_buffer: Option<wgpu::Buffer>,
    point_count: u32,
    slide_manifest: Option<SlideManifest>,
    cell_overlay_manifest: Option<CellOverlayManifest>,
    heatmap_manifest: Option<HeatmapManifest>,
    texture_cache: TextureCache,
    heatmap_cache: TextureCache,
    overlay_cache: OverlayCache,
    overlay_style: OverlayStyle,
    heatmap_style: HeatmapStyle,
    hovered_cell: Option<CellHit>,
    selected_cell: Option<CellHit>,
    frame_index: u64,
    last_upload_time_ms: f64,
    gpu_buffer_memory_bytes: u32,
    cpu_memory_bytes: u32,
}

#[derive(Clone, Copy)]
struct OverlayStyle {
    visible: bool,
    opacity: f32,
    point_size_px: f32,
    outline_width_px: f32,
}

#[derive(Clone, Copy)]
struct HeatmapStyle {
    visible: bool,
    opacity: f32,
    range_min: f32,
    range_max: f32,
    colormap_id: f32,
}

impl Default for HeatmapStyle {
    fn default() -> Self {
        Self {
            visible: true,
            opacity: 0.4,
            range_min: 0.05,
            range_max: 1.0,
            colormap_id: 0.0,
        }
    }
}

impl Default for OverlayStyle {
    fn default() -> Self {
        Self {
            visible: true,
            opacity: 0.78,
            point_size_px: 3.0,
            outline_width_px: 1.25,
        }
    }
}

impl Renderer {
    async fn new(canvas: HtmlCanvasElement) -> Result<Self, JsValue> {
        let width = canvas.width().max(1);
        let height = canvas.height().max(1);

        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::BROWSER_WEBGPU,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let surface = create_canvas_surface(&instance, canvas)?;
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
            })
            .await
            .map_err(|err| js_error(&format!("WebGPU adapter unavailable: {err:?}")))?;

        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("fovea-device"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::default(),
                experimental_features: wgpu::ExperimentalFeatures::disabled(),
                memory_hints: wgpu::MemoryHints::Performance,
                trace: wgpu::Trace::Off,
            })
            .await
            .map_err(|err| js_error(&format!("failed to request WebGPU device: {err:?}")))?;

        let surface_caps = surface.get_capabilities(&adapter);
        let format = surface_caps
            .formats
            .iter()
            .copied()
            .find(|format| format.is_srgb())
            .unwrap_or(surface_caps.formats[0]);

        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width,
            height,
            present_mode: wgpu::PresentMode::Fifo,
            alpha_mode: surface_caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &config);

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("fovea-phase2-shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/phase0.wgsl").into()),
        });

        let overlay_style = OverlayStyle::default();
        let heatmap_style = HeatmapStyle::default();
        let camera_uniform = Camera::fit_dimensions(width, height, WORLD_SIZE, WORLD_SIZE)
            .as_uniform_with_heatmap(overlay_style, heatmap_style);
        let camera_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("fovea-camera"),
            contents: bytemuck::bytes_of(&camera_uniform),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let camera_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("fovea-camera-layout"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
            });
        let camera_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("fovea-camera-bind-group"),
            layout: &camera_bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: camera_buffer.as_entire_binding(),
            }],
        });

        let texture_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("fovea-tile-texture-layout"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
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
            });

        let triangle_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("fovea-triangle-pipeline-layout"),
                bind_group_layouts: &[],
                immediate_size: 0,
            });
        let tile_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("fovea-tile-pipeline-layout"),
            bind_group_layouts: &[
                Some(&camera_bind_group_layout),
                Some(&texture_bind_group_layout),
            ],
            immediate_size: 0,
        });
        let point_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("fovea-point-pipeline-layout"),
                bind_group_layouts: &[Some(&camera_bind_group_layout)],
                immediate_size: 0,
            });
        let overlay_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("fovea-overlay-pipeline-layout"),
                bind_group_layouts: &[Some(&camera_bind_group_layout)],
                immediate_size: 0,
            });

        let triangle_pipeline =
            create_triangle_pipeline(&device, &triangle_pipeline_layout, &shader, format);
        let tile_pipeline = create_tile_pipeline(&device, &tile_pipeline_layout, &shader, format);
        let heatmap_pipeline =
            create_heatmap_pipeline(&device, &tile_pipeline_layout, &shader, format);
        let point_pipeline =
            create_point_pipeline(&device, &point_pipeline_layout, &shader, format);
        let overlay_point_pipeline =
            create_overlay_point_pipeline(&device, &overlay_pipeline_layout, &shader, format);
        let overlay_line_pipeline =
            create_overlay_line_pipeline(&device, &overlay_pipeline_layout, &shader, format);

        Ok(Self {
            surface,
            device,
            queue,
            config,
            width,
            height,
            triangle_pipeline,
            tile_pipeline,
            heatmap_pipeline,
            point_pipeline,
            overlay_point_pipeline,
            overlay_line_pipeline,
            camera_buffer,
            camera_bind_group,
            texture_bind_group_layout,
            point_buffer: None,
            point_count: 0,
            slide_manifest: None,
            cell_overlay_manifest: None,
            heatmap_manifest: None,
            texture_cache: TextureCache::new(
                TILE_CACHE_SOFT_LIMIT_BYTES,
                TILE_CACHE_HARD_LIMIT_BYTES,
            ),
            heatmap_cache: TextureCache::new(
                HEATMAP_CACHE_SOFT_LIMIT_BYTES,
                HEATMAP_CACHE_HARD_LIMIT_BYTES,
            ),
            overlay_cache: OverlayCache::new(
                OVERLAY_CACHE_SOFT_LIMIT_BYTES,
                OVERLAY_CACHE_HARD_LIMIT_BYTES,
            ),
            overlay_style,
            heatmap_style,
            hovered_cell: None,
            selected_cell: None,
            frame_index: 0,
            last_upload_time_ms: 0.0,
            gpu_buffer_memory_bytes: std::mem::size_of::<CameraUniform>() as u32,
            cpu_memory_bytes: 0,
        })
    }

    fn resize(&mut self, width: u32, height: u32) {
        let width = width.max(1);
        let height = height.max(1);

        if self.width == width && self.height == height {
            return;
        }

        self.width = width;
        self.height = height;
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&self.device, &self.config);
    }

    fn set_slide_manifest(&mut self, manifest: SlideManifest) {
        self.texture_cache.clear();
        self.slide_manifest = Some(manifest);
        self.last_upload_time_ms = 0.0;
    }

    fn set_cell_overlay_manifest(&mut self, manifest: CellOverlayManifest) {
        self.overlay_cache.clear();
        self.cell_overlay_manifest = Some(manifest);
        self.last_upload_time_ms = 0.0;
    }

    fn set_heatmap_manifest(&mut self, manifest: HeatmapManifest) {
        self.heatmap_cache.clear();
        self.heatmap_manifest = Some(manifest);
        self.last_upload_time_ms = 0.0;
    }

    fn slide_dimensions(&self) -> Option<(f64, f64)> {
        self.slide_manifest
            .as_ref()
            .map(|manifest| (manifest.width, manifest.height))
    }

    fn write_camera(&self, camera: &Camera) {
        self.queue.write_buffer(
            &self.camera_buffer,
            0,
            bytemuck::bytes_of(
                &camera.as_uniform_with_heatmap(self.overlay_style, self.heatmap_style),
            ),
        );
    }

    fn set_point_count(&mut self, count: u32) -> Result<(), JsValue> {
        let count = count.min(MAX_POINTS);
        let start = Date::now();
        let mut points = Vec::with_capacity(count as usize);
        let mut seed = 0x1234_5678_u32;

        for index in 0..count {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let x = (seed as f64 / u32::MAX as f64) * WORLD_SIZE;
            seed = seed
                .wrapping_mul(1_664_525)
                .wrapping_add(index ^ 1_013_904_223);
            let y = (seed as f64 / u32::MAX as f64) * WORLD_SIZE;
            points.push(PointVertex {
                position: [x as f32, y as f32],
            });
        }

        let bytes = bytemuck::cast_slice(&points);
        self.point_buffer = Some(self.device.create_buffer_init(
            &wgpu::util::BufferInitDescriptor {
                label: Some("fovea-synthetic-points"),
                contents: bytes,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            },
        ));
        self.point_count = count;
        self.last_upload_time_ms = Date::now() - start;
        self.update_memory_stats(bytes.len());

        Ok(())
    }

    fn visible_tile_requests(&self, camera: &Camera, max_requests: usize) -> Option<String> {
        let manifest = self.slide_manifest.as_ref()?;
        let best_level = manifest.best_level(camera);
        let mut requests = Vec::new();
        let mut seen = HashSet::new();

        for level in manifest.levels.iter().rev() {
            if level.index < best_level.index {
                continue;
            }

            let level_bias = if level.index == best_level.index {
                1_000_000_000.0
            } else {
                f64::from(best_level.index.abs_diff(level.index)) * 10_000.0
            };

            for visible in manifest.visible_tiles(camera, level) {
                if self.texture_cache.contains(visible.id) || !seen.insert(visible.id) {
                    continue;
                }

                if let Some(tile) = manifest.tiles.get(&visible.id) {
                    requests.push(TileRequest {
                        level: visible.id.level,
                        x: visible.id.x,
                        y: visible.id.y,
                        width: tile.width,
                        height: tile.height,
                        path: tile.path.clone(),
                        priority: level_bias + visible.distance,
                    });
                }
            }
        }

        requests.sort_by(|left, right| {
            left.priority
                .partial_cmp(&right.priority)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        requests.truncate(max_requests);
        serde_json::to_string(&requests).ok()
    }

    fn upload_tile_rgba(
        &mut self,
        id: TileId,
        width: u32,
        height: u32,
        rgba: &[u8],
    ) -> Result<(), JsValue> {
        let Some(manifest) = &self.slide_manifest else {
            return Ok(());
        };

        if !manifest.tiles.contains_key(&id) {
            return Ok(());
        }

        let expected_len = width as usize * height as usize * 4;
        if rgba.len() != expected_len {
            return Err(js_error(&format!(
                "tile RGBA byte length mismatch: got {}, expected {expected_len}",
                rgba.len()
            )));
        }

        let start = Date::now();
        self.texture_cache.insert_rgba(
            &self.device,
            &self.queue,
            &self.texture_bind_group_layout,
            id,
            width,
            height,
            rgba,
            self.frame_index,
        );
        self.last_upload_time_ms = Date::now() - start;
        self.update_memory_stats(0);
        Ok(())
    }

    fn visible_overlay_chunk_requests(
        &self,
        camera: &Camera,
        max_requests: usize,
    ) -> Option<String> {
        if !self.overlay_style.visible {
            return Some("[]".to_string());
        }

        let manifest = self.cell_overlay_manifest.as_ref()?;
        let mut requests = Vec::new();

        for visible in manifest.visible_chunks(camera) {
            if self.overlay_cache.contains(visible.id) {
                continue;
            }

            requests.push(OverlayChunkRequest {
                x: visible.id.x,
                y: visible.id.y,
                path: visible.path,
                cell_count: visible.cell_count,
                byte_size: visible.byte_size,
                priority: visible.distance,
            });
        }

        requests.truncate(max_requests);
        serde_json::to_string(&requests).ok()
    }

    fn upload_overlay_chunk_bytes(
        &mut self,
        id: OverlayChunkId,
        bytes: &[u8],
    ) -> Result<(), JsValue> {
        let Some(manifest) = &self.cell_overlay_manifest else {
            return Ok(());
        };

        if !manifest.chunks.contains_key(&id) {
            return Ok(());
        }

        let start = Date::now();
        self.overlay_cache
            .insert_chunk(&self.device, id, bytes, self.frame_index)?;
        self.last_upload_time_ms = Date::now() - start;
        self.update_memory_stats(0);
        Ok(())
    }

    fn visible_heatmap_tile_requests(
        &self,
        camera: &Camera,
        max_requests: usize,
    ) -> Option<String> {
        if !self.heatmap_style.visible {
            return Some("[]".to_string());
        }

        let manifest = self.heatmap_manifest.as_ref()?;
        let best_level = manifest.best_level(camera);
        let mut requests = Vec::new();
        let mut seen = HashSet::new();

        for level in manifest.levels.iter().rev() {
            if level.index < best_level.index {
                continue;
            }

            let level_bias = if level.index == best_level.index {
                1_000_000_000.0
            } else {
                f64::from(best_level.index.abs_diff(level.index)) * 10_000.0
            };

            for visible in manifest.visible_tiles(camera, level) {
                if self.heatmap_cache.contains(visible.id) || !seen.insert(visible.id) {
                    continue;
                }

                if let Some(tile) = manifest.tiles.get(&visible.id) {
                    requests.push(HeatmapTileRequest {
                        level: visible.id.level,
                        x: visible.id.x,
                        y: visible.id.y,
                        width: tile.width,
                        height: tile.height,
                        path: tile.path.clone(),
                        byte_size: tile.byte_size,
                        priority: level_bias + visible.distance,
                    });
                }
            }
        }

        requests.sort_by(|left, right| {
            left.priority
                .partial_cmp(&right.priority)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        requests.truncate(max_requests);
        serde_json::to_string(&requests).ok()
    }

    fn upload_heatmap_tile_bytes(
        &mut self,
        id: TileId,
        width: u32,
        height: u32,
        bytes: &[u8],
    ) -> Result<(), JsValue> {
        let Some(manifest) = &self.heatmap_manifest else {
            return Ok(());
        };

        if !manifest.tiles.contains_key(&id) {
            return Ok(());
        }

        let expected_len = width as usize * height as usize;
        if bytes.len() != expected_len {
            return Err(js_error(&format!(
                "heatmap tile byte length mismatch: got {}, expected {expected_len}",
                bytes.len()
            )));
        }

        let start = Date::now();
        self.heatmap_cache.insert_r8(
            &self.device,
            &self.queue,
            &self.texture_bind_group_layout,
            id,
            width,
            height,
            bytes,
            self.frame_index,
        );
        self.last_upload_time_ms = Date::now() - start;
        self.update_memory_stats(0);
        Ok(())
    }

    fn set_overlay_visibility(&mut self, visible: bool) {
        self.overlay_style.visible = visible;
        if !visible {
            self.hovered_cell = None;
            self.selected_cell = None;
        }
    }

    fn set_overlay_opacity(&mut self, opacity: f64) {
        self.overlay_style.opacity = (opacity as f32).clamp(0.0, 1.0);
    }

    fn set_overlay_point_size(&mut self, size_px: f64) {
        self.overlay_style.point_size_px = (size_px as f32).clamp(1.0, 12.0);
    }

    fn set_overlay_outline_width(&mut self, width_px: f64) {
        self.overlay_style.outline_width_px = (width_px as f32).clamp(0.25, 6.0);
    }

    fn set_heatmap_visibility(&mut self, visible: bool) {
        self.heatmap_style.visible = visible;
    }

    fn set_heatmap_opacity(&mut self, opacity: f64) {
        self.heatmap_style.opacity = (opacity as f32).clamp(0.0, 1.0);
    }

    fn set_heatmap_range(&mut self, min: f64, max: f64) {
        let min = (min as f32).clamp(0.0, 1.0);
        let max = (max as f32).clamp(0.0, 1.0);

        if max <= min {
            self.heatmap_style.range_min = min.min(0.99);
            self.heatmap_style.range_max = (self.heatmap_style.range_min + 0.01).min(1.0);
        } else {
            self.heatmap_style.range_min = min;
            self.heatmap_style.range_max = max;
        }
    }

    fn set_heatmap_colormap(&mut self, colormap: &str) {
        self.heatmap_style.colormap_id = match colormap {
            "viridis" => 1.0,
            "gray" | "grey" => 2.0,
            _ => 0.0,
        };
    }

    fn hovered_cell_id(&self) -> Option<u64> {
        self.hovered_cell.as_ref().map(|cell| cell.cell_id)
    }

    fn hovered_class_id(&self) -> Option<u32> {
        self.hovered_cell.as_ref().map(|cell| cell.class_id)
    }

    fn hovered_slide_position(&self) -> Option<(f64, f64)> {
        self.hovered_cell
            .as_ref()
            .map(|cell| (f64::from(cell.centroid[0]), f64::from(cell.centroid[1])))
    }

    fn set_hovered_cell(&mut self, hit: Option<CellHit>) {
        self.hovered_cell = hit;
    }

    fn set_selected_cell(&mut self, hit: Option<CellHit>) {
        self.selected_cell = hit;
    }

    fn pick_cell(&self, camera: &Camera, screen_x: f64, screen_y: f64) -> Option<CellHit> {
        if !self.overlay_style.visible {
            return None;
        }

        let manifest = self.cell_overlay_manifest.as_ref()?;
        let slide = camera.screen_to_slide(screen_x, screen_y);
        let radius_slide = OVERLAY_PICK_RADIUS_PX / camera.zoom.max(0.00001);
        let mut best: Option<(CellHit, f64)> = None;

        for visible in manifest.visible_chunks(camera) {
            let Some(entry) = self.overlay_cache.entry(visible.id) else {
                continue;
            };

            for cell in &entry.cells {
                if !cell.bbox_intersects_point(slide.0 as f32, slide.1 as f32, radius_slide as f32)
                {
                    continue;
                }

                let inside_polygon =
                    entry.cell_contains_point(cell, slide.0 as f32, slide.1 as f32);
                let screen_distance = cell.screen_distance_squared(camera, screen_x, screen_y);

                if !inside_polygon && screen_distance > OVERLAY_PICK_RADIUS_PX.powi(2) {
                    continue;
                }

                let score = if inside_polygon {
                    screen_distance * 0.01
                } else {
                    screen_distance
                };

                match &best {
                    Some((_, best_score)) if *best_score <= score => {}
                    _ => {
                        best = Some((
                            CellHit {
                                cell_id: cell.cell_id,
                                class_id: cell.class_id,
                                centroid: cell.centroid,
                            },
                            score,
                        ));
                    }
                }
            }
        }

        best.map(|(hit, _)| hit)
    }

    fn render(&mut self, camera: &Camera) -> Result<FrameStats, JsValue> {
        let start = Date::now();
        self.frame_index = self.frame_index.wrapping_add(1);
        let slide_draw = self.prepare_slide_draw(camera);
        let heatmap_draw = self.prepare_heatmap_draw(camera);
        let overlay_draw = self.prepare_overlay_draw(camera);
        let overlay_highlights = self.prepare_overlay_highlights();
        let tile_vertex_buffer = if slide_draw.vertices.is_empty() {
            None
        } else {
            Some(
                self.device
                    .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("fovea-visible-tile-vertices"),
                        contents: bytemuck::cast_slice(&slide_draw.vertices),
                        usage: wgpu::BufferUsages::VERTEX,
                    }),
            )
        };
        let overlay_highlight_buffer = if overlay_highlights.is_empty() {
            None
        } else {
            Some(
                self.device
                    .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("fovea-overlay-highlights"),
                        contents: bytemuck::cast_slice(&overlay_highlights),
                        usage: wgpu::BufferUsages::VERTEX,
                    }),
            )
        };

        let output = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(output)
            | wgpu::CurrentSurfaceTexture::Suboptimal(output) => output,
            wgpu::CurrentSurfaceTexture::Lost | wgpu::CurrentSurfaceTexture::Outdated => {
                self.surface.configure(&self.device, &self.config);
                match self.surface.get_current_texture() {
                    wgpu::CurrentSurfaceTexture::Success(output)
                    | wgpu::CurrentSurfaceTexture::Suboptimal(output) => output,
                    other => return Err(surface_texture_error(other)),
                }
            }
            other => return Err(surface_texture_error(other)),
        };

        let view = output
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("fovea-frame-encoder"),
            });

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("fovea-main-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(if self.slide_manifest.is_some() {
                            wgpu::Color {
                                r: 0.965,
                                g: 0.965,
                                b: 0.955,
                                a: 1.0,
                            }
                        } else {
                            wgpu::Color {
                                r: 0.05,
                                g: 0.055,
                                b: 0.06,
                                a: 1.0,
                            }
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });

            if let Some(vertex_buffer) = &tile_vertex_buffer {
                pass.set_pipeline(&self.tile_pipeline);
                pass.set_bind_group(0, &self.camera_bind_group, &[]);
                pass.set_vertex_buffer(0, vertex_buffer.slice(..));

                for command in &slide_draw.commands {
                    if let Some(entry) = self.texture_cache.entry(command.texture_id) {
                        pass.set_bind_group(1, &entry.bind_group, &[]);
                        pass.draw(command.vertex_start..command.vertex_start + 6, 0..1);
                    }
                }
            } else if self.slide_manifest.is_none()
                && self.cell_overlay_manifest.is_none()
                && self.heatmap_manifest.is_none()
            {
                pass.set_pipeline(&self.triangle_pipeline);
                pass.draw(0..3, 0..1);

                if let Some(point_buffer) = &self.point_buffer {
                    pass.set_pipeline(&self.point_pipeline);
                    pass.set_bind_group(0, &self.camera_bind_group, &[]);
                    pass.set_vertex_buffer(0, point_buffer.slice(..));
                    pass.draw(0..self.point_count, 0..1);
                }
            }

            if !heatmap_draw.commands.is_empty() && !heatmap_draw.vertices.is_empty() {
                let heatmap_vertex_buffer =
                    self.device
                        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                            label: Some("fovea-visible-heatmap-vertices"),
                            contents: bytemuck::cast_slice(&heatmap_draw.vertices),
                            usage: wgpu::BufferUsages::VERTEX,
                        });
                pass.set_pipeline(&self.heatmap_pipeline);
                pass.set_bind_group(0, &self.camera_bind_group, &[]);
                pass.set_vertex_buffer(0, heatmap_vertex_buffer.slice(..));

                for command in &heatmap_draw.commands {
                    if let Some(entry) = self.heatmap_cache.entry(command.texture_id) {
                        pass.set_bind_group(1, &entry.bind_group, &[]);
                        pass.draw(command.vertex_start..command.vertex_start + 6, 0..1);
                    }
                }
            }

            if !overlay_draw.commands.is_empty() || overlay_highlight_buffer.is_some() {
                pass.set_bind_group(0, &self.camera_bind_group, &[]);

                if camera.zoom >= POLYGON_OUTLINE_MIN_ZOOM {
                    pass.set_pipeline(&self.overlay_line_pipeline);
                    for command in &overlay_draw.commands {
                        if let Some(entry) = self.overlay_cache.entry(command.id) {
                            if entry.stroke_vertex_count == 0 {
                                continue;
                            }

                            pass.set_vertex_buffer(0, entry.stroke_buffer.slice(..));
                            pass.draw(0..entry.stroke_vertex_count, 0..1);
                        }
                    }
                }

                pass.set_pipeline(&self.overlay_point_pipeline);
                for command in &overlay_draw.commands {
                    if let Some(entry) = self.overlay_cache.entry(command.id) {
                        pass.set_vertex_buffer(0, entry.point_buffer.slice(..));
                        pass.draw(0..6, 0..entry.point_count);
                    }
                }

                if let Some(highlight_buffer) = &overlay_highlight_buffer {
                    pass.set_vertex_buffer(0, highlight_buffer.slice(..));
                    pass.draw(0..6, 0..overlay_highlights.len() as u32);
                }
            }
        }

        self.queue.submit(Some(encoder.finish()));
        output.present();
        self.texture_cache.evict(
            &slide_draw.cache_visible_ids,
            camera,
            self.slide_manifest.as_ref(),
        );
        self.heatmap_cache
            .evict(&heatmap_draw.cache_visible_ids, camera, None);
        self.overlay_cache.evict(&overlay_draw.visible_ids);
        self.update_memory_stats(0);
        let synthetic_draw_calls = if self.slide_manifest.is_none()
            && self.cell_overlay_manifest.is_none()
            && self.heatmap_manifest.is_none()
        {
            2
        } else {
            0
        };
        let heatmap_draw_calls = heatmap_draw.commands.len();
        let overlay_draw_calls = overlay_draw.draw_call_count(camera.zoom)
            + usize::from(overlay_highlight_buffer.is_some());

        Ok(FrameStats {
            frame_time_ms: Date::now() - start,
            upload_time_ms: self.last_upload_time_ms,
            draw_call_count: (slide_draw.commands.len()
                + heatmap_draw_calls
                + synthetic_draw_calls
                + overlay_draw_calls) as u32,
            visible_object_count: if overlay_draw.visible_cell_count > 0 {
                overlay_draw.visible_cell_count
            } else if heatmap_draw.visible_tile_count > 0 {
                heatmap_draw.visible_tile_count as u32
            } else if self.slide_manifest.is_some() {
                slide_draw.visible_tile_count as u32
            } else {
                self.point_count
            },
            visible_tile_count: slide_draw.visible_tile_count as u32,
            loaded_tile_count: self.texture_cache.len() as u32,
            visible_heatmap_tile_count: heatmap_draw.visible_tile_count as u32,
            loaded_heatmap_tile_count: self.heatmap_cache.len() as u32,
            visible_cell_chunk_count: overlay_draw.visible_chunk_count,
            loaded_cell_chunk_count: self.overlay_cache.len() as u32,
            visible_cell_count: overlay_draw.visible_cell_count,
            gpu_buffer_memory_bytes: self.gpu_buffer_memory_bytes,
            cpu_memory_bytes: self.cpu_memory_bytes,
        })
    }

    fn prepare_slide_draw(&mut self, camera: &Camera) -> SlideDraw {
        let Some(manifest) = &self.slide_manifest else {
            return SlideDraw::default();
        };

        let best_level = manifest.best_level(camera);
        let visible_tiles = manifest.visible_tiles(camera, best_level);
        let mut vertices = Vec::with_capacity(visible_tiles.len() * 6);
        let mut commands = Vec::new();
        let mut cache_visible_ids = HashSet::new();

        for visible in &visible_tiles {
            cache_visible_ids.insert(visible.id);

            let Some(sample) = self.sample_for_visible_tile(manifest, best_level, *visible) else {
                continue;
            };

            cache_visible_ids.insert(sample.texture_id);
            self.texture_cache
                .touch(sample.texture_id, self.frame_index);

            let vertex_start = vertices.len() as u32;
            push_tile_vertices(&mut vertices, visible.rect, sample.uv);
            commands.push(DrawCommand {
                texture_id: sample.texture_id,
                vertex_start,
            });
        }

        SlideDraw {
            vertices,
            commands,
            cache_visible_ids,
            visible_tile_count: visible_tiles.len(),
        }
    }

    fn prepare_heatmap_draw(&mut self, camera: &Camera) -> SlideDraw {
        if !self.heatmap_style.visible || self.heatmap_style.opacity <= 0.0 {
            return SlideDraw::default();
        }

        let Some(manifest) = &self.heatmap_manifest else {
            return SlideDraw::default();
        };

        let best_level = manifest.best_level(camera);
        let visible_tiles = manifest.visible_tiles(camera, best_level);
        let mut vertices = Vec::with_capacity(visible_tiles.len() * 6);
        let mut commands = Vec::new();
        let mut cache_visible_ids = HashSet::new();

        for visible in &visible_tiles {
            cache_visible_ids.insert(visible.id);

            let Some(sample) = self.sample_for_visible_heatmap_tile(manifest, best_level, *visible)
            else {
                continue;
            };

            cache_visible_ids.insert(sample.texture_id);
            self.heatmap_cache
                .touch(sample.texture_id, self.frame_index);

            let vertex_start = vertices.len() as u32;
            push_tile_vertices(&mut vertices, visible.rect, sample.uv);
            commands.push(DrawCommand {
                texture_id: sample.texture_id,
                vertex_start,
            });
        }

        SlideDraw {
            vertices,
            commands,
            cache_visible_ids,
            visible_tile_count: visible_tiles.len(),
        }
    }

    fn prepare_overlay_draw(&mut self, camera: &Camera) -> OverlayDraw {
        if !self.overlay_style.visible {
            return OverlayDraw::default();
        }

        let Some(manifest) = &self.cell_overlay_manifest else {
            return OverlayDraw::default();
        };

        let visible_chunks = manifest.visible_chunks(camera);
        let mut commands = Vec::new();
        let mut visible_ids = HashSet::new();
        let mut visible_cell_count = 0_u32;
        let visible_chunk_count = visible_chunks.len() as u32;

        for visible in visible_chunks {
            visible_ids.insert(visible.id);

            if self.overlay_cache.contains(visible.id) {
                self.overlay_cache.touch(visible.id, self.frame_index);
                commands.push(OverlayDrawCommand { id: visible.id });
                visible_cell_count = visible_cell_count.saturating_add(visible.cell_count);
            }
        }

        OverlayDraw {
            commands,
            visible_ids,
            visible_cell_count,
            visible_chunk_count,
        }
    }

    fn prepare_overlay_highlights(&self) -> Vec<OverlayPointVertex> {
        if !self.overlay_style.visible {
            return Vec::new();
        }

        let mut highlights = Vec::with_capacity(2);

        if let Some(cell) = &self.selected_cell {
            highlights.push(OverlayPointVertex {
                position: cell.centroid,
                class_id: OVERLAY_SELECTED_CLASS_ID,
                _pad: 0,
            });
        }

        if let Some(cell) = &self.hovered_cell {
            if self.selected_cell.as_ref().map(|selected| selected.cell_id) != Some(cell.cell_id) {
                highlights.push(OverlayPointVertex {
                    position: cell.centroid,
                    class_id: OVERLAY_HOVER_CLASS_ID,
                    _pad: 0,
                });
            }
        }

        highlights
    }

    fn sample_for_visible_tile(
        &self,
        manifest: &SlideManifest,
        best_level: &LevelManifest,
        visible: VisibleTile,
    ) -> Option<TileSample> {
        if self.texture_cache.contains(visible.id) {
            return Some(TileSample {
                texture_id: visible.id,
                uv: UvRect::full(),
            });
        }

        for level in manifest
            .levels
            .iter()
            .filter(|level| level.index > best_level.index)
        {
            let center = visible.rect.center();
            let level_x = (center.0 / level.downsample).floor().max(0.0) as u32;
            let level_y = (center.1 / level.downsample).floor().max(0.0) as u32;
            let id = TileId {
                level: level.index,
                x: (level_x / manifest.tile_size).min(level.tile_cols.saturating_sub(1)),
                y: (level_y / manifest.tile_size).min(level.tile_rows.saturating_sub(1)),
            };

            if !self.texture_cache.contains(id) {
                continue;
            }

            let tile = manifest.tiles.get(&id)?;
            let parent_rect = manifest.tile_world_rect(level, tile);
            let uv = UvRect {
                u0: ((visible.rect.x - parent_rect.x) / parent_rect.width).clamp(0.0, 1.0) as f32,
                v0: ((visible.rect.y - parent_rect.y) / parent_rect.height).clamp(0.0, 1.0) as f32,
                u1: ((visible.rect.right() - parent_rect.x) / parent_rect.width).clamp(0.0, 1.0)
                    as f32,
                v1: ((visible.rect.bottom() - parent_rect.y) / parent_rect.height).clamp(0.0, 1.0)
                    as f32,
            };

            return Some(TileSample { texture_id: id, uv });
        }

        None
    }

    fn sample_for_visible_heatmap_tile(
        &self,
        manifest: &HeatmapManifest,
        best_level: &LevelManifest,
        visible: VisibleTile,
    ) -> Option<TileSample> {
        if self.heatmap_cache.contains(visible.id) {
            return Some(TileSample {
                texture_id: visible.id,
                uv: UvRect::full(),
            });
        }

        for level in manifest
            .levels
            .iter()
            .filter(|level| level.index > best_level.index)
        {
            let center = visible.rect.center();
            let level_x = (center.0 / level.downsample).floor().max(0.0) as u32;
            let level_y = (center.1 / level.downsample).floor().max(0.0) as u32;
            let id = TileId {
                level: level.index,
                x: (level_x / manifest.tile_size).min(level.tile_cols.saturating_sub(1)),
                y: (level_y / manifest.tile_size).min(level.tile_rows.saturating_sub(1)),
            };

            if !self.heatmap_cache.contains(id) {
                continue;
            }

            let tile = manifest.tiles.get(&id)?;
            let parent_rect = manifest.tile_world_rect(level, tile);
            let uv = UvRect {
                u0: ((visible.rect.x - parent_rect.x) / parent_rect.width).clamp(0.0, 1.0) as f32,
                v0: ((visible.rect.y - parent_rect.y) / parent_rect.height).clamp(0.0, 1.0) as f32,
                u1: ((visible.rect.right() - parent_rect.x) / parent_rect.width).clamp(0.0, 1.0)
                    as f32,
                v1: ((visible.rect.bottom() - parent_rect.y) / parent_rect.height).clamp(0.0, 1.0)
                    as f32,
            };

            return Some(TileSample { texture_id: id, uv });
        }

        None
    }

    fn update_memory_stats(&mut self, extra_cpu_bytes: usize) {
        let point_bytes = self
            .point_count
            .checked_mul(std::mem::size_of::<PointVertex>() as u32)
            .unwrap_or(u32::MAX);
        let gpu_bytes = std::mem::size_of::<CameraUniform>() as usize
            + self.texture_cache.bytes
            + self.heatmap_cache.bytes
            + self.overlay_cache.bytes
            + point_bytes as usize;

        self.gpu_buffer_memory_bytes = gpu_bytes.min(u32::MAX as usize) as u32;
        self.cpu_memory_bytes =
            (extra_cpu_bytes + self.overlay_cache.cpu_bytes).min(u32::MAX as usize) as u32;
    }
}

#[derive(Default)]
struct SlideDraw {
    vertices: Vec<TileVertex>,
    commands: Vec<DrawCommand>,
    cache_visible_ids: HashSet<TileId>,
    visible_tile_count: usize,
}

#[derive(Default)]
struct OverlayDraw {
    commands: Vec<OverlayDrawCommand>,
    visible_ids: HashSet<OverlayChunkId>,
    visible_cell_count: u32,
    visible_chunk_count: u32,
}

impl OverlayDraw {
    fn draw_call_count(&self, zoom: f64) -> usize {
        if zoom >= POLYGON_OUTLINE_MIN_ZOOM {
            self.commands.len() * 2
        } else {
            self.commands.len()
        }
    }
}

struct OverlayDrawCommand {
    id: OverlayChunkId,
}

struct DrawCommand {
    texture_id: TileId,
    vertex_start: u32,
}

struct TileSample {
    texture_id: TileId,
    uv: UvRect,
}

#[derive(Clone, Copy)]
struct UvRect {
    u0: f32,
    v0: f32,
    u1: f32,
    v1: f32,
}

impl UvRect {
    fn full() -> Self {
        Self {
            u0: 0.0,
            v0: 0.0,
            u1: 1.0,
            v1: 1.0,
        }
    }
}

fn push_tile_vertices(vertices: &mut Vec<TileVertex>, rect: Rect, uv: UvRect) {
    let x0 = rect.x as f32;
    let y0 = rect.y as f32;
    let x1 = rect.right() as f32;
    let y1 = rect.bottom() as f32;

    vertices.extend_from_slice(&[
        TileVertex::new(x0, y0, uv.u0, uv.v0),
        TileVertex::new(x1, y0, uv.u1, uv.v0),
        TileVertex::new(x1, y1, uv.u1, uv.v1),
        TileVertex::new(x0, y0, uv.u0, uv.v0),
        TileVertex::new(x1, y1, uv.u1, uv.v1),
        TileVertex::new(x0, y1, uv.u0, uv.v1),
    ]);
}

struct OverlayCache {
    soft_limit_bytes: usize,
    hard_limit_bytes: usize,
    entries: HashMap<OverlayChunkId, OverlayEntry>,
    bytes: usize,
    cpu_bytes: usize,
}

impl OverlayCache {
    fn new(soft_limit_bytes: usize, hard_limit_bytes: usize) -> Self {
        Self {
            soft_limit_bytes,
            hard_limit_bytes,
            entries: HashMap::new(),
            bytes: 0,
            cpu_bytes: 0,
        }
    }

    fn clear(&mut self) {
        self.entries.clear();
        self.bytes = 0;
        self.cpu_bytes = 0;
    }

    fn contains(&self, id: OverlayChunkId) -> bool {
        self.entries.contains_key(&id)
    }

    fn len(&self) -> usize {
        self.entries.len()
    }

    fn entry(&self, id: OverlayChunkId) -> Option<&OverlayEntry> {
        self.entries.get(&id)
    }

    fn touch(&mut self, id: OverlayChunkId, frame_index: u64) {
        if let Some(entry) = self.entries.get_mut(&id) {
            entry.last_used_frame = frame_index;
        }
    }

    fn insert_chunk(
        &mut self,
        device: &wgpu::Device,
        id: OverlayChunkId,
        bytes: &[u8],
        frame_index: u64,
    ) -> Result<(), JsValue> {
        if let Some(old) = self.entries.remove(&id) {
            self.bytes = self.bytes.saturating_sub(old.bytes);
            self.cpu_bytes = self.cpu_bytes.saturating_sub(old.cpu_bytes);
        }

        let decoded = decode_overlay_chunk(id, bytes)?;
        let point_bytes = bytemuck::cast_slice(&decoded.points);
        let stroke_bytes = bytemuck::cast_slice(&decoded.strokes);
        let point_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("fovea-overlay-points"),
            contents: point_bytes,
            usage: wgpu::BufferUsages::VERTEX,
        });
        let stroke_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("fovea-overlay-strokes"),
            contents: stroke_bytes,
            usage: wgpu::BufferUsages::VERTEX,
        });
        let entry_bytes = point_bytes.len() + stroke_bytes.len();
        let cpu_bytes = decoded.cells.len() * std::mem::size_of::<CellPickRecord>()
            + decoded.polygon_points.len() * std::mem::size_of::<[f32; 2]>();

        self.entries.insert(
            id,
            OverlayEntry {
                point_buffer,
                stroke_buffer,
                point_count: decoded.points.len() as u32,
                stroke_vertex_count: decoded.strokes.len() as u32,
                cells: decoded.cells,
                polygon_points: decoded.polygon_points,
                bytes: entry_bytes,
                cpu_bytes,
                last_used_frame: frame_index,
            },
        );
        self.bytes += entry_bytes;
        self.cpu_bytes += cpu_bytes;
        Ok(())
    }

    fn evict(&mut self, visible_ids: &HashSet<OverlayChunkId>) {
        if self.bytes <= self.hard_limit_bytes && self.bytes <= self.soft_limit_bytes {
            return;
        }

        let mut candidates: Vec<_> = self
            .entries
            .iter()
            .filter(|(id, _)| !visible_ids.contains(id))
            .map(|(id, entry)| (*id, entry.last_used_frame))
            .collect();
        candidates.sort_by_key(|(_, last_used_frame)| *last_used_frame);

        let target = self.soft_limit_bytes.min(self.hard_limit_bytes);
        for (id, _) in candidates {
            if self.bytes <= target {
                break;
            }

            if let Some(entry) = self.entries.remove(&id) {
                self.bytes = self.bytes.saturating_sub(entry.bytes);
                self.cpu_bytes = self.cpu_bytes.saturating_sub(entry.cpu_bytes);
            }
        }
    }
}

struct OverlayEntry {
    point_buffer: wgpu::Buffer,
    stroke_buffer: wgpu::Buffer,
    point_count: u32,
    stroke_vertex_count: u32,
    cells: Vec<CellPickRecord>,
    polygon_points: Vec<[f32; 2]>,
    bytes: usize,
    cpu_bytes: usize,
    last_used_frame: u64,
}

impl OverlayEntry {
    fn cell_contains_point(&self, cell: &CellPickRecord, x: f32, y: f32) -> bool {
        let start = cell.vertex_offset as usize;
        let len = usize::from(cell.vertex_count);

        if len < 3 || start + len > self.polygon_points.len() {
            return false;
        }

        point_in_polygon(x, y, &self.polygon_points[start..start + len])
    }
}

#[derive(Clone)]
struct CellHit {
    cell_id: u64,
    class_id: u32,
    centroid: [f32; 2],
}

struct DecodedOverlayChunk {
    points: Vec<OverlayPointVertex>,
    strokes: Vec<OverlayStrokeVertex>,
    cells: Vec<CellPickRecord>,
    polygon_points: Vec<[f32; 2]>,
}

#[derive(Clone, Copy)]
struct OverlayCellHeader {
    class_id: u32,
    vertex_count: u16,
    vertex_offset: u32,
}

struct CellPickRecord {
    cell_id: u64,
    class_id: u32,
    centroid: [f32; 2],
    bbox: [f32; 4],
    vertex_offset: u32,
    vertex_count: u16,
}

impl CellPickRecord {
    fn bbox_intersects_point(&self, x: f32, y: f32, padding: f32) -> bool {
        x >= self.bbox[0] - padding
            && x <= self.bbox[2] + padding
            && y >= self.bbox[1] - padding
            && y <= self.bbox[3] + padding
    }

    fn screen_distance_squared(&self, camera: &Camera, screen_x: f64, screen_y: f64) -> f64 {
        let centroid =
            camera.slide_to_screen(f64::from(self.centroid[0]), f64::from(self.centroid[1]));
        (centroid.0 - screen_x).powi(2) + (centroid.1 - screen_y).powi(2)
    }
}

fn decode_overlay_chunk(id: OverlayChunkId, bytes: &[u8]) -> Result<DecodedOverlayChunk, JsValue> {
    let mut reader = ByteReader::new(bytes);
    let magic = reader.read_bytes(4)?;

    if magic != b"FOVC" {
        return Err(js_error("overlay chunk has invalid magic"));
    }

    let version = reader.read_u32()?;
    if version != 1 {
        return Err(js_error("unsupported overlay chunk version"));
    }

    let chunk_x = reader.read_u32()?;
    let chunk_y = reader.read_u32()?;

    if chunk_x != id.x || chunk_y != id.y {
        return Err(js_error("overlay chunk coordinates do not match request"));
    }

    let origin_x = reader.read_f32()?;
    let origin_y = reader.read_f32()?;
    let chunk_width = reader.read_f32()?;
    let chunk_height = reader.read_f32()?;
    let cell_count = reader.read_u32()? as usize;
    let polygon_vertex_count = reader.read_u32()? as usize;
    let mut points = Vec::with_capacity(cell_count);
    let mut headers = Vec::with_capacity(cell_count);
    let mut cells = Vec::with_capacity(cell_count);

    for _ in 0..cell_count {
        let cell_id = reader.read_u64()?;
        let class_id = u32::from(reader.read_u16()?);
        let vertex_count = reader.read_u16()?;
        let _confidence = reader.read_f32()?;
        let centroid_x = reader.read_u16()?;
        let centroid_y = reader.read_u16()?;
        let vertex_offset = reader.read_u32()?;
        let bbox_min_x = reader.read_u16()?;
        let bbox_min_y = reader.read_u16()?;
        let bbox_max_x = reader.read_u16()?;
        let bbox_max_y = reader.read_u16()?;
        let centroid = [
            dequantize(centroid_x, origin_x, chunk_width),
            dequantize(centroid_y, origin_y, chunk_height),
        ];
        let bbox = [
            dequantize(bbox_min_x, origin_x, chunk_width),
            dequantize(bbox_min_y, origin_y, chunk_height),
            dequantize(bbox_max_x, origin_x, chunk_width),
            dequantize(bbox_max_y, origin_y, chunk_height),
        ];
        points.push(OverlayPointVertex {
            position: centroid,
            class_id,
            _pad: 0,
        });
        headers.push(OverlayCellHeader {
            class_id,
            vertex_count,
            vertex_offset,
        });
        cells.push(CellPickRecord {
            cell_id,
            class_id,
            centroid,
            bbox,
            vertex_offset,
            vertex_count,
        });
    }

    let mut polygon_points = Vec::with_capacity(polygon_vertex_count);
    for _ in 0..polygon_vertex_count {
        let x = reader.read_u16()?;
        let y = reader.read_u16()?;
        polygon_points.push([
            dequantize(x, origin_x, chunk_width),
            dequantize(y, origin_y, chunk_height),
        ]);
    }

    let mut strokes = Vec::new();
    for header in headers {
        let start = header.vertex_offset as usize;
        let len = usize::from(header.vertex_count);

        if len < 2 || start + len > polygon_points.len() {
            continue;
        }

        for index in 0..len {
            let a = polygon_points[start + index];
            let b = polygon_points[start + ((index + 1) % len)];
            push_stroke_segment(&mut strokes, a, b, header.class_id);
        }
    }

    Ok(DecodedOverlayChunk {
        points,
        strokes,
        cells,
        polygon_points,
    })
}

fn push_stroke_segment(
    strokes: &mut Vec<OverlayStrokeVertex>,
    a: [f32; 2],
    b: [f32; 2],
    class_id: u32,
) {
    strokes.extend_from_slice(&[
        OverlayStrokeVertex {
            segment_start: a,
            segment_end: b,
            endpoint: 0.0,
            side: -1.0,
            class_id,
            _pad: 0,
        },
        OverlayStrokeVertex {
            segment_start: a,
            segment_end: b,
            endpoint: 1.0,
            side: -1.0,
            class_id,
            _pad: 0,
        },
        OverlayStrokeVertex {
            segment_start: a,
            segment_end: b,
            endpoint: 1.0,
            side: 1.0,
            class_id,
            _pad: 0,
        },
        OverlayStrokeVertex {
            segment_start: a,
            segment_end: b,
            endpoint: 0.0,
            side: -1.0,
            class_id,
            _pad: 0,
        },
        OverlayStrokeVertex {
            segment_start: a,
            segment_end: b,
            endpoint: 1.0,
            side: 1.0,
            class_id,
            _pad: 0,
        },
        OverlayStrokeVertex {
            segment_start: a,
            segment_end: b,
            endpoint: 0.0,
            side: 1.0,
            class_id,
            _pad: 0,
        },
    ]);
}

fn point_in_polygon(x: f32, y: f32, polygon: &[[f32; 2]]) -> bool {
    let mut inside = false;
    let mut j = polygon.len() - 1;

    for i in 0..polygon.len() {
        let yi = polygon[i][1];
        let yj = polygon[j][1];

        if (yi > y) != (yj > y) {
            let xi = polygon[i][0];
            let xj = polygon[j][0];
            let intersection_x = (xj - xi) * (y - yi) / (yj - yi) + xi;

            if x < intersection_x {
                inside = !inside;
            }
        }

        j = i;
    }

    inside
}

fn dequantize(value: u16, origin: f32, size: f32) -> f32 {
    origin + (f32::from(value) / f32::from(u16::MAX)) * size
}

struct ByteReader<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> ByteReader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }

    fn read_bytes(&mut self, len: usize) -> Result<&'a [u8], JsValue> {
        if self.offset + len > self.bytes.len() {
            return Err(js_error("overlay chunk ended unexpectedly"));
        }

        let start = self.offset;
        self.offset += len;
        Ok(&self.bytes[start..self.offset])
    }

    fn read_u16(&mut self) -> Result<u16, JsValue> {
        let bytes = self.read_bytes(2)?;
        Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
    }

    fn read_u32(&mut self) -> Result<u32, JsValue> {
        let bytes = self.read_bytes(4)?;
        Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    fn read_u64(&mut self) -> Result<u64, JsValue> {
        let bytes = self.read_bytes(8)?;
        Ok(u64::from_le_bytes([
            bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        ]))
    }

    fn read_f32(&mut self) -> Result<f32, JsValue> {
        Ok(f32::from_bits(self.read_u32()?))
    }
}

struct TextureCache {
    soft_limit_bytes: usize,
    hard_limit_bytes: usize,
    entries: HashMap<TileId, TextureEntry>,
    bytes: usize,
}

impl TextureCache {
    fn new(soft_limit_bytes: usize, hard_limit_bytes: usize) -> Self {
        Self {
            soft_limit_bytes,
            hard_limit_bytes,
            entries: HashMap::new(),
            bytes: 0,
        }
    }

    fn clear(&mut self) {
        self.entries.clear();
        self.bytes = 0;
    }

    fn contains(&self, id: TileId) -> bool {
        self.entries.contains_key(&id)
    }

    fn len(&self) -> usize {
        self.entries.len()
    }

    fn entry(&self, id: TileId) -> Option<&TextureEntry> {
        self.entries.get(&id)
    }

    fn touch(&mut self, id: TileId, frame_index: u64) {
        if let Some(entry) = self.entries.get_mut(&id) {
            entry.last_used_frame = frame_index;
        }
    }

    fn insert_rgba(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        layout: &wgpu::BindGroupLayout,
        id: TileId,
        width: u32,
        height: u32,
        rgba: &[u8],
        frame_index: u64,
    ) {
        if let Some(old) = self.entries.remove(&id) {
            self.bytes = self.bytes.saturating_sub(old.bytes);
        }

        let texture_size = wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        };
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("fovea-slide-tile"),
            size: texture_size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(width * 4),
                rows_per_image: Some(height),
            },
            texture_size,
        );
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("fovea-slide-tile-sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("fovea-slide-tile-bind-group"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });
        let bytes = width as usize * height as usize * 4;

        self.entries.insert(
            id,
            TextureEntry {
                texture,
                view,
                sampler,
                bind_group,
                width,
                height,
                bytes,
                last_used_frame: frame_index,
            },
        );
        self.bytes += bytes;
    }

    fn insert_r8(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        layout: &wgpu::BindGroupLayout,
        id: TileId,
        width: u32,
        height: u32,
        values: &[u8],
        frame_index: u64,
    ) {
        if let Some(old) = self.entries.remove(&id) {
            self.bytes = self.bytes.saturating_sub(old.bytes);
        }

        let texture_size = wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        };
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("fovea-heatmap-tile"),
            size: texture_size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            values,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(width),
                rows_per_image: Some(height),
            },
            texture_size,
        );
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("fovea-heatmap-tile-sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("fovea-heatmap-tile-bind-group"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });
        let bytes = width as usize * height as usize;

        self.entries.insert(
            id,
            TextureEntry {
                texture,
                view,
                sampler,
                bind_group,
                width,
                height,
                bytes,
                last_used_frame: frame_index,
            },
        );
        self.bytes += bytes;
    }

    fn evict(
        &mut self,
        visible_ids: &HashSet<TileId>,
        camera: &Camera,
        manifest: Option<&SlideManifest>,
    ) {
        if self.bytes <= self.hard_limit_bytes && self.bytes <= self.soft_limit_bytes {
            return;
        }

        let mut candidates: Vec<_> = self
            .entries
            .keys()
            .copied()
            .filter(|id| !visible_ids.contains(id))
            .collect();

        candidates.sort_by(|left, right| {
            let left_score = eviction_score(*left, camera, manifest, self.entries.get(left));
            let right_score = eviction_score(*right, camera, manifest, self.entries.get(right));
            right_score
                .partial_cmp(&left_score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        let target = self.soft_limit_bytes.min(self.hard_limit_bytes);
        for id in candidates {
            if self.bytes <= target {
                break;
            }

            if let Some(entry) = self.entries.remove(&id) {
                self.bytes = self.bytes.saturating_sub(entry.bytes);
            }
        }
    }
}

fn eviction_score(
    id: TileId,
    camera: &Camera,
    manifest: Option<&SlideManifest>,
    entry: Option<&TextureEntry>,
) -> f64 {
    let high_resolution_bias = f64::from(u32::MAX - id.level) * 1_000_000_000.0;
    let age_bias = entry
        .map(|entry| entry.last_used_frame as f64)
        .unwrap_or_default()
        * -1.0;
    let distance = manifest
        .and_then(|manifest| {
            let level = manifest.level(id.level)?;
            let tile = manifest.tiles.get(&id)?;
            let rect = manifest.tile_world_rect(level, tile);
            let center = rect.center();
            Some((center.0 - camera.center_x).powi(2) + (center.1 - camera.center_y).powi(2))
        })
        .unwrap_or_default();

    high_resolution_bias + distance + age_bias
}

struct TextureEntry {
    #[allow(dead_code)]
    texture: wgpu::Texture,
    #[allow(dead_code)]
    view: wgpu::TextureView,
    #[allow(dead_code)]
    sampler: wgpu::Sampler,
    bind_group: wgpu::BindGroup,
    #[allow(dead_code)]
    width: u32,
    #[allow(dead_code)]
    height: u32,
    bytes: usize,
    last_used_frame: u64,
}

fn create_triangle_pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    format: wgpu::TextureFormat,
) -> wgpu::RenderPipeline {
    create_pipeline(
        device,
        "fovea-triangle-pipeline",
        layout,
        shader,
        "vs_triangle",
        "fs_triangle",
        format,
        &[],
        wgpu::PrimitiveTopology::TriangleList,
    )
}

fn create_tile_pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    format: wgpu::TextureFormat,
) -> wgpu::RenderPipeline {
    create_pipeline(
        device,
        "fovea-tile-pipeline",
        layout,
        shader,
        "vs_tile",
        "fs_tile",
        format,
        &[TileVertex::layout()],
        wgpu::PrimitiveTopology::TriangleList,
    )
}

fn create_heatmap_pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    format: wgpu::TextureFormat,
) -> wgpu::RenderPipeline {
    create_pipeline(
        device,
        "fovea-heatmap-pipeline",
        layout,
        shader,
        "vs_tile",
        "fs_heatmap",
        format,
        &[TileVertex::layout()],
        wgpu::PrimitiveTopology::TriangleList,
    )
}

fn create_point_pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    format: wgpu::TextureFormat,
) -> wgpu::RenderPipeline {
    create_pipeline(
        device,
        "fovea-point-pipeline",
        layout,
        shader,
        "vs_point",
        "fs_point",
        format,
        &[PointVertex::layout()],
        wgpu::PrimitiveTopology::PointList,
    )
}

fn create_overlay_point_pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    format: wgpu::TextureFormat,
) -> wgpu::RenderPipeline {
    create_pipeline(
        device,
        "fovea-overlay-point-pipeline",
        layout,
        shader,
        "vs_overlay_point",
        "fs_point",
        format,
        &[OverlayPointVertex::layout()],
        wgpu::PrimitiveTopology::TriangleList,
    )
}

fn create_overlay_line_pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    format: wgpu::TextureFormat,
) -> wgpu::RenderPipeline {
    create_pipeline(
        device,
        "fovea-overlay-line-pipeline",
        layout,
        shader,
        "vs_overlay_line",
        "fs_point",
        format,
        &[OverlayStrokeVertex::layout()],
        wgpu::PrimitiveTopology::TriangleList,
    )
}

fn create_pipeline(
    device: &wgpu::Device,
    label: &str,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    vertex_entry: &str,
    fragment_entry: &str,
    format: wgpu::TextureFormat,
    vertex_buffers: &[wgpu::VertexBufferLayout],
    topology: wgpu::PrimitiveTopology,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some(vertex_entry),
            buffers: vertex_buffers,
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some(fragment_entry),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        }),
        primitive: wgpu::PrimitiveState {
            topology,
            strip_index_format: None,
            front_face: wgpu::FrontFace::Ccw,
            cull_mode: None,
            polygon_mode: wgpu::PolygonMode::Fill,
            unclipped_depth: false,
            conservative: false,
        },
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    })
}

fn surface_texture_error(err: wgpu::CurrentSurfaceTexture) -> JsValue {
    js_error(&format!("surface texture error: {err:?}"))
}

#[cfg(target_arch = "wasm32")]
fn create_canvas_surface(
    instance: &wgpu::Instance,
    canvas: HtmlCanvasElement,
) -> Result<wgpu::Surface<'static>, JsValue> {
    instance
        .create_surface(wgpu::SurfaceTarget::Canvas(canvas))
        .map_err(|err| js_error(&format!("failed to create WebGPU surface: {err:?}")))
}

#[cfg(not(target_arch = "wasm32"))]
fn create_canvas_surface(
    _instance: &wgpu::Instance,
    _canvas: HtmlCanvasElement,
) -> Result<wgpu::Surface<'static>, JsValue> {
    Err(js_error(
        "FoveaViewer WebGPU canvas surfaces are only available on wasm32",
    ))
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct CameraUniform {
    center: [f32; 2],
    zoom: f32,
    _pad0: f32,
    viewport: [f32; 2],
    _pad1: [f32; 2],
    overlay: [f32; 4],
    heatmap: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct TileVertex {
    position: [f32; 2],
    uv: [f32; 2],
}

impl TileVertex {
    fn new(x: f32, y: f32, u: f32, v: f32) -> Self {
        Self {
            position: [x, y],
            uv: [u, v],
        }
    }

    fn layout<'a>() -> wgpu::VertexBufferLayout<'a> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<TileVertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &[
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x2,
                    offset: 0,
                    shader_location: 0,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x2,
                    offset: std::mem::size_of::<[f32; 2]>() as wgpu::BufferAddress,
                    shader_location: 1,
                },
            ],
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct PointVertex {
    position: [f32; 2],
}

impl PointVertex {
    fn layout<'a>() -> wgpu::VertexBufferLayout<'a> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<PointVertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &[wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Float32x2,
                offset: 0,
                shader_location: 0,
            }],
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct OverlayPointVertex {
    position: [f32; 2],
    class_id: u32,
    _pad: u32,
}

impl OverlayPointVertex {
    fn layout<'a>() -> wgpu::VertexBufferLayout<'a> {
        overlay_vertex_layout::<OverlayPointVertex>(wgpu::VertexStepMode::Instance)
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct OverlayStrokeVertex {
    segment_start: [f32; 2],
    segment_end: [f32; 2],
    endpoint: f32,
    side: f32,
    class_id: u32,
    _pad: u32,
}

impl OverlayStrokeVertex {
    fn layout<'a>() -> wgpu::VertexBufferLayout<'a> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<OverlayStrokeVertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &[
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x2,
                    offset: 0,
                    shader_location: 0,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x2,
                    offset: std::mem::size_of::<[f32; 2]>() as wgpu::BufferAddress,
                    shader_location: 1,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32,
                    offset: (std::mem::size_of::<[f32; 2]>() * 2) as wgpu::BufferAddress,
                    shader_location: 2,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32,
                    offset: (std::mem::size_of::<[f32; 2]>() * 2 + std::mem::size_of::<f32>())
                        as wgpu::BufferAddress,
                    shader_location: 3,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Uint32,
                    offset: (std::mem::size_of::<[f32; 2]>() * 2 + std::mem::size_of::<f32>() * 2)
                        as wgpu::BufferAddress,
                    shader_location: 4,
                },
            ],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EPSILON: f64 = 0.25;

    #[test]
    fn camera_round_trip_preserves_slide_coordinates() {
        let camera = Camera::fit_dimensions(800, 600, 10_000.0, 8_000.0);
        let slide = (4_321.25, 3_456.75);
        let screen = camera.slide_to_screen(slide.0, slide.1);
        let round_trip = camera.screen_to_slide(screen.0, screen.1);

        assert_close(round_trip.0, slide.0, EPSILON);
        assert_close(round_trip.1, slide.1, EPSILON);
    }

    #[test]
    fn zoom_at_keeps_anchor_slide_coordinate_stable() {
        let mut camera = Camera::fit_dimensions(800, 600, 10_000.0, 8_000.0);
        let anchor = (520.0, 375.0);
        let before = camera.screen_to_slide(anchor.0, anchor.1);

        camera.zoom_at(anchor.0, anchor.1, -240.0);

        let after = camera.screen_to_slide(anchor.0, anchor.1);
        assert_close(after.0, before.0, EPSILON);
        assert_close(after.1, before.1, EPSILON);
    }

    #[test]
    fn slide_manifest_rejects_empty_pyramid() {
        let manifest = r#"{
            "tile_size": 256,
            "width": 1024,
            "height": 768,
            "levels": [],
            "tiles": []
        }"#;

        assert!(SlideManifest::from_json(manifest).is_err());
    }

    #[test]
    fn slide_tile_culling_returns_visible_edge_tiles() {
        let manifest = SlideManifest::from_json(
            r#"{
                "tile_size": 256,
                "width": 512,
                "height": 512,
                "levels": [
                    {
                        "index": 0,
                        "width": 512,
                        "height": 512,
                        "downsample": 1.0,
                        "tile_cols": 2,
                        "tile_rows": 2,
                        "tile_count": 4
                    }
                ],
                "tiles": [
                    {"level":0,"x":0,"y":0,"width":256,"height":256,"path":"0_0.webp","byte_size":1,"skipped":false},
                    {"level":0,"x":1,"y":0,"width":256,"height":256,"path":"1_0.webp","byte_size":1,"skipped":false},
                    {"level":0,"x":0,"y":1,"width":256,"height":256,"path":"0_1.webp","byte_size":1,"skipped":false},
                    {"level":0,"x":1,"y":1,"width":256,"height":256,"path":"1_1.webp","byte_size":1,"skipped":false}
                ]
            }"#,
        )
        .expect("manifest parses");
        let mut camera = Camera::fit_dimensions(256, 256, 512.0, 512.0);
        camera.center_x = 256.0;
        camera.center_y = 256.0;
        camera.zoom = 1.0;

        let tiles = manifest.visible_tiles(&camera, manifest.level(0).expect("level 0 exists"));
        let ids: HashSet<_> = tiles.iter().map(|tile| tile.id).collect();

        assert_eq!(ids.len(), 4);
        assert!(ids.contains(&TileId {
            level: 0,
            x: 0,
            y: 0
        }));
        assert!(ids.contains(&TileId {
            level: 0,
            x: 1,
            y: 1
        }));
    }

    #[test]
    fn heatmap_manifest_accepts_camel_case_tiles() {
        let manifest = HeatmapManifest::from_json(
            r#"{
                "id": "density",
                "width": 1024,
                "height": 768,
                "tileSize": 256,
                "levels": [
                    {
                        "index": 0,
                        "width": 8,
                        "height": 6,
                        "downsample": 128.0,
                        "tileCols": 1,
                        "tileRows": 1
                    }
                ],
                "tiles": [
                    {
                        "level": 0,
                        "x": 0,
                        "y": 0,
                        "width": 8,
                        "height": 6,
                        "path": "tiles/0/0_0.fovh",
                        "byteSize": 48
                    }
                ]
            }"#,
        )
        .expect("heatmap manifest parses");

        assert_eq!(manifest.tile_size, 256);
        assert_eq!(manifest.tiles.len(), 1);
    }

    #[test]
    fn cell_overlay_manifest_rejects_zero_chunk_size() {
        let manifest = r#"{
            "id": "cells",
            "width": 1024,
            "height": 768,
            "chunkWidth": 0,
            "chunkHeight": 256,
            "chunks": []
        }"#;

        assert!(CellOverlayManifest::from_json(manifest).is_err());
    }

    #[test]
    fn overlay_chunk_parser_decodes_cells_and_polygon_strokes() {
        let bytes = overlay_chunk_bytes();
        let decoded = decode_overlay_chunk(OverlayChunkId { x: 2, y: 3 }, &bytes)
            .expect("overlay chunk parses");

        assert_eq!(decoded.points.len(), 1);
        assert_eq!(decoded.cells.len(), 1);
        assert_eq!(decoded.polygon_points.len(), 4);
        assert_eq!(decoded.strokes.len(), 24);
        assert_eq!(decoded.cells[0].cell_id, 42);
        assert_eq!(decoded.cells[0].class_id, 7);
        assert_close(f64::from(decoded.cells[0].centroid[0]), 600.0, 0.02);
        assert_close(f64::from(decoded.cells[0].centroid[1]), 850.0, 0.02);
    }

    #[test]
    fn quantization_round_trip_error_stays_below_quarter_pixel() {
        let origin = 512.0;
        let size = 4096.0;
        let value = 2345.625;
        let quantized = quantize_for_test(value, origin, size);
        let restored = f64::from(dequantize(quantized, origin, size));

        assert_close(restored, f64::from(value), 0.07);
    }

    #[test]
    fn point_in_polygon_identifies_inside_and_outside_points() {
        let polygon = [[10.0, 10.0], [30.0, 10.0], [30.0, 30.0], [10.0, 30.0]];

        assert!(point_in_polygon(20.0, 20.0, &polygon));
        assert!(!point_in_polygon(40.0, 20.0, &polygon));
    }

    #[test]
    fn cell_pick_distance_uses_camera_screen_coordinates() {
        let camera = Camera::fit_dimensions(400, 300, 1_000.0, 1_000.0);
        let cell = CellPickRecord {
            cell_id: 1,
            class_id: 2,
            centroid: [500.0, 500.0],
            bbox: [490.0, 490.0, 510.0, 510.0],
            vertex_offset: 0,
            vertex_count: 0,
        };
        let screen = camera.slide_to_screen(500.0, 500.0);

        assert_close(
            cell.screen_distance_squared(&camera, screen.0, screen.1),
            0.0,
            0.001,
        );
        assert!(cell.bbox_intersects_point(500.0, 500.0, 0.0));
        assert!(!cell.bbox_intersects_point(540.0, 500.0, 8.0));
    }

    fn overlay_chunk_bytes() -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"FOVC");
        push_u32(&mut bytes, 1);
        push_u32(&mut bytes, 2);
        push_u32(&mut bytes, 3);
        push_f32(&mut bytes, 512.0);
        push_f32(&mut bytes, 768.0);
        push_f32(&mut bytes, 1024.0);
        push_f32(&mut bytes, 1024.0);
        push_u32(&mut bytes, 1);
        push_u32(&mut bytes, 4);

        push_u64(&mut bytes, 42);
        push_u16(&mut bytes, 7);
        push_u16(&mut bytes, 4);
        push_f32(&mut bytes, 0.95);
        push_u16(&mut bytes, quantize_for_test(600.0, 512.0, 1024.0));
        push_u16(&mut bytes, quantize_for_test(850.0, 768.0, 1024.0));
        push_u32(&mut bytes, 0);
        push_u16(&mut bytes, quantize_for_test(580.0, 512.0, 1024.0));
        push_u16(&mut bytes, quantize_for_test(830.0, 768.0, 1024.0));
        push_u16(&mut bytes, quantize_for_test(620.0, 512.0, 1024.0));
        push_u16(&mut bytes, quantize_for_test(870.0, 768.0, 1024.0));

        for (x, y) in [
            (580.0, 830.0),
            (620.0, 830.0),
            (620.0, 870.0),
            (580.0, 870.0),
        ] {
            push_u16(&mut bytes, quantize_for_test(x, 512.0, 1024.0));
            push_u16(&mut bytes, quantize_for_test(y, 768.0, 1024.0));
        }

        bytes
    }

    fn quantize_for_test(value: f32, origin: f32, size: f32) -> u16 {
        (((value - origin) / size).clamp(0.0, 1.0) * f32::from(u16::MAX)).round() as u16
    }

    fn push_u16(bytes: &mut Vec<u8>, value: u16) {
        bytes.extend_from_slice(&value.to_le_bytes());
    }

    fn push_u32(bytes: &mut Vec<u8>, value: u32) {
        bytes.extend_from_slice(&value.to_le_bytes());
    }

    fn push_u64(bytes: &mut Vec<u8>, value: u64) {
        bytes.extend_from_slice(&value.to_le_bytes());
    }

    fn push_f32(bytes: &mut Vec<u8>, value: f32) {
        bytes.extend_from_slice(&value.to_bits().to_le_bytes());
    }

    fn assert_close(actual: f64, expected: f64, tolerance: f64) {
        let delta = (actual - expected).abs();
        assert!(
            delta <= tolerance,
            "actual {actual} differs from expected {expected} by {delta}, tolerance {tolerance}"
        );
    }
}

fn overlay_vertex_layout<'a, T>(step_mode: wgpu::VertexStepMode) -> wgpu::VertexBufferLayout<'a> {
    wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<T>() as wgpu::BufferAddress,
        step_mode,
        attributes: &[
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Float32x2,
                offset: 0,
                shader_location: 0,
            },
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Uint32,
                offset: std::mem::size_of::<[f32; 2]>() as wgpu::BufferAddress,
                shader_location: 1,
            },
        ],
    }
}
