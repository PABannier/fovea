# Fovea

Fovea is a WebGPU whole-slide image viewer for pathology slides and AI-native cell outputs. It serves OpenSlide-readable WSI files and histotyper protobuf cell masks directly, then streams only the visible slide tiles, cell chunks, and heatmap tiles needed for smooth pan/zoom.

The browser package is `@fovea/viewer`. The native data server is `fovea-pack`.

## Get Started

Build the workspace:

```sh
npm install
npm run build
```

Start the viewer UI:

```sh
npm run dev
```

Serve a slide directly from a WSI and optional protobuf cell mask file:

```sh
cargo run -p fovea-pack -- serve \
  --wsi /path/to/slide.svs \
  --cells-protobuf /path/to/cell_masks.bin \
  --heatmap \
  --port 7878
```

Open the URL printed by `fovea-pack`, for example:

```text
http://127.0.0.1:5173/?slide=http://127.0.0.1:7878/slide&cells=http://127.0.0.1:7878/cells&heatmap=http://127.0.0.1:7878/heatmap
```

The example viewer hides controls and performance metrics by default. Add `controls=1&performance=1` to show them.

## Documentation

### Browser Usage

```ts
import { FoveaViewer } from "@fovea/viewer";

const canvas = document.querySelector<HTMLCanvasElement>("#viewer");

if (!canvas) {
  throw new Error("Missing canvas");
}

const viewer = await FoveaViewer.create({
  canvas,
  slideUrl: "http://127.0.0.1:7878/slide",
  cellsUrl: "http://127.0.0.1:7878/cells",
  heatmapUrl: "http://127.0.0.1:7878/heatmap"
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

`@fovea/viewer` renders only into the canvas. It does not create loader controls, status panels, or performance panels; those are example-app UI.

### Viewer Options

```ts
interface FoveaViewerOptions {
  canvas: HTMLCanvasElement;
  pointCount?: 10_000 | 100_000 | 500_000 | 1_000_000;
  slideUrl?: string;
  cellsUrl?: string;
  heatmapUrl?: string;
  tileRequestBatchSize?: number;
  cellsRequestBatchSize?: number;
  heatmapRequestBatchSize?: number;
  maxConcurrentTileRequests?: number;
  maxConcurrentCellsRequests?: number;
  maxConcurrentHeatmapRequests?: number;
  onStats?: (stats: FrameStats, rolling: RollingFrameStats) => void;
}
```

- `canvas`: Required render target.
- `pointCount`: Synthetic benchmark point count used when no slide, cells, or heatmap are loaded.
- `slideUrl`: Direct server slide endpoint, usually `/slide`.
- `cellsUrl`: Direct server cell endpoint, usually `/cells`.
- `heatmapUrl`: Direct server heatmap endpoint, usually `/heatmap`.
- `tileRequestBatchSize`: Max slide tile requests considered per frame. Default: `96`.
- `cellsRequestBatchSize`: Max cell chunk requests considered per frame. Default: `64`.
- `heatmapRequestBatchSize`: Max heatmap tile requests considered per frame. Default: `64`.
- `maxConcurrentTileRequests`: Parallel slide tile fetches. Default: `8`.
- `maxConcurrentCellsRequests`: Parallel cell chunk fetches. Default: `6`.
- `maxConcurrentHeatmapRequests`: Parallel heatmap tile fetches. Default: `6`.
- `onStats`: Per-frame metrics callback.

### Viewer Methods

```ts
viewer.start();
viewer.stop();
viewer.destroy();

await viewer.loadSlide("http://127.0.0.1:7878/slide");
await viewer.loadCells("http://127.0.0.1:7878/cells");
await viewer.loadHeatmap("http://127.0.0.1:7878/heatmap");

viewer.resetCamera();
viewer.panByScreenDelta(20, 0);
viewer.zoomAtCanvasPoint(400, 300, -120);

viewer.setLayerVisibility("cells", true);
viewer.setLayerVisibility("heatmap", true);
viewer.setLayerOpacity("cells", 0.75);
viewer.setLayerOpacity("heatmap", 0.4);

viewer.setCellPointSize(3);
viewer.setCellOutlineWidth(1.25);

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

### Direct Server

```sh
cargo run -p fovea-pack -- serve --help
```

Important options:

- `--wsi`: OpenSlide-readable whole-slide image.
- `--cells-protobuf`: Optional `new_cell_masks.proto` or legacy `cell_masks.proto` protobuf payload.
- `--heatmap`: Build an in-memory density heatmap from the cell data.
- `--host`: HTTP bind host. Default: `127.0.0.1`.
- `--port`: HTTP bind port. Default: `7878`.
- `--tile-size`: Served slide tile size. Default: `512`.
- `--image-format`: Served slide tile format. Default: `webp`.
- `--chunk-size`: Cell chunk size in level-0 slide pixels. Default: `4096`.
- `--max-vertices-per-cell`: Polygon vertex cap. Default: `256`.
- `--heatmap-bin-size`: Level-0 slide pixels per heatmap pixel. Default: `128`.
- `--heatmap-tile-size`: Heatmap tile edge in heatmap pixels. Default: `256`.
- `--tile-cache-mb`: RAM cache budget for encoded slide tiles. Default: `1024`.

Cell chunks and heatmap tiles are prepared in memory at startup. Slide tiles are read from OpenSlide on demand, encoded, and cached in RAM.

### Performance Metrics

```ts
const stats = viewer.getPerformanceStats();
```

The result includes FPS, p50/p95/p99 frame times, upload time, draw count, visible/loaded slide tiles, visible/loaded heatmap tiles, visible/loaded cell chunks, visible cells, GPU memory, CPU memory estimate, and in-flight request counts.

Benchmark page:

```text
http://127.0.0.1:5173/benchmark.html?slide=...&cells=...&heatmap=...
```

The benchmark writes its latest result to `window.__foveaBenchmark`.

### React Example

```sh
npm -w fovea-react-example run dev
```

Open:

```text
http://127.0.0.1:5174/?slide=...&cells=...&heatmap=...
```

### Requirements

- A browser with WebGPU support.
- Native `fovea-pack` serving requires OpenSlide-compatible slide support.
