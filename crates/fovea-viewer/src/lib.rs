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

static PANIC_HOOK: Once = Once::new();

#[wasm_bindgen]
pub struct FoveaViewer {
    renderer: Renderer,
    camera: Camera,
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

        Ok(Self { renderer, camera })
    }

    pub fn resize(&mut self, width: u32, height: u32, device_pixel_ratio: f64) {
        self.renderer.resize(width, height);
        self.camera.viewport_width_px = width;
        self.camera.viewport_height_px = height;
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
    }

    #[wasm_bindgen(js_name = zoomAt)]
    pub fn zoom_at(&mut self, screen_x: f64, screen_y: f64, wheel_delta_y: f64) {
        self.camera.zoom_at(screen_x, screen_y, wheel_delta_y);
        self.camera.clamp_to_world();
        self.renderer.write_camera(&self.camera);
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

    pub fn render(&mut self) -> Result<FrameStats, JsValue> {
        self.renderer.render(&self.camera)
    }
}

#[wasm_bindgen]
pub struct FrameStats {
    frame_time_ms: f64,
    upload_time_ms: f64,
    draw_call_count: u32,
    visible_object_count: u32,
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

    fn as_uniform(&self) -> CameraUniform {
        CameraUniform {
            center: [self.center_x as f32, self.center_y as f32],
            zoom: self.zoom as f32,
            _pad0: 0.0,
            viewport: [
                self.viewport_width_px as f32,
                self.viewport_height_px as f32,
            ],
            _pad1: [0.0, 0.0],
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
            .map_err(|err| JsValue::from_str(&format!("failed to parse manifest: {err}")))?;
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
            return Err(JsValue::from_str("manifest has no pyramid levels"));
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
    point_pipeline: wgpu::RenderPipeline,
    camera_buffer: wgpu::Buffer,
    camera_bind_group: wgpu::BindGroup,
    texture_bind_group_layout: wgpu::BindGroupLayout,
    point_buffer: Option<wgpu::Buffer>,
    point_count: u32,
    slide_manifest: Option<SlideManifest>,
    texture_cache: TextureCache,
    frame_index: u64,
    last_upload_time_ms: f64,
    gpu_buffer_memory_bytes: u32,
    cpu_memory_bytes: u32,
}

impl Renderer {
    async fn new(canvas: HtmlCanvasElement) -> Result<Self, JsValue> {
        let width = canvas.width().max(1);
        let height = canvas.height().max(1);

        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::BROWSER_WEBGPU,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let surface = instance
            .create_surface(wgpu::SurfaceTarget::Canvas(canvas))
            .map_err(|err| {
                JsValue::from_str(&format!("failed to create WebGPU surface: {err:?}"))
            })?;
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
            })
            .await
            .map_err(|err| JsValue::from_str(&format!("WebGPU adapter unavailable: {err:?}")))?;

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
            .map_err(|err| {
                JsValue::from_str(&format!("failed to request WebGPU device: {err:?}"))
            })?;

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

        let camera_uniform =
            Camera::fit_dimensions(width, height, WORLD_SIZE, WORLD_SIZE).as_uniform();
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
                    visibility: wgpu::ShaderStages::VERTEX,
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

        let triangle_pipeline =
            create_triangle_pipeline(&device, &triangle_pipeline_layout, &shader, format);
        let tile_pipeline = create_tile_pipeline(&device, &tile_pipeline_layout, &shader, format);
        let point_pipeline =
            create_point_pipeline(&device, &point_pipeline_layout, &shader, format);

        Ok(Self {
            surface,
            device,
            queue,
            config,
            width,
            height,
            triangle_pipeline,
            tile_pipeline,
            point_pipeline,
            camera_buffer,
            camera_bind_group,
            texture_bind_group_layout,
            point_buffer: None,
            point_count: 0,
            slide_manifest: None,
            texture_cache: TextureCache::new(
                TILE_CACHE_SOFT_LIMIT_BYTES,
                TILE_CACHE_HARD_LIMIT_BYTES,
            ),
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

    fn slide_dimensions(&self) -> Option<(f64, f64)> {
        self.slide_manifest
            .as_ref()
            .map(|manifest| (manifest.width, manifest.height))
    }

    fn write_camera(&self, camera: &Camera) {
        self.queue.write_buffer(
            &self.camera_buffer,
            0,
            bytemuck::bytes_of(&camera.as_uniform()),
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
            return Err(JsValue::from_str(&format!(
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

    fn render(&mut self, camera: &Camera) -> Result<FrameStats, JsValue> {
        let start = Date::now();
        self.frame_index = self.frame_index.wrapping_add(1);
        let slide_draw = self.prepare_slide_draw(camera);
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
            } else if self.slide_manifest.is_none() {
                pass.set_pipeline(&self.triangle_pipeline);
                pass.draw(0..3, 0..1);

                if let Some(point_buffer) = &self.point_buffer {
                    pass.set_pipeline(&self.point_pipeline);
                    pass.set_bind_group(0, &self.camera_bind_group, &[]);
                    pass.set_vertex_buffer(0, point_buffer.slice(..));
                    pass.draw(0..self.point_count, 0..1);
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
        self.update_memory_stats(0);

        Ok(FrameStats {
            frame_time_ms: Date::now() - start,
            upload_time_ms: self.last_upload_time_ms,
            draw_call_count: slide_draw
                .commands
                .len()
                .max(if self.slide_manifest.is_some() { 0 } else { 2 })
                as u32,
            visible_object_count: if self.slide_manifest.is_some() {
                slide_draw.visible_tile_count as u32
            } else {
                self.point_count
            },
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

    fn update_memory_stats(&mut self, extra_cpu_bytes: usize) {
        let point_bytes = self
            .point_count
            .checked_mul(std::mem::size_of::<PointVertex>() as u32)
            .unwrap_or(u32::MAX);
        let gpu_bytes = std::mem::size_of::<CameraUniform>() as usize
            + self.texture_cache.bytes
            + point_bytes as usize;

        self.gpu_buffer_memory_bytes = gpu_bytes.min(u32::MAX as usize) as u32;
        self.cpu_memory_bytes = extra_cpu_bytes.min(u32::MAX as usize) as u32;
    }
}

#[derive(Default)]
struct SlideDraw {
    vertices: Vec<TileVertex>,
    commands: Vec<DrawCommand>,
    cache_visible_ids: HashSet<TileId>,
    visible_tile_count: usize,
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
    JsValue::from_str(&format!("surface texture error: {err:?}"))
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct CameraUniform {
    center: [f32; 2],
    zoom: f32,
    _pad0: f32,
    viewport: [f32; 2],
    _pad1: [f32; 2],
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
