# Fovea Implementation Plan

_Last updated: 2026-06-09_

## Direction

Fovea serves pathology data directly from native sources:

- OpenSlide-readable WSI files for slide pixels.
- Histotyper protobuf files for cell masks.
- In-memory density heatmaps derived from cell centroids.

The browser viewer remains a WebGPU renderer. It fetches manifests and visible byte ranges from the local `fovea-pack serve` process, uploads prepared resources to the GPU, and keeps the frame path free of slide decoding, protobuf parsing, and global cell scans.

## Architecture

```text
fovea-pack serve
├── opens WSI with OpenSlide
├── decodes optional protobuf cell masks once
├── builds optional heatmap tiles in memory
├── serves slide/cells/heatmap endpoints over HTTP
└── caches encoded slide tiles in RAM

@fovea/viewer
├── loads slide, cells, and heatmap endpoints
├── requests only visible tiles/chunks
├── uploads bytes to WebGPU incrementally
└── renders slide + cells + heatmap at interactive frame rates
```

## Performance Rules

- The render loop never opens WSI files, parses protobufs, scans all cells, or waits for a request.
- Cell chunks and heatmap tiles are prepared in memory at server startup.
- Slide tiles are generated on demand and cached as encoded bytes in RAM.
- Request scheduling remains viewport-driven and bounded by concurrency limits.
- Warm-cache slide tile latency should be near static byte serving; cold tile latency includes OpenSlide read and image encoding.

## Current Scope

- Direct slide serving from `.svs` and other OpenSlide-readable formats.
- Direct protobuf cell loading for `new_cell_masks.proto` and legacy `cell_masks.proto` payloads.
- In-memory cell chunk serving.
- In-memory density heatmap serving.
- WebGPU slide, cell, and heatmap rendering.
- Hover/click cell picking.
- Benchmark and performance stats APIs.

## Non-Goals

- Browser-native SVS decoding.
- Persistent generated tile stores.
- Polygon editing.
- Multi-user annotation workflows.
- Mobile-first touch UI.

## Acceptance

- A user can run `fovea-pack serve --wsi slide.svs --cells-protobuf cell_masks.bin --heatmap`.
- The printed viewer URL opens slide, cells, and heatmap without any pre-generation step.
- Warm-cache interaction remains smooth and below the frame budgets described by the performance metrics.
- `npm test` and `npm run check` pass.
