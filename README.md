# Fovea

Fovea is a WebGPU whole-slide image viewer for pathology slides and AI-native overlays. It renders precomputed WSI tile pyramids, protobuf-derived cell masks, and density heatmaps from static bundles, with smooth pan/zoom, hover/click picking, and runtime performance metrics.

The browser package is `@fovea/viewer`. Native packing tools live in `fovea-pack`.

## Get Started

### 1. Build the workspace

```sh
npm install
npm run build
```

### 2. Generate viewer bundles

Pack a whole-slide image:

```sh
cargo run -p fovea-pack -- slide \
  --wsi /path/to/slide.svs \
  --out ./target/case.slide.fovea \
  --tile-size 512 \
  --image-format webp \
  --skip-background-tiles \
  --force
```

Pack protobuf cell masks:

```sh
cargo run -p fovea-pack -- cells-protobuf \
  --proto /path/to/cell_masks.pb \
  --out ./target/case.cells.overlay \
  --chunk-size 4096 \
  --force
```

Generate a density heatmap from the cell overlay:

```sh
cargo run -p fovea-pack -- heatmap-overlay \
  --overlay ./target/case.cells.overlay \
  --out ./target/case.density.heatmap \
  --bin-size 128 \
  --tile-size 256 \
  --force
```

### 3. Open the example viewer

```sh
npm run dev
```

Then open:

```text
http://127.0.0.1:5173/?bundle=/@fs/absolute/path/case.slide.fovea&overlay=/@fs/absolute/path/case.cells.overlay&heatmap=/@fs/absolute/path/case.density.heatmap
```

The example viewer hides its control and performance panels by default. Add `controls=1` and `performance=1` to show them:

```text
http://127.0.0.1:5173/?bundle=...&overlay=...&heatmap=...&controls=1&performance=1
```

## Documentation

### Minimal Browser Usage

```ts
import { FoveaViewer } from "@fovea/viewer";

const canvas = document.querySelector<HTMLCanvasElement>("#viewer");

if (!canvas) {
  throw new Error("Missing canvas");
}

const viewer = await FoveaViewer.create({
  canvas,
  bundleUrl: "/slides/case.slide.fovea",
  overlayUrl: "/slides/case.cells.overlay",
  heatmapUrl: "/slides/case.density.heatmap"
});

viewer.start();
```

The canvas should have stable CSS dimensions:

```css
#viewer {
  width: 100%;
  height: 100%;
  display: block;
  touch-action: none;
}
```

Bundle URLs may point either to a bundle directory or directly to its `manifest.json`.

`@fovea/viewer` does not render built-in UI panels. The top-left loader controls and bottom-right performance panel in `examples/web` are example-app chrome only; production apps decide whether to render any controls around the canvas.

### Viewer Options

```ts
interface FoveaViewerOptions {
  canvas: HTMLCanvasElement;
  pointCount?: 10_000 | 100_000 | 500_000 | 1_000_000;
  bundleUrl?: string;
  overlayUrl?: string;
  heatmapUrl?: string;
  tileRequestBatchSize?: number;
  overlayRequestBatchSize?: number;
  heatmapRequestBatchSize?: number;
  maxConcurrentTileRequests?: number;
  maxConcurrentOverlayRequests?: number;
  maxConcurrentHeatmapRequests?: number;
  onStats?: (stats: FrameStats, rolling: RollingFrameStats) => void;
}
```

- `canvas`: Required render target.
- `pointCount`: Synthetic benchmark point count used when no slide/overlay/heatmap is loaded.
- `bundleUrl`: `.fovea` slide bundle directory or slide `manifest.json`.
- `overlayUrl`: `.overlay` cell overlay bundle directory or overlay `manifest.json`.
- `heatmapUrl`: `.heatmap` bundle directory or heatmap `manifest.json`.
- `tileRequestBatchSize`: Max slide tile requests considered per frame. Default: `96`.
- `overlayRequestBatchSize`: Max cell chunk requests considered per frame. Default: `64`.
- `heatmapRequestBatchSize`: Max heatmap tile requests considered per frame. Default: `64`.
- `maxConcurrentTileRequests`: Parallel slide tile fetches. Default: `8`.
- `maxConcurrentOverlayRequests`: Parallel cell chunk fetches. Default: `6`.
- `maxConcurrentHeatmapRequests`: Parallel heatmap tile fetches. Default: `6`.
- `onStats`: Per-frame metrics callback.

### Viewer Methods

```ts
viewer.start();
viewer.stop();
viewer.destroy();

await viewer.loadBundle("/path/to/case.slide.fovea");
await viewer.loadOverlay("/path/to/case.cells.overlay");
await viewer.loadHeatmap("/path/to/case.density.heatmap");

viewer.resetCamera();
viewer.panByScreenDelta(20, 0);
viewer.zoomAtCanvasPoint(400, 300, -120);

viewer.setLayerVisibility("cells", true);
viewer.setLayerVisibility("heatmap", true);
viewer.setLayerOpacity("cells", 0.75);
viewer.setLayerOpacity("heatmap", 0.4);

viewer.setOverlayPointSize(3);
viewer.setOverlayOutlineWidth(1.25);

viewer.setHeatmapRange("heatmap", { min: 0.05, max: 1 });
viewer.setHeatmapColormap("heatmap", "magma"); // "magma" | "viridis" | "gray"

const stats = viewer.getPerformanceStats();
```

### Events

```ts
const off = viewer.on("cell-click", (event) => {
  console.log(event.cellId, event.classId, event.slideX, event.slideY);
});

viewer.on("cell-hover", (event) => {
  console.log(event.cellId);
});

viewer.on("selection-change", (event) => {
  console.log(event.count);
});

viewer.on("viewport-change", (event) => {
  console.log(event.centerX, event.centerY, event.zoom);
});

off();
```

Supported event names:

- `cell-hover`
- `cell-click`
- `selection-change`
- `viewport-change`

### Performance Metrics

```ts
const stats = viewer.getPerformanceStats();
```

Returns:

```ts
interface PerformanceStats {
  fps: number;
  frameTimeP50Ms: number;
  frameTimeP95Ms: number;
  frameTimeP99Ms: number;
  frameTimeMs: number;
  uploadTimeMs: number;
  drawCalls: number;
  visibleTiles: number;
  loadedTiles: number;
  visibleHeatmapTiles: number;
  loadedHeatmapTiles: number;
  visibleCellChunks: number;
  loadedCellChunks: number;
  visibleCells: number;
  visibleObjects: number;
  gpuMemoryMb: number;
  cpuMemoryMbEstimate: number;
  gpuBufferMemoryBytes: number;
  cpuMemoryBytes: number;
  inflightTileRequests: number;
  inflightOverlayRequests: number;
  inflightHeatmapRequests: number;
}
```

The example UI displays these metrics in the bottom-right panel. A benchmark page is also available:

```text
http://127.0.0.1:5173/benchmark.html?bundle=...&overlay=...&heatmap=...
```

The benchmark writes its latest result to `window.__foveaBenchmark`.

### React Example

```sh
npm -w fovea-react-example run dev
```

Open:

```text
http://127.0.0.1:5174/?bundle=...&overlay=...&heatmap=...
```

### Requirements

- A browser with WebGPU support.
- Static hosting that can serve bundle files by URL.
- Native `fovea-pack` commands require OpenSlide-compatible slide support.
