# Fovea: Implementation Plan

_Document version: 0.1_
_Last updated: 2026-06-08_
_Status: Draft implementation plan_

---

## 1. Executive Summary

### 1.1 Problem Statement

Whole-slide pathology images are not normal images. A single WSI can be **50,000–150,000 pixels wide**, often containing billions of pixels stored as a multi-resolution tiled pyramid. Rendering the image alone is already a streaming problem. Rendering the image together with AI-generated spatial outputs — **hundreds of thousands to millions of cell polygons, tissue segmentations, heatmaps, and editable annotations** — is a real-time graphics and spatial data systems problem.

Existing pathology viewers are usually optimized for one of three things:

| Viewer Type | Strength | Weakness |
|---|---|---|
| Classical WSI viewers | Smooth slide viewing | Weak AI overlay rendering and interaction |
| Annotation tools | Manual polygon workflows | Poor performance with massive model outputs |
| ML notebooks / dashboards | Flexible analysis | Not usable by pathologists at slide scale |

**Fovea** solves the missing middle:

> A high-performance WebGPU WSI rendering engine, written in Rust and compiled to WebAssembly, designed from the ground up for AI-native pathology overlays.

The target user should be able to open a WSI, pan and zoom smoothly, inspect model-generated cells and heatmaps, select and edit objects, and understand model behavior in the same visual environment used for annotation.

The core insight:

> A WSI viewer for AI pathology is not just an image viewer. It is a real-time spatial database, GPU renderer, tile streaming system, and annotation engine.

---

### 1.2 Computational Constraint

Fovea must treat performance as a first-class citizen from day one.

At 60 FPS, the frame budget is:

```text
16.7 ms per frame
```

Within that budget, the viewer must:

```text
- Update camera transforms
- Determine visible image tiles
- Determine visible overlay chunks
- Draw WSI tiles
- Draw heatmaps
- Draw cell polygons or centroids
- Draw selected/hovered objects
- Process user interaction
- Avoid blocking the browser main thread
```

The render path **cannot** do expensive work such as:

```text
- Decode large TIFF regions
- Parse massive JSON files
- Scan all cells
- Tessellate millions of polygons
- Allocate large temporary arrays
- Rebuild GPU buffers from scratch
- Cross the JS/WASM boundary per object
```

Therefore the architecture must enforce:

```text
Heavy work happens off the frame-critical path.
The render loop only draws prepared GPU resources.
Spatial data is chunked.
Visible data only is loaded.
Geometry is cached.
GPU uploads are incremental.
JS/WASM calls are coarse-grained.
```

Performance is not a later optimization. It determines the data model, file format, API, rendering architecture, and implementation roadmap.

---

### 1.3 Target Deliverable

The final deliverable is a reusable library and tooling stack:

```text
Fovea
├── Native Rust ingestion / packing tools
├── Rust → WASM WebGPU rendering engine
├── TypeScript public API
├── Optional React wrapper
└── Standardized WSI + overlay serialization format
```

The browser-facing API should feel simple:

```ts
import init, { FoveaViewer } from "@fovea/viewer";

await init();

const viewer = new FoveaViewer({
  canvas,
  slide: {
    manifestUrl: "/slides/case-001/manifest.json",
  },
  overlays: [
    {
      id: "cells",
      type: "cell-polygons",
      manifestUrl: "/slides/case-001/overlays/cells/manifest.json",
    },
    {
      id: "tumor-response",
      type: "heatmap",
      manifestUrl: "/slides/case-001/heatmaps/response/manifest.json",
    },
  ],
});

viewer.start();
```

The user-facing product capability:

```text
- Open a WSI
- Render tiled pyramid image with WebGPU
- Overlay cell centroids, polygons, tissue regions, and heatmaps
- Pan/zoom smoothly
- Toggle and style layers
- Hover/click/select cells
- Inspect object metadata
- Eventually edit annotations and export deltas
```

---

### 1.4 Key Innovations

| Dimension | Innovation | Impact |
|---|---|---|
| **Rust/WASM/WebGPU Core** | Rendering engine written in Rust, compiled to WASM, using WebGPU through `wgpu` | Performance, memory safety, native/web portability |
| **AI-Native Overlay Model** | Cells, tissue regions, heatmaps, and annotations are first-class spatial layers | Avoids treating AI outputs as static images |
| **Spatial Chunking** | All overlays are stored in viewport-queryable chunks | Makes million-object overlays tractable |
| **GPU-Oriented Data Layout** | Binary formats optimized for typed arrays, quantized geometry, and GPU buffer upload | Avoids JSON bottlenecks and excessive memory use |
| **OpenSlide Ingestion** | Native Rust ingestion via `openslide-rs` | Supports real-world pathology slide formats |
| **Progressive Rendering** | Low-zoom density/centroids, high-zoom full polygons | Keeps visual interaction smooth |
| **Annotation Deltas** | Edits stored as localized deltas over model output | Avoids rewriting massive model outputs |
| **One Tool for ML + Annotation** | Same viewer for pathologists and ML engineers | Reduces the failure mode where engineers stop questioning the data |

---

### 1.5 Design Philosophy

1. **Performance over completeness**

   A slow viewer with many features is a failed viewer. Every feature must preserve interactivity.

2. **Spatial-first architecture**

   Every object lives in slide coordinates and must be chunked by space.

3. **Prepared GPU resources only in the render loop**

   The render loop should draw, not compute.

4. **Normalize formats before viewing**

   Use native Rust/OpenSlide tooling to ingest messy vendor WSIs. The browser viewer consumes clean manifests and chunks.

5. **Avoid per-object JS/WASM calls**

   The WASM boundary must be coarse-grained. Large data moves in binary buffers, not object-by-object calls.

6. **Progressive fidelity**

   At low zoom, show density or points. At high zoom, show full polygons and edit handles.

7. **Debuggability matters**

   The viewer should expose enough introspection to help ML engineers answer: “Why did the model do this here?”

8. **Annotation provenance is part of the data model**

   Every edit should retain object identity, source model version, user, timestamp, and delta history.

---

### 1.6 MVP Scope and Non-Goals

#### MVP includes

```text
- Native Rust packer using openslide-rs
- Normalized Fovea slide manifest
- Static tiled image pyramid generation
- Rust/WASM WebGPU renderer
- Smooth pan/zoom
- Image tile streaming and GPU texture cache
- Cell centroid layer
- Cell polygon outline layer
- One heatmap layer
- Layer opacity and visibility controls
- Hover/click cell picking
- Object metadata callback to TypeScript
- Basic benchmark harness
```

#### MVP does not include

```text
- Full collaborative annotation
- Arbitrary in-browser SVS decoding
- Native browser OpenSlide
- Polygon editing
- Multi-user review workflow
- Server-side dynamic tile rendering
- GPU-based picking
- Full OME-Zarr spatial omics browser
- DICOM WSI support
- Mobile/touch-first support
```

#### Definition of done for MVP

```text
- 100k × 80k WSI renders smoothly from precomputed tiles
- 500k cells can be loaded as spatial chunks
- Visible centroids render at 60 FPS during pan/zoom on modern Chrome
- Polygon outlines render smoothly at high zoom
- One heatmap layer can be blended over the slide
- Hover/click returns cell ID and metadata
- Tile and overlay loading never blocks interaction
- Render loop remains below 16.7 ms p95 during normal pan/zoom
```

---

## 2. Core Architecture

### 2.1 System Overview

Fovea should be split into three major packages:

```text
Fovea
├── fovea-pack       Native Rust CLI / library
├── fovea-viewer     Rust → WASM WebGPU engine
└── fovea-js         TypeScript wrapper and app integration
```

High-level architecture:

```text
                     ┌────────────────────────┐
                     │    Vendor WSI file      │
                     │ SVS / NDPI / MRXS / etc │
                     └───────────┬────────────┘
                                 │
                                 ▼
                     ┌────────────────────────┐
                     │       fovea-pack        │
                     │ Native Rust + OpenSlide │
                     └───────────┬────────────┘
                                 │
                                 ▼
                     ┌────────────────────────┐
                     │     Fovea bundle        │
                     │ manifest + tiles/chunks │
                     └───────────┬────────────┘
                                 │
                                 ▼
┌─────────────────────────────────────────────────────────────────┐
│                         Browser App                              │
│                                                                 │
│  ┌──────────────────┐      ┌─────────────────────────────────┐  │
│  │   TypeScript     │─────▶│ Rust/WASM WebGPU Engine          │  │
│  │   host layer     │◀─────│ fovea-viewer                     │  │
│  └──────────────────┘      └─────────────────────────────────┘  │
│             │                           │                       │
│             │                           ▼                       │
│             │                  ┌────────────────┐               │
│             └─────────────────▶│ HTML Canvas     │               │
│                                │ WebGPU Surface  │               │
│                                └────────────────┘               │
└─────────────────────────────────────────────────────────────────┘
```

The native ingestion layer handles compatibility with vendor slide formats. The browser viewer handles rendering, interaction, and GPU resources.

---

### 2.2 Native Rust Ingestion Layer: `fovea-pack`

The ingestion layer handles the messy world:

```text
- Vendor WSI formats
- OpenSlide metadata
- Tile extraction
- Pyramid normalization
- Model output conversion
- Spatial chunking
- Heatmap pyramid generation
- Manifest generation
```

It is native Rust, not WASM.

Primary responsibilities:

```text
- Open WSI using openslide-rs
- Extract slide dimensions and metadata
- Generate normalized image pyramid
- Convert model outputs into Fovea overlay chunks
- Build spatial indices
- Quantize polygon coordinates
- Generate heatmap tiles
- Validate all coordinate systems
- Write a static Fovea bundle
```

Proposed CLI:

```bash
fovea-pack slide \
  --wsi /data/case_001.svs \
  --cells /data/case_001_cells.parquet \
  --heatmap /data/case_001_response_score.zarr \
  --out /public/slides/case_001.fovea \
  --tile-size 512 \
  --image-format webp \
  --cell-chunk-size 4096 \
  --polygon-quantization u16 \
  --generate-lod true
```

Expected output:

```text
case_001.fovea/
├── manifest.json
├── images/
│   ├── level_0/
│   │   ├── 0_0.webp
│   │   ├── 1_0.webp
│   │   └── ...
│   ├── level_1/
│   └── ...
├── overlays/
│   └── cells/
│       ├── manifest.json
│       ├── index.bin
│       └── chunks/
│           ├── 000000.bin
│           ├── 000001.bin
│           └── ...
├── heatmaps/
│   └── response_score/
│       ├── manifest.json
│       ├── level_0/
│       ├── level_1/
│       └── ...
└── thumbnails/
    ├── macro.webp
    └── label.webp
```

---

### 2.3 Browser Rendering Layer: `fovea-viewer`

The rendering engine is Rust compiled to WebAssembly.

Responsibilities:

```text
- Initialize WebGPU through wgpu
- Own renderer state
- Own camera state
- Compute visible tiles
- Compute visible overlay chunks
- Manage GPU textures
- Manage GPU buffers
- Render image tiles
- Render heatmaps
- Render cell centroids and polygons
- Handle interaction state
- Perform picking
- Emit events to TypeScript
```

The Rust engine should own all performance-critical state:

```text
- Camera transforms
- Viewport math
- Tile cache
- Overlay chunk cache
- GPU resources
- Render batches
- Spatial indices
- Selection state
- Hover state
```

TypeScript should not manage per-frame rendering.

---

### 2.4 TypeScript Host Layer: `fovea-js`

TypeScript exists for ergonomics and browser integration.

Responsibilities:

```text
- Initialize WASM
- Attach engine to canvas
- Fetch manifests
- Fetch tile/chunk bytes if we choose host-managed fetching
- Forward input events
- Expose public API
- Integrate with React
- Render UI panels/tooltips outside canvas
- Listen to event batches from Rust
```

The public API should be coarse-grained:

```ts
const viewer = await FoveaViewer.create({
  canvas,
  manifestUrl: "/slides/case_001.fovea/manifest.json",
});

await viewer.addLayer({
  id: "cells",
  type: "cells",
  manifestUrl: "/slides/case_001.fovea/overlays/cells/manifest.json",
});

viewer.setLayerOpacity("cells", 0.7);
viewer.setLayerVisibility("cells", true);
viewer.setColorBy("cells", "class_id");

viewer.on("cell-click", event => {
  console.log(event.cellId, event.slideX, event.slideY);
});
```

Avoid APIs like:

```ts
viewer.addCell(cell);
viewer.addPolygon(polygon);
viewer.updateCellEveryFrame(cellId, coords);
```

Those APIs are too chatty and will destroy performance.

---

### 2.5 Runtime Data Flow

At runtime, the viewer does this:

```text
1. Load slide manifest
2. Initialize camera to fit slide bounds
3. Compute visible image tiles
4. Request missing tiles
5. Decode/upload tiles to GPU textures
6. Compute visible overlay chunks
7. Request missing overlay chunks
8. Parse chunks in WASM
9. Prepare GPU buffers
10. Draw frame
11. Repeat on pan/zoom/input
```

Frame loop:

```text
Every frame:
  - Read camera state
  - Compute visible tile set
  - Submit async load requests for missing resources
  - Draw available image tiles
  - Draw available heatmap tiles
  - Draw available overlay batches
  - Draw hover/selection layer
  - Drain interaction events
```

Important:

> The render loop should never wait for tiles or chunks. It draws what is available and schedules what is missing.

---

### 2.6 Render Pass Architecture

A frame should be organized into explicit passes:

```text
Frame
├── Pass 1: Background clear
├── Pass 2: WSI image tile pass
├── Pass 3: Heatmap pass
├── Pass 4: Tissue region fill pass
├── Pass 5: Cell polygon fill pass
├── Pass 6: Cell polygon outline pass
├── Pass 7: Centroid / point pass
├── Pass 8: Selection and hover highlight pass
├── Pass 9: Edit handles pass
└── Optional Pass 10: ID-picking pass
```

MVP passes:

```text
- Background clear
- Image tile pass
- Heatmap pass
- Cell centroid pass
- Cell outline pass
- Hover/selection pass
```

Later passes:

```text
- Filled polygons
- Tissue segmentation
- Edit handles
- GPU picking
- Text labels
```

---

### 2.7 Camera and Coordinate System

Everything must be expressed in **level-0 slide coordinates**.

Coordinate systems:

```text
Screen coordinates:
  pixels in browser canvas

NDC coordinates:
  WebGPU normalized device coordinates

World coordinates:
  slide level-0 pixel coordinates

Tile coordinates:
  pyramid level + tile x/y

Micron coordinates:
  optional physical coordinate system using MPP metadata
```

Canonical rule:

> All overlays are stored in level-0 slide coordinates.

Camera model:

```rust
pub struct Camera {
    pub center_x: f64,
    pub center_y: f64,
    pub zoom: f64,
    pub viewport_width_px: u32,
    pub viewport_height_px: u32,
    pub device_pixel_ratio: f64,
}
```

Required transforms:

```rust
screen_to_slide(screen_x, screen_y) -> SlidePoint
slide_to_screen(slide_x, slide_y) -> ScreenPoint
slide_rect_visible() -> Rect
best_pyramid_level() -> PyramidLevel
```

Performance rule:

```text
Camera math must allocate zero memory per frame.
```

---

### 2.8 Image Tile Scheduler

The image tile scheduler determines what image tiles are needed.

Input:

```text
- Camera
- Viewport size
- Pyramid metadata
- Tile size
- Current cache state
```

Output:

```text
- Visible tile IDs
- Prefetch tile IDs
- Eviction candidates
```

Tile ID:

```rust
pub struct TileId {
    pub level: u8,
    pub x: u32,
    pub y: u32,
}
```

Scheduling priority:

```text
Priority 0: visible tiles nearest viewport center
Priority 1: visible tiles near viewport edges
Priority 2: one-tile margin around viewport
Priority 3: likely next tiles based on pan velocity
```

Cache behavior:

```text
- Never block frame waiting for tile
- Use lower-resolution fallback tile if high-resolution tile missing
- Upload tile to GPU asynchronously
- Evict least-recently-used offscreen textures
```

Texture cache constraints:

```text
MVP default GPU tile cache:
- 512 MB soft limit
- 768 MB hard limit
- LRU eviction
- Separate accounting for image tiles and heatmap tiles
```

---

### 2.9 Overlay Scheduler

Overlay layers are also spatially chunked.

For every frame, the overlay scheduler asks:

```text
- Which cell chunks intersect the visible slide rectangle?
- Which chunks are already loaded?
- Which chunks are needed at current zoom?
- Which LOD should be rendered?
- Which chunks should be prefetched?
- Which chunks can be evicted?
```

Overlay chunk ID:

```rust
pub struct ChunkId {
    pub layer_id: LayerId,
    pub lod_level: u8,
    pub x: u32,
    pub y: u32,
}
```

LOD rules:

```text
Low zoom:
  render density heatmap or aggregated centroids

Medium zoom:
  render centroids or simplified polygons

High zoom:
  render full-resolution polygon outlines/fills
```

Suggested thresholds:

```text
zoom < 0.05 px/slide-px:
  no individual cells; show density only

0.05 <= zoom < 0.25:
  centroids only

0.25 <= zoom < 1.0:
  simplified polygons

zoom >= 1.0:
  full polygons
```

Exact thresholds should be benchmarked, not guessed.

---

### 2.10 Geometry Pipeline

Cell polygons should not be rendered object-by-object.

Pipeline:

```text
Cell chunk binary
  ↓
Parse into packed arrays
  ↓
Decode quantized coordinates
  ↓
Generate GPU vertex/index buffers
  ↓
Group by style/class
  ↓
Cache as RenderBatch
  ↓
Draw with few draw calls
```

For MVP:

```text
- Centroids rendered as instanced quads or points
- Polygon outlines rendered as line strips or generated stroke geometry
```

For filled polygons:

```text
- Use Rust tessellation
- Candidate library: lyon
- Triangulate once per chunk
- Cache GPU buffers
```

Important performance rule:

> Tessellation must never happen synchronously inside the frame render path.

---

### 2.11 Heatmap Renderer

Heatmaps should be represented as tiled raster pyramids.

Each heatmap tile stores scalar values, not pre-colored RGBA if possible.

Options:

```text
u8 values:
  compact, enough for many visual overlays

u16 values:
  better dynamic range

f16/f32 values:
  expensive but useful for scientific exactness
```

MVP recommendation:

```text
- Store heatmap tiles as quantized u16
- Store per-layer min/max or robust p01/p99
- Upload as single-channel texture
- Apply colormap in WGSL shader
```

Heatmap shader inputs:

```text
- scalar texture
- opacity
- min/max
- colormap type
- clamp mode
- blend mode
```

Default behavior:

```text
- transparent where value is NaN or mask = 0
- opacity controlled per layer
- colormap computed on GPU
```

---

### 2.12 Interaction Engine

Interaction should be handled in Rust wherever it touches spatial data.

Input events forwarded from TypeScript:

```text
- pointerdown
- pointermove
- pointerup
- wheel
- keydown
- resize
```

Rust maintains:

```text
- current interaction mode
- cursor screen position
- cursor slide position
- hovered object ID
- selected object IDs
- lasso polygon
- drag state
```

Picking MVP:

```text
1. Convert cursor screen coordinate to slide coordinate
2. Query visible chunk spatial index
3. Find candidate cells near cursor
4. Test nearest centroid or polygon bbox
5. Optionally test point-in-polygon
6. Return top candidate
```

Do not start with GPU picking. CPU picking is simpler and probably good enough if spatial chunks are small.

---

### 2.13 Event Bridge

Rust emits event batches to TypeScript.

Example events:

```rust
pub enum ViewerEvent {
    ViewportChanged {
        center_x: f64,
        center_y: f64,
        zoom: f64,
    },
    CellHovered {
        cell_id: u64,
        slide_x: f64,
        slide_y: f64,
    },
    CellClicked {
        cell_id: u64,
        slide_x: f64,
        slide_y: f64,
    },
    SelectionChanged {
        count: u32,
    },
    TileLoadError {
        tile_id: TileId,
        error_code: u32,
    },
}
```

TypeScript API:

```ts
viewer.on("cell-hover", callback);
viewer.on("cell-click", callback);
viewer.on("selection-change", callback);
viewer.on("viewport-change", callback);
```

Implementation detail:

```text
Rust stores events in a small ring buffer.
TypeScript drains events once per animation frame.
```

This avoids excessive callback overhead.

---

## 3. Data Models

### 3.1 Fovea Bundle Layout

Canonical v1 bundle:

```text
case_001.fovea/
├── manifest.json
├── images/
│   ├── level_0/
│   │   ├── 0_0.webp
│   │   ├── 1_0.webp
│   │   └── ...
│   ├── level_1/
│   └── level_N/
├── overlays/
│   └── cells/
│       ├── manifest.json
│       ├── index.bin
│       └── chunks/
│           ├── 000000.bin
│           ├── 000001.bin
│           └── ...
├── heatmaps/
│   └── response_score/
│       ├── manifest.json
│       └── level_0/
│           ├── 0_0.bin
│           └── ...
└── annotations/
    └── deltas.jsonl
```

The browser should be able to load the bundle over plain HTTP/S3/GCS/CDN without a backend.

---

### 3.2 Slide Manifest

`manifest.json`:

```json
{
  "version": "0.1.0",
  "bundle_type": "fovea_slide",
  "slide_id": "case_001",
  "source": {
    "filename": "case_001.svs",
    "vendor": "aperio",
    "openslide_detected_format": "aperio",
    "openslide_version": "x.y.z"
  },
  "dimensions": {
    "width": 100000,
    "height": 80000
  },
  "physical": {
    "mpp_x": 0.25,
    "mpp_y": 0.25,
    "objective_power": 40
  },
  "pyramid": {
    "tile_size": 512,
    "format": "webp",
    "levels": [
      {
        "level": 0,
        "width": 100000,
        "height": 80000,
        "downsample": 1.0,
        "cols": 196,
        "rows": 157,
        "url_template": "images/level_0/{x}_{y}.webp"
      },
      {
        "level": 1,
        "width": 50000,
        "height": 40000,
        "downsample": 2.0,
        "cols": 98,
        "rows": 79,
        "url_template": "images/level_1/{x}_{y}.webp"
      }
    ]
  },
  "layers": [
    {
      "id": "cells",
      "type": "cell_polygons",
      "manifest": "overlays/cells/manifest.json"
    },
    {
      "id": "response_score",
      "type": "heatmap",
      "manifest": "heatmaps/response_score/manifest.json"
    }
  ]
}
```

Required invariants:

```text
- All coordinates are level-0 slide pixel coordinates.
- Pyramid levels have explicit downsample values.
- Tile URLs are deterministic.
- MPP is stored when available.
- Missing MPP is allowed but explicit.
```

---

### 3.3 Cell Layer Manifest

`overlays/cells/manifest.json`:

```json
{
  "version": "0.1.0",
  "layer_id": "cells",
  "layer_type": "cell_polygons",
  "coordinate_space": "slide_level_0",
  "object_count": 742381,
  "chunking": {
    "strategy": "fixed_grid",
    "chunk_width": 4096,
    "chunk_height": 4096,
    "cols": 25,
    "rows": 20
  },
  "lod": [
    {
      "lod_level": 0,
      "description": "centroids",
      "min_zoom": 0.05,
      "max_zoom": 0.25
    },
    {
      "lod_level": 1,
      "description": "simplified_polygons",
      "min_zoom": 0.25,
      "max_zoom": 1.0
    },
    {
      "lod_level": 2,
      "description": "full_polygons",
      "min_zoom": 1.0,
      "max_zoom": 20.0
    }
  ],
  "schema": {
    "id_type": "u64",
    "coordinate_encoding": "chunk_local_u16",
    "classes": [
      { "id": 0, "name": "tumor" },
      { "id": 1, "name": "lymphocyte" },
      { "id": 2, "name": "fibroblast" },
      { "id": 3, "name": "macrophage" }
    ]
  },
  "chunks": {
    "index": "index.bin",
    "url_template": "chunks/{chunk_id}.bin"
  }
}
```

---

### 3.4 Cell Chunk Binary Format

Cell chunks should be binary, not JSON.

Goals:

```text
- Fast parse
- Minimal allocations
- Compact transfer
- Direct mapping to typed arrays
- GPU-friendly packing
```

Proposed `CellChunkV1` layout:

```text
Header
├── magic: "FVCELL01"
├── version: u16
├── chunk_id: u32
├── lod_level: u8
├── object_count: u32
├── vertex_count: u32
├── index_count: u32
├── chunk_origin_x: f64
├── chunk_origin_y: f64
├── chunk_width: f32
├── chunk_height: f32

Object table
├── ids: u64[object_count]
├── centroid_x: u16[object_count]
├── centroid_y: u16[object_count]
├── bbox_min_x: u16[object_count]
├── bbox_min_y: u16[object_count]
├── bbox_max_x: u16[object_count]
├── bbox_max_y: u16[object_count]
├── class_id: u16[object_count]
├── confidence: f16 or u16[object_count]
├── polygon_offset: u32[object_count]
└── polygon_length: u16[object_count]

Geometry table
└── polygon_coords: packed u16 x/y pairs

Optional GPU-ready section
├── vertex_buffer: packed f32 or normalized u16
└── index_buffer: u32[index_count]
```

Coordinate decoding:

```rust
slide_x = chunk_origin_x + (encoded_x as f64 / 65535.0) * chunk_width
slide_y = chunk_origin_y + (encoded_y as f64 / 65535.0) * chunk_height
```

Why chunk-local `u16`?

```text
- 4 bytes per point instead of 8 or 16
- Enough precision within 4096×4096 chunks
- Small transfer size
- Easy GPU upload
```

Precision estimate:

```text
4096 px / 65535 ≈ 0.0625 px precision
```

That is more than enough for pathology overlay visualization.

---

### 3.5 Runtime Cell Model

Rust runtime structs:

```rust
pub struct CellLayer {
    pub id: LayerId,
    pub manifest: CellLayerManifest,
    pub chunks: ChunkCache<CellChunk>,
    pub style: CellLayerStyle,
    pub visibility: bool,
    pub opacity: f32,
}

pub struct CellChunk {
    pub chunk_id: ChunkId,
    pub bounds: Rect,
    pub object_count: u32,
    pub objects: CellObjectTable,
    pub geometry: PolygonGeometry,
    pub gpu_batches: Option<Vec<RenderBatch>>,
    pub last_used_frame: u64,
}

pub struct CellObjectTable {
    pub ids: Vec<u64>,
    pub centroids: Vec<EncodedPoint>,
    pub bboxes: Vec<EncodedBBox>,
    pub class_ids: Vec<u16>,
    pub confidences: Vec<u16>,
    pub polygon_offsets: Vec<u32>,
    pub polygon_lengths: Vec<u16>,
}
```

No per-cell heap objects in hot paths.

Avoid:

```rust
Vec<CellObject>
```

Prefer structure-of-arrays:

```rust
CellObjectTable {
    ids: Vec<u64>,
    centroids_x: Vec<u16>,
    centroids_y: Vec<u16>,
    class_ids: Vec<u16>,
    ...
}
```

This improves cache locality and GPU packing.

---

### 3.6 Heatmap Data Model

Heatmap manifest:

```json
{
  "version": "0.1.0",
  "layer_id": "response_score",
  "layer_type": "heatmap",
  "coordinate_space": "slide_level_0",
  "value_type": "u16",
  "encoding": {
    "kind": "linear_quantized",
    "min": -3.2,
    "max": 7.8,
    "nan_value": 0
  },
  "pyramid": {
    "tile_size": 512,
    "levels": [
      {
        "level": 0,
        "width": 25000,
        "height": 20000,
        "downsample": 4.0,
        "url_template": "level_0/{x}_{y}.bin"
      }
    ]
  },
  "style": {
    "default_colormap": "viridis",
    "default_opacity": 0.5,
    "blend_mode": "alpha"
  }
}
```

Runtime heatmap layer:

```rust
pub struct HeatmapLayer {
    pub id: LayerId,
    pub manifest: HeatmapManifest,
    pub texture_cache: TextureCache<HeatmapTileId>,
    pub opacity: f32,
    pub colormap: Colormap,
    pub min_value: f32,
    pub max_value: f32,
}
```

Shader behavior:

```wgsl
value = decode_u16(texture_sample);
normalized = clamp((value - min) / (max - min), 0.0, 1.0);
color = colormap(normalized);
output = vec4(color.rgb, color.a * opacity);
```

---

### 3.7 Annotation Delta Model

Editing should not rewrite base model outputs.

Base model output:

```text
cells/chunks/*.bin
```

User edits:

```text
annotations/deltas.jsonl
```

Delta format:

```json
{
  "delta_id": "delta_000001",
  "timestamp": "2026-06-08T12:34:56Z",
  "user_id": "pathologist_001",
  "operation": "relabel_object",
  "target": {
    "layer_id": "cells",
    "object_id": 1829381
  },
  "before": {
    "class_id": 1
  },
  "after": {
    "class_id": 3
  },
  "provenance": {
    "source_model": "histoplus-v1",
    "source_checkpoint": "sha256:...",
    "viewer_version": "0.1.0"
  }
}
```

Operations:

```text
- add_polygon
- delete_object
- update_polygon
- relabel_object
- merge_objects
- split_object
- add_region
- delete_region
```

Runtime composition:

```text
base model chunks + annotation deltas = current visible state
```

Performance rule:

```text
Deltas should be applied only to visible or recently touched chunks.
```

---

### 3.8 Layer Styling Model

Each layer has style state:

```rust
pub struct LayerStyle {
    pub visible: bool,
    pub opacity: f32,
    pub color_mode: ColorMode,
    pub outline_width_px: f32,
    pub fill_enabled: bool,
    pub outline_enabled: bool,
}
```

Color modes:

```text
- fixed color
- color by class
- color by confidence
- color by scalar feature
- color by selection state
```

For per-object scalar features, avoid uploading new colors every frame. Instead:

```text
- Store scalar feature in object table
- Upload scalar buffer once
- Apply colormap in shader
```

---

## 4. Implementation Roadmap

### 4.0 Phase 0: Technical Spike and Performance Harness

#### Goal

Prove that the Rust/WASM/WebGPU stack can sustain the required rendering model before building WSI-specific complexity.

#### Tasks

##### 4.0.1 Rust/WASM/WebGPU skeleton

```text
- Create Rust workspace
- Add fovea-viewer crate
- Compile to WASM with wasm-bindgen
- Initialize wgpu on browser canvas
- Render a colored triangle
- Render a textured quad
- Add WGSL shader loading
- Add TypeScript wrapper package
```

Deliverable:

```text
Browser page rendering via Rust/WASM/wgpu/WebGPU.
```

##### 4.0.2 Synthetic large-world benchmark

```text
- Generate synthetic 100k × 100k coordinate space
- Render 10k, 100k, 500k, 1M synthetic points
- Add pan/zoom camera
- Measure FPS, frame time, GPU upload time
- Add benchmark overlay in UI
```

Metrics:

```text
- frame_time_p50
- frame_time_p95
- frame_time_p99
- draw_call_count
- visible_object_count
- GPU_buffer_memory
- CPU_memory
```

Definition of done:

```text
- 100k visible points render at 60 FPS
- Camera pan/zoom does not allocate in hot path
- JS/WASM event bridge works
```

---

### 4.1 Phase 1: Native Slide Ingestion with OpenSlide

#### Goal

Build the native Rust path that opens real WSI files and produces normalized Fovea slide bundles.

#### Tasks

##### 4.1.1 OpenSlide reader abstraction

```rust
pub trait SlideReader {
    fn dimensions(&self) -> Size;
    fn level_count(&self) -> usize;
    fn level_dimensions(&self, level: usize) -> Size;
    fn level_downsample(&self, level: usize) -> f64;
    fn properties(&self) -> SlideProperties;
    fn read_region_rgba(
        &self,
        level: usize,
        x: i64,
        y: i64,
        width: u32,
        height: u32,
    ) -> Result<RgbaImage>;
}
```

Implement:

```rust
pub struct OpenSlideReader {
    inner: openslide_rs::OpenSlide,
}
```

##### 4.1.2 Metadata extraction

Extract and store:

```text
- width
- height
- level dimensions
- level downsamples
- mpp_x
- mpp_y
- objective power
- vendor
- background color
- bounds_x / bounds_y if present
- bounds_width / bounds_height if present
```

##### 4.1.3 Tile pyramid generation

Implement:

```bash
fovea-pack slide --wsi input.svs --out output.fovea
```

Features:

```text
- tile size 512 default
- output WebP or JPEG
- skip empty/background tiles optionally
- write manifest.json
- generate thumbnail
- verify coordinate consistency
```

##### 4.1.4 Performance constraints

Packing performance targets:

```text
- Tile extraction parallelized
- Bounded memory usage
- Progress reporting
- Resume-safe output directory
- Deterministic tile paths
```

Definition of done:

```text
- Can convert at least 3 real SVS slides
- Manifest loads in browser
- Generated tiles align correctly
- No coordinate drift across pyramid levels
```

---

### 4.2 Phase 2: WebGPU Image Tile Renderer

#### Goal

Render precomputed Fovea image pyramids smoothly in the browser.

#### Tasks

##### 4.2.1 Manifest loading

```text
- TypeScript fetches manifest
- Passes JSON string or bytes to Rust
- Rust parses manifest
- Camera initialized to fit slide
```

##### 4.2.2 Tile visibility computation

```text
- Compute visible slide rect
- Pick best pyramid level
- Compute tile x/y range
- Sort visible tiles by distance to viewport center
```

##### 4.2.3 Tile request pipeline

Decision to make:

```text
Option A: TypeScript fetches image bytes and passes ArrayBuffer to Rust
Option B: Rust/WASM initiates fetch through web-sys
```

Recommended MVP:

```text
TypeScript fetches.
Rust receives decoded RGBA or compressed bytes.
```

Fastest practical path:

```text
- Browser fetches image tile
- Browser decodes using createImageBitmap
- Copy into GPU texture through WebGPU path where feasible
```

If `wgpu` interop makes this awkward, fallback:

```text
- Fetch compressed image bytes
- Decode in WASM using image crate for PNG/JPEG/WebP
- Upload RGBA to GPU texture
```

Benchmark both early.

##### 4.2.4 Texture cache

Implement:

```rust
pub struct TextureCache {
    soft_limit_bytes: usize,
    hard_limit_bytes: usize,
    entries: HashMap<TileId, TextureEntry>,
    lru: LruList<TileId>,
}
```

Eviction policy:

```text
- Never evict visible tiles
- Evict farthest offscreen first
- Prefer evicting high-resolution tiles before low-res fallback tiles
```

##### 4.2.5 Fallback rendering

If high-resolution tile is missing:

```text
- Draw lower-resolution parent tile stretched into place
- Replace with sharper tile once loaded
```

This is important for perceived performance.

Definition of done:

```text
- Smooth pan/zoom on 100k × 80k slide
- No white flashes during normal navigation
- Frame p95 < 16.7 ms while panning after initial load
- Tile loading is asynchronous and cancellable
```

---

### 4.3 Phase 3: Cell Overlay Packing

#### Goal

Convert model outputs into compact spatial chunks optimized for WASM parsing and GPU rendering.

#### Input formats to support first

Prioritize:

```text
1. Parquet
2. GeoJSON
3. CSV with WKT polygon
4. COCO-style JSON later
```

Recommended first input schema:

```text
cell_id: u64
class_id: u16
confidence: f32
centroid_x: f64
centroid_y: f64
polygon: list[(x, y)] or WKB/WKT
```

#### Tasks

##### 4.3.1 Cell parser

```text
- Read Parquet cell table
- Validate coordinate bounds
- Validate polygon orientation
- Compute bbox
- Compute centroid if missing
- Drop or flag invalid polygons
```

##### 4.3.2 Spatial chunking

Default:

```text
chunk_width = 4096 slide px
chunk_height = 4096 slide px
```

Chunk assignment:

```text
- Assign object by centroid for storage
- Keep bbox for query/picking
- If polygon crosses chunk boundary, still stored once by centroid
```

Alternative later:

```text
- Duplicate object into all intersecting chunks for faster visibility
```

##### 4.3.3 Quantization

For each chunk:

```text
encoded_x = round((x - chunk_origin_x) / chunk_width * 65535)
encoded_y = round((y - chunk_origin_y) / chunk_height * 65535)
```

Store:

```text
- u16 x/y coordinates
- object table
- polygon offsets
- polygon lengths
```

##### 4.3.4 LOD generation

Generate three LODs:

```text
LOD 0:
  centroids only

LOD 1:
  simplified polygon using Ramer-Douglas-Peucker or Visvalingam

LOD 2:
  full polygon
```

Target simplification:

```text
LOD 1 max vertices per cell: 8–12
LOD 2 original polygon, capped or flagged if pathological
```

##### 4.3.5 Binary writer

Write:

```text
- index.bin
- chunk files
- manifest.json
```

Definition of done:

```text
- 500k cells packed into chunks
- Browser loads only visible chunks
- Chunk parse time measured and below target
```

Performance targets:

```text
- Average chunk size < 1 MB compressed or < 4 MB uncompressed
- Chunk parse time < 5 ms p95
- GPU batch build < 10 ms p95 off render path
```

---

### 4.4 Phase 4: Cell Overlay Renderer

#### Goal

Render cell centroids and polygons at interactive frame rates.

#### Tasks

##### 4.4.1 Centroid renderer

Implementation:

```text
- Instanced quads or point sprites
- One instance per cell
- Position from packed centroid buffer
- Class/confidence buffer for styling
```

Draw strategy:

```text
- One draw call per visible chunk per style group initially
- Later batch chunks by layer/style
```

Definition of done:

```text
- 500k total cells
- 50k visible centroids
- 60 FPS pan/zoom
```

##### 4.4.2 Polygon outline renderer

Implementation options:

```text
Option A: WebGPU line primitives
Option B: CPU-generated stroke triangles
```

Recommendation:

```text
Use generated stroke triangles for predictable width and quality.
```

Pipeline:

```text
- Decode polygon coordinates
- Generate stroke geometry per chunk
- Upload vertex/index buffers
- Cache RenderBatch
```

Definition of done:

```text
- Full polygon outlines visible at high zoom
- Switching from centroid to polygon LOD is smooth
- No frame hitch when chunks enter viewport
```

##### 4.4.3 Filled polygon renderer

Use tessellation:

```text
- lyon tessellator in Rust
- triangulate per chunk
- cache index buffer
```

Defer to after outlines unless fills are essential.

##### 4.4.4 Styling

MVP styles:

```text
- fixed color
- color by class
- opacity
- outline width
- selected highlight
- hovered highlight
```

Later:

```text
- color by scalar
- color by gene expression
- color by uncertainty
- dynamic filters
```

---

### 4.5 Phase 5: Heatmap Renderer

#### Goal

Render tiled heatmaps with GPU colormapping and opacity blending.

#### Tasks

##### 4.5.1 Heatmap packer

Inputs:

```text
- dense numpy/zarr array
- sparse tile-level table
- per-cell scalar values converted to density grid
```

Outputs:

```text
- heatmap manifest
- multiresolution tiles
- quantization metadata
```

##### 4.5.2 Heatmap tile renderer

```text
- Load visible heatmap tiles
- Upload single-channel textures
- Apply colormap in shader
- Blend over WSI
```

##### 4.5.3 UI controls

Expose:

```ts
viewer.setLayerOpacity("response_score", 0.4);
viewer.setHeatmapRange("response_score", { min: 0.1, max: 0.9 });
viewer.setHeatmapColormap("response_score", "magma");
```

Definition of done:

```text
- One heatmap layer over WSI
- Smooth pan/zoom
- Range and opacity update without reloading tiles
```

---

### 4.6 Phase 6: Interaction and Picking

#### Goal

Make the viewer useful for ML debugging and inspection.

#### Tasks

##### 4.6.1 Hover picking

```text
- Convert cursor to slide coordinate
- Query visible chunks
- Check candidate bboxes
- Return nearest centroid or polygon hit
```

Performance target:

```text
hover picking < 2 ms p95
```

##### 4.6.2 Click selection

```text
- Single click selects object
- Shift-click multi-select
- Escape clears selection
- Selected objects rendered in highlight pass
```

##### 4.6.3 Metadata callback

Event:

```ts
viewer.on("cell-click", ({ cellId, layerId, slideX, slideY }) => {
  // app fetches or displays metadata
});
```

##### 4.6.4 Box and lasso selection

Box selection:

```text
- Query chunks intersecting box
- Select objects whose centroid is inside box
```

Lasso selection:

```text
- Convert lasso screen points to slide coordinates
- Query chunks intersecting lasso bbox
- Point-in-polygon test on centroids
```

Definition of done:

```text
- Hover/click feels instant
- Selection supports thousands of cells
- Selection state does not require re-uploading all geometry
```

---

### 4.7 Phase 7: Editing and Annotation Deltas

#### Goal

Support annotation workflows without compromising base rendering performance.

#### Tasks

##### 4.7.1 Dynamic edit layer

Separate layer:

```text
- Temporary geometry
- Selected object handles
- New polygons
- Modified polygons
```

Do not mutate base chunks directly.

##### 4.7.2 Editing operations

MVP editing:

```text
- create polygon
- delete object
- relabel object
- move vertex
- undo/redo
```

Later:

```text
- merge cells
- split cells
- brush masks
- review states
```

##### 4.7.3 Delta export

```ts
const deltas = viewer.exportAnnotationDeltas();
```

Persist:

```text
- JSONL for simplicity
- Later binary delta format if needed
```

Definition of done:

```text
- User can correct model output
- Deltas round-trip correctly
- Base model chunks remain immutable
```

---

### 4.8 Phase 8: Production Hardening

#### Goal

Make Fovea reliable as a reusable library.

#### Tasks

##### 4.8.1 Packaging

```text
- npm package: @fovea/viewer
- WASM bundle
- TypeScript types
- Vite example
- React example
```

##### 4.8.2 Testing

Required tests:

```text
- Camera transform tests
- Tile culling tests
- Coordinate round-trip tests
- Chunk parser golden tests
- Quantization precision tests
- Manifest validation tests
- Picking correctness tests
- Render smoke tests
```

##### 4.8.3 Performance regression suite

Playwright benchmark page:

```text
- Load benchmark slide
- Pan for 10 seconds
- Zoom in/out
- Toggle cell layer
- Toggle heatmap
- Record frame time p50/p95/p99
```

CI thresholds:

```text
- frame p95 must not regress by >10%
- memory must not exceed baseline by >15%
- chunk parse p95 must not regress by >10%
```

##### 4.8.4 Observability

Expose internal metrics:

```ts
viewer.getPerformanceStats();
```

Returns:

```ts
{
  fps: 59.4,
  frameTimeP95Ms: 14.2,
  visibleTiles: 42,
  loadedTiles: 381,
  visibleCellChunks: 12,
  loadedCellChunks: 84,
  visibleCells: 31522,
  gpuMemoryMb: 412,
  cpuMemoryMbEstimate: 280,
  drawCalls: 38
}
```

This is not optional. A high-performance viewer without metrics becomes impossible to debug.

---

## 5. Performance Requirements

### 5.1 Runtime Performance Targets

| Operation | Target | Notes |
|---|---:|---|
| Pan/zoom frame time | <16.7 ms p95 | 60 FPS target |
| Hover picking | <2 ms p95 | CPU spatial query |
| Tile culling | <0.5 ms p95 | Pure math, no allocation |
| Overlay chunk query | <1 ms p95 | Fixed-grid index |
| Visible centroid rendering | 50k visible cells @ 60 FPS | MVP target |
| Full polygon rendering | 5k–20k visible polygons @ 60 FPS | High zoom |
| Heatmap blending | <2 ms GPU pass | Single layer |
| Initial usable viewport | <2 s | Thumbnail/low-res first |
| Tile upload burst | no frame stalls >50 ms | Async/incremental |
| JS/WASM event overhead | <1 ms/frame | Batched events |

---

### 5.2 Memory Targets

| Resource | Soft Limit | Hard Limit |
|---|---:|---:|
| Image tile GPU cache | 512 MB | 768 MB |
| Heatmap GPU cache | 128 MB | 256 MB |
| Overlay CPU chunks | 512 MB | 1 GB |
| Overlay GPU buffers | 512 MB | 1 GB |
| Total practical browser memory | <2 GB | Avoid crashes |

---

### 5.3 Performance Anti-Patterns to Forbid

Explicitly disallow:

```text
- Loading all cells at startup
- Parsing giant JSON overlays
- One draw call per cell
- Rebuilding all GPU buffers after style changes
- Synchronous tile decode in render loop
- Per-cell JS/WASM calls
- Per-frame heap allocations in camera/tile culling
- CPU scanning all objects for hover
- Rendering full polygons at low zoom
```

---

## 6. Technical Stack Summary

| Layer | Technology | Notes |
|---|---|---|
| Native ingestion | Rust | CLI + library |
| WSI reading | `openslide-rs` | Native only |
| WASM bindings | `wasm-bindgen` | Browser bridge |
| GPU abstraction | `wgpu` | Rust → WebGPU |
| Shaders | WGSL | WebGPU shader language |
| Geometry tessellation | `lyon` | For polygon fills/strokes |
| Serialization | Custom binary + JSON manifests | Performance-critical |
| JS API | TypeScript | Public API and app integration |
| Example frontend | Vite + React | Later optional |
| Static hosting | S3/GCS/R2/CDN | Preferred v1 deployment |
| Benchmarks | Playwright + browser perf APIs | Regression tests |

---

## 7. Success Metrics

| Metric | Target |
|---|---:|
| WSI pan/zoom FPS | 60 FPS p95 on modern Chrome |
| Cell centroid scale | 500k total cells, 50k visible |
| Polygon overlay scale | 500k total polygons, visible chunks only |
| Heatmap layers | 1 MVP, 3+ later |
| Initial viewer load | <2 s to first usable view |
| Hover latency | <2 ms p95 |
| Click selection latency | <10 ms p95 |
| Tile cache hit rate after warmup | >90% during local pan |
| JS/WASM calls per frame | O(1), not O(objects) |
| Frame p99 hitch | <50 ms during normal navigation |
| Bundle format validation | 100% deterministic manifest checks |
| Coordinate round-trip error | <0.25 px level-0 equivalent |

---

## 8. Summary

Fovea should be built as a performance-first WSI rendering engine, not as a conventional web viewer.

The winning architecture is:

```text
Native Rust/OpenSlide ingestion
  → normalized static Fovea bundle
  → Rust/WASM WebGPU renderer
  → TypeScript public API
```

The central implementation rule:

> Everything must be spatially chunked, incrementally loaded, GPU-batched, and benchmarked.

If that discipline holds, Fovea can become the shared visual layer for AI pathology: the place where pathologists annotate, ML engineers debug, and teams inspect model behavior directly on the tissue.
