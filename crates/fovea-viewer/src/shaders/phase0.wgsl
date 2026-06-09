struct Camera {
  center: vec2<f32>,
  zoom: f32,
  _pad0: f32,
  viewport: vec2<f32>,
  _pad1: vec2<f32>,
  overlay: vec4<f32>,
  heatmap: vec4<f32>,
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

fn magma_colormap(value: f32) -> vec3<f32> {
  let v = clamp(value, 0.0, 1.0);
  let c0 = vec3<f32>(0.0015, 0.0005, 0.0139);
  let c1 = vec3<f32>(0.2515, 0.0714, 0.4854);
  let c2 = vec3<f32>(0.7164, 0.2140, 0.4753);
  let c3 = vec3<f32>(0.9867, 0.5356, 0.3822);
  let c4 = vec3<f32>(0.9871, 0.9914, 0.7495);

  if v < 0.25 {
    return mix(c0, c1, v / 0.25);
  }
  if v < 0.5 {
    return mix(c1, c2, (v - 0.25) / 0.25);
  }
  if v < 0.75 {
    return mix(c2, c3, (v - 0.5) / 0.25);
  }
  return mix(c3, c4, (v - 0.75) / 0.25);
}

fn viridis_colormap(value: f32) -> vec3<f32> {
  let v = clamp(value, 0.0, 1.0);
  let c0 = vec3<f32>(0.2670, 0.0049, 0.3294);
  let c1 = vec3<f32>(0.2297, 0.3224, 0.5457);
  let c2 = vec3<f32>(0.1276, 0.5669, 0.5506);
  let c3 = vec3<f32>(0.3692, 0.7889, 0.3829);
  let c4 = vec3<f32>(0.9932, 0.9062, 0.1439);

  if v < 0.25 {
    return mix(c0, c1, v / 0.25);
  }
  if v < 0.5 {
    return mix(c1, c2, (v - 0.25) / 0.25);
  }
  if v < 0.75 {
    return mix(c2, c3, (v - 0.5) / 0.25);
  }
  return mix(c3, c4, (v - 0.75) / 0.25);
}

fn heatmap_color(value: f32, colormap_id: f32) -> vec3<f32> {
  if colormap_id > 1.5 {
    return vec3<f32>(value);
  }
  if colormap_id > 0.5 {
    return viridis_colormap(value);
  }
  return magma_colormap(value);
}

@fragment
fn fs_heatmap(input: VertexOut) -> @location(0) vec4<f32> {
  let raw = textureSample(quad_texture, quad_sampler, input.uv).r;
  let range = max(camera.heatmap.z - camera.heatmap.y, 0.0001);
  let value = clamp((raw - camera.heatmap.y) / range, 0.0, 1.0);
  let alpha = clamp(camera.heatmap.x, 0.0, 1.0) * smoothstep(0.001, 0.08, value);
  return vec4<f32>(heatmap_color(value, camera.heatmap.w), alpha);
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

fn class_color(class_id: u32) -> vec4<f32> {
  if class_id == 4294967295u {
    return vec4<f32>(1.0, 1.0, 1.0, 1.0);
  }
  if class_id == 4294967294u {
    return vec4<f32>(1.0, 0.92, 0.16, 1.0);
  }

  let value = class_id % 6u;
  if value == 0u {
    return vec4<f32>(0.00, 0.78, 0.86, 1.0);
  }
  if value == 1u {
    return vec4<f32>(1.00, 0.55, 0.24, 1.0);
  }
  if value == 2u {
    return vec4<f32>(0.48, 0.82, 0.36, 1.0);
  }
  if value == 3u {
    return vec4<f32>(0.92, 0.38, 0.58, 1.0);
  }
  if value == 4u {
    return vec4<f32>(0.62, 0.55, 0.98, 1.0);
  }
  return vec4<f32>(0.98, 0.82, 0.24, 1.0);
}

@vertex
fn vs_overlay_point(
  @builtin(vertex_index) vertex_index: u32,
  @location(0) world_position: vec2<f32>,
  @location(1) class_id: u32
) -> VertexOut {
  var corners = array<vec2<f32>, 6>(
    vec2<f32>(-1.0, -1.0),
    vec2<f32>(1.0, -1.0),
    vec2<f32>(1.0, 1.0),
    vec2<f32>(-1.0, -1.0),
    vec2<f32>(1.0, 1.0),
    vec2<f32>(-1.0, 1.0)
  );
  var size = camera.overlay.y;
  if class_id >= 4294967294u {
    size = camera.overlay.y + 7.0;
  }
  let screen = (world_position - camera.center) * camera.zoom + camera.viewport * 0.5;
  let sized = screen + corners[vertex_index] * size * 0.5;
  let ndc = vec2<f32>(
    (sized.x / camera.viewport.x) * 2.0 - 1.0,
    1.0 - (sized.y / camera.viewport.y) * 2.0
  );

  var out: VertexOut;
  out.position = vec4<f32>(ndc, 0.0, 1.0);
  let base_color = class_color(class_id);
  var alpha = camera.overlay.x * 0.78;
  if class_id >= 4294967294u {
    alpha = 1.0;
  }
  out.color = vec4<f32>(base_color.rgb, alpha);
  out.uv = vec2<f32>(0.0, 0.0);
  return out;
}

@vertex
fn vs_overlay_line(
  @location(0) segment_start: vec2<f32>,
  @location(1) segment_end: vec2<f32>,
  @location(2) endpoint: f32,
  @location(3) side: f32,
  @location(4) class_id: u32
) -> VertexOut {
  let world_position = segment_start + (segment_end - segment_start) * endpoint;
  let screen = (world_position - camera.center) * camera.zoom + camera.viewport * 0.5;
  let start_screen = (segment_start - camera.center) * camera.zoom + camera.viewport * 0.5;
  let end_screen = (segment_end - camera.center) * camera.zoom + camera.viewport * 0.5;
  let delta = end_screen - start_screen;
  var normal = vec2<f32>(0.0, 1.0);
  if dot(delta, delta) > 0.0001 {
    let direction = normalize(delta);
    normal = vec2<f32>(-direction.y, direction.x);
  }
  let sized = screen + normal * side * camera.overlay.z * 0.5;
  let ndc = vec2<f32>(
    (sized.x / camera.viewport.x) * 2.0 - 1.0,
    1.0 - (sized.y / camera.viewport.y) * 2.0
  );

  var out: VertexOut;
  out.position = vec4<f32>(ndc, 0.0, 1.0);
  out.color = vec4<f32>(class_color(class_id).rgb, camera.overlay.x * 0.72);
  out.uv = vec2<f32>(0.0, 0.0);
  return out;
}
