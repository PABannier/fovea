struct Camera {
  center: vec2<f32>,
  zoom: f32,
  _pad0: f32,
  viewport: vec2<f32>,
  _pad1: vec2<f32>,
};

@group(0) @binding(0)
var<uniform> camera: Camera;

@group(1) @binding(0)
var quad_texture: texture_2d<f32>;

@group(1) @binding(1)
var quad_sampler: sampler;

struct VertexOut {
  @builtin(position) position: vec4<f32>,
  @location(0) color: vec4<f32>,
  @location(1) uv: vec2<f32>,
};

@vertex
fn vs_triangle(@builtin(vertex_index) vertex_index: u32) -> VertexOut {
  var positions = array<vec2<f32>, 3>(
    vec2<f32>(-0.92, -0.78),
    vec2<f32>(-0.64, -0.78),
    vec2<f32>(-0.78, -0.42)
  );

  var colors = array<vec4<f32>, 3>(
    vec4<f32>(0.94, 0.33, 0.36, 1.0),
    vec4<f32>(0.21, 0.65, 0.54, 1.0),
    vec4<f32>(0.23, 0.55, 0.87, 1.0)
  );

  var out: VertexOut;
  out.position = vec4<f32>(positions[vertex_index], 0.0, 1.0);
  out.color = colors[vertex_index];
  out.uv = vec2<f32>(0.0, 0.0);
  return out;
}

@fragment
fn fs_triangle(input: VertexOut) -> @location(0) vec4<f32> {
  return input.color;
}

@vertex
fn vs_tile(@location(0) world_position: vec2<f32>, @location(1) uv: vec2<f32>) -> VertexOut {
  let screen = (world_position - camera.center) * camera.zoom + camera.viewport * 0.5;
  let ndc = vec2<f32>(
    (screen.x / camera.viewport.x) * 2.0 - 1.0,
    1.0 - (screen.y / camera.viewport.y) * 2.0
  );
  var out: VertexOut;
  out.position = vec4<f32>(ndc, 0.0, 1.0);
  out.color = vec4<f32>(1.0);
  out.uv = uv;
  return out;
}

@fragment
fn fs_tile(input: VertexOut) -> @location(0) vec4<f32> {
  return textureSample(quad_texture, quad_sampler, input.uv);
}

@vertex
fn vs_point(@location(0) world_position: vec2<f32>) -> VertexOut {
  let screen = (world_position - camera.center) * camera.zoom + camera.viewport * 0.5;
  let ndc = vec2<f32>(
    (screen.x / camera.viewport.x) * 2.0 - 1.0,
    1.0 - (screen.y / camera.viewport.y) * 2.0
  );

  var out: VertexOut;
  out.position = vec4<f32>(ndc, 0.0, 1.0);
  out.color = vec4<f32>(0.93, 0.86, 0.42, 0.72);
  out.uv = vec2<f32>(0.0, 0.0);
  return out;
}

@fragment
fn fs_point(input: VertexOut) -> @location(0) vec4<f32> {
  return input.color;
}
