use std::sync::Once;

use bytemuck::{Pod, Zeroable};
use js_sys::Date;
use wasm_bindgen::prelude::*;
use web_sys::HtmlCanvasElement;
use wgpu::util::DeviceExt;

const WORLD_SIZE: f64 = 100_000.0;
const MAX_POINTS: u32 = 1_000_000;

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
        let camera = Camera::fit_world(renderer.width, renderer.height);
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

    #[wasm_bindgen(js_name = resetCamera)]
    pub fn reset_camera(&mut self) {
        self.camera = Camera::fit_world(self.renderer.width, self.renderer.height);
        self.renderer.write_camera(&self.camera);
    }

    #[wasm_bindgen(js_name = panByScreenDelta)]
    pub fn pan_by_screen_delta(&mut self, delta_x: f64, delta_y: f64) {
        self.camera.pan_by_screen_delta(delta_x, delta_y);
        self.renderer.write_camera(&self.camera);
    }

    #[wasm_bindgen(js_name = zoomAt)]
    pub fn zoom_at(&mut self, screen_x: f64, screen_y: f64, wheel_delta_y: f64) {
        self.camera.zoom_at(screen_x, screen_y, wheel_delta_y);
        self.renderer.write_camera(&self.camera);
    }

    #[wasm_bindgen(js_name = setPointCount)]
    pub fn set_point_count(&mut self, count: u32) -> Result<(), JsValue> {
        self.renderer.set_point_count(count.min(MAX_POINTS))
    }

    pub fn render(&mut self) -> Result<FrameStats, JsValue> {
        self.renderer.render()
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
}

impl Camera {
    fn fit_world(width: u32, height: u32) -> Self {
        let shortest_edge = width.min(height).max(1) as f64;
        let zoom = (shortest_edge * 0.9) / WORLD_SIZE;

        Self {
            center_x: WORLD_SIZE * 0.5,
            center_y: WORLD_SIZE * 0.5,
            zoom,
            viewport_width_px: width.max(1),
            viewport_height_px: height.max(1),
            device_pixel_ratio: 1.0,
        }
    }

    fn pan_by_screen_delta(&mut self, delta_x: f64, delta_y: f64) {
        self.center_x -= delta_x / self.zoom;
        self.center_y -= delta_y / self.zoom;
    }

    fn zoom_at(&mut self, screen_x: f64, screen_y: f64, wheel_delta_y: f64) {
        let before = self.screen_to_slide(screen_x, screen_y);
        let factor = (-wheel_delta_y * 0.001).exp();
        self.zoom = (self.zoom * factor).clamp(0.0005, 8.0);
        let after = self.screen_to_slide(screen_x, screen_y);

        self.center_x += before.0 - after.0;
        self.center_y += before.1 - after.1;
    }

    fn screen_to_slide(&self, screen_x: f64, screen_y: f64) -> (f64, f64) {
        let x = self.center_x + (screen_x - self.viewport_width_px as f64 * 0.5) / self.zoom;
        let y = self.center_y + (screen_y - self.viewport_height_px as f64 * 0.5) / self.zoom;
        (x, y)
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

struct Renderer {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    width: u32,
    height: u32,
    triangle_pipeline: wgpu::RenderPipeline,
    quad_pipeline: wgpu::RenderPipeline,
    point_pipeline: wgpu::RenderPipeline,
    camera_buffer: wgpu::Buffer,
    camera_bind_group: wgpu::BindGroup,
    quad_bind_group: wgpu::BindGroup,
    point_buffer: Option<wgpu::Buffer>,
    point_count: u32,
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
            ..Default::default()
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
            .ok_or_else(|| JsValue::from_str("WebGPU adapter unavailable"))?;

        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    label: Some("fovea-device"),
                    required_features: wgpu::Features::empty(),
                    required_limits: wgpu::Limits::default(),
                },
                None,
            )
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
            label: Some("fovea-phase0-shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/phase0.wgsl").into()),
        });

        let camera_uniform = Camera::fit_world(width, height).as_uniform();
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

        let quad_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("fovea-quad-texture-layout"),
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
        let quad_bind_group = create_quad_texture(&device, &queue, &quad_bind_group_layout);

        let triangle_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("fovea-triangle-pipeline-layout"),
                bind_group_layouts: &[],
                push_constant_ranges: &[],
            });
        let quad_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("fovea-quad-pipeline-layout"),
            bind_group_layouts: &[&camera_bind_group_layout, &quad_bind_group_layout],
            push_constant_ranges: &[],
        });
        let point_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("fovea-point-pipeline-layout"),
                bind_group_layouts: &[&camera_bind_group_layout],
                push_constant_ranges: &[],
            });

        let triangle_pipeline =
            create_triangle_pipeline(&device, &triangle_pipeline_layout, &shader, format);
        let quad_pipeline = create_quad_pipeline(&device, &quad_pipeline_layout, &shader, format);
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
            quad_pipeline,
            point_pipeline,
            camera_buffer,
            camera_bind_group,
            quad_bind_group,
            point_buffer: None,
            point_count: 0,
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
        self.gpu_buffer_memory_bytes =
            std::mem::size_of::<CameraUniform>() as u32 + bytes.len() as u32;
        self.cpu_memory_bytes = bytes.len() as u32;

        Ok(())
    }

    fn render(&mut self) -> Result<FrameStats, JsValue> {
        let start = Date::now();
        let output = match self.surface.get_current_texture() {
            Ok(output) => output,
            Err(wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated) => {
                self.surface.configure(&self.device, &self.config);
                self.surface.get_current_texture().map_err(surface_error)?
            }
            Err(err) => return Err(surface_error(err)),
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
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.05,
                            g: 0.055,
                            b: 0.06,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });

            pass.set_pipeline(&self.triangle_pipeline);
            pass.draw(0..3, 0..1);

            pass.set_pipeline(&self.quad_pipeline);
            pass.set_bind_group(1, &self.quad_bind_group, &[]);
            pass.draw(0..6, 0..1);

            if let Some(point_buffer) = &self.point_buffer {
                pass.set_pipeline(&self.point_pipeline);
                pass.set_bind_group(0, &self.camera_bind_group, &[]);
                pass.set_vertex_buffer(0, point_buffer.slice(..));
                pass.draw(0..self.point_count, 0..1);
            }
        }

        self.queue.submit(Some(encoder.finish()));
        output.present();

        Ok(FrameStats {
            frame_time_ms: Date::now() - start,
            upload_time_ms: self.last_upload_time_ms,
            draw_call_count: 3,
            visible_object_count: self.point_count,
            gpu_buffer_memory_bytes: self.gpu_buffer_memory_bytes,
            cpu_memory_bytes: self.cpu_memory_bytes,
        })
    }
}

fn create_quad_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    layout: &wgpu::BindGroupLayout,
) -> wgpu::BindGroup {
    let texture_size = wgpu::Extent3d {
        width: 2,
        height: 2,
        depth_or_array_layers: 1,
    };
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("fovea-synthetic-texture"),
        size: texture_size,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let pixels = [
        240, 86, 92, 255, 54, 167, 138, 255, 58, 141, 222, 255, 238, 196, 93, 255,
    ];
    queue.write_texture(
        wgpu::ImageCopyTexture {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        &pixels,
        wgpu::ImageDataLayout {
            offset: 0,
            bytes_per_row: Some(8),
            rows_per_image: Some(2),
        },
        texture_size,
    );
    let texture_view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("fovea-synthetic-texture-sampler"),
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Nearest,
        min_filter: wgpu::FilterMode::Nearest,
        ..Default::default()
    });

    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("fovea-quad-texture-bind-group"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&texture_view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(&sampler),
            },
        ],
    })
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

fn create_quad_pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    format: wgpu::TextureFormat,
) -> wgpu::RenderPipeline {
    create_pipeline(
        device,
        "fovea-quad-pipeline",
        layout,
        shader,
        "vs_quad",
        "fs_quad",
        format,
        &[],
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
            entry_point: vertex_entry,
            buffers: vertex_buffers,
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: fragment_entry,
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
        multiview: None,
    })
}

fn surface_error(err: wgpu::SurfaceError) -> JsValue {
    JsValue::from_str(&format!("surface error: {err:?}"))
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
