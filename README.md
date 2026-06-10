# Fovea

[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](#license)
[![Rust](https://img.shields.io/badge/server-Rust-orange.svg)](https://www.rust-lang.org/)
[![WebGPU](https://img.shields.io/badge/render-WebGPU-5e35b1.svg)](https://developer.mozilla.org/docs/Web/API/WebGPU_API)

**WebGPU whole-slide viewer for pathology slides and AI-native cell outputs — streamed straight from a WSI and a protobuf, no pre-baked bundle.**

Fovea serves OpenSlide-readable whole-slide images and cell masks **directly**, then streams only the slide tiles, cell chunks, and heatmap tiles the current viewport needs for smooth pan/zoom over gigapixel slides and millions of cells.

The browser package is `@fovea/viewer`. The native data server is `fovea-pack`.

<!-- 🎬 DEMO: Record a 15-30s GIF panning/zooming a slide with the cell layer and heatmap
toggled on, ideally showing a cell-click readout. Tools: Kap (macOS), Peek (Linux), or
vhs/asciinema for the terminal. Save it to assets/demo.gif and uncomment the line below. -->
<!-- ![Fovea in action](assets/demo.gif) -->

## Why Fovea?

Computational-pathology viewers usually demand a heavyweight conversion step: tile the slide into a DeepZoom/pyramid bundle, rasterize cell overlays, and ship the whole thing to disk before you can look at anything. Fovea skips that. Point it at a `.svs` and an optional cell-mask protobuf and it serves them live — slide tiles are read from OpenSlide on demand and cached in RAM, cells are decoded once into in-memory spatial chunks, and the optional density heatmap is built in memory at startup. The browser only ever fetches what's on screen.

- **Direct serve, zero pre-processing** — no generated bundle on disk; serve a WSI + protobuf and open the viewer.
- **Gigapixel-ready streaming** — only visible slide tiles, cell chunks, and heatmap tiles are fetched, batched and prioritized per frame.
- **GPU rendering via WebGPU** — slides, cell polygons/points, and heatmaps composite on the GPU at interactive frame rates.
- **Optional density heatmap** — `--heatmap` builds an in-memory heatmap from the cells, with configurable colormap, range, and opacity.
- **Render-only library** — `@fovea/viewer` draws into your canvas and exposes a clean API and events; UI chrome is yours to build.

## Quick Start

**Prerequisites:** a browser with WebGPU support, Node.js, a Rust toolchain, and OpenSlide installed for `fovea-pack`.

Build the workspace:

```sh
npm install
npm run build
```

Serve a slide directly from a WSI and an optional protobuf cell mask file:

```sh
cargo run -p fovea-pack -- serve \
  --wsi /path/to/slide.svs \
  --cells-protobuf /path/to/cell_masks.bin \
  --heatmap \
  --port 7878
```

`fovea-pack` prints the address it's serving and a ready-to-open viewer URL:

```text
fovea-pack serve: listening on http://127.0.0.1:7878
fovea-pack serve: open http://127.0.0.1:5173/?slide=http://127.0.0.1:7878/slide&cells=http://127.0.0.1:7878/cells&heatmap=http://127.0.0.1:7878/heatmap
```

Start the viewer UI and open that URL:

```sh
npm run dev
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

// Filter cells by class. Classes come from the cells manifest.
const classes = viewer.getCellClasses(); // [{ id: 0, name: "tumor" }, ...]
// Per-class visibility, indexed by class id (1 = shown, 0 = hidden).
viewer.setCellClassVisibility([1, 0, 1]); // show class 0 and 2, hide class 1
// Optional per-class colors: 4 floats (r, g, b, a) per class, indexed by id.
viewer.setCellClassColors([1, 0, 0, 1, /* class 1 */ 0, 1, 0, 1]);

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

`fovea-pack` reads sources directly and exposes three endpoints — `/slide`, `/cells`, and `/heatmap` — each with a `manifest.json` plus on-demand tile/chunk paths. There is no bundle build step.

The `/cells/manifest.json` includes a `classes` array — `[{ "id": 0, "name": "tumor" }, ...]` — listing every cell class present on the slide, taken from the cell-mask protobuf. The viewer surfaces this via `viewer.getCellClasses()` and filters by class with `viewer.setCellClassVisibility(...)`.

```sh
cargo run -p fovea-pack -- serve --help
```

Important options:

- `--wsi`: OpenSlide-readable whole-slide image (e.g. `.svs`, `.ndpi`).
- `--cells-protobuf`: Optional `histotyper` `SlideSegmentationData` protobuf payload. Current (`histotyper_v2`) and legacy formats are auto-detected.
- `--heatmap`: Build and serve an in-memory density heatmap from the cell data.
- `--host`: HTTP bind host. Default: `127.0.0.1`.
- `--port`: HTTP bind port. Default: `7878`.
- `--tile-size`: Served slide tile size. Default: `512`.
- `--image-format`: Served slide tile format (`webp` | `jpeg` | `png`). Default: `webp`.
- `--chunk-size`: Cell chunk size in level-0 slide pixels. Default: `4096`.
- `--max-vertices-per-cell`: Polygon vertex cap (`0` for no cap). Default: `256`.
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
- Native `fovea-pack` serving requires an OpenSlide-compatible slide and OpenSlide installed on the host.

## Contributing

Issues and pull requests are welcome. The workspace combines a Rust server (`crates/fovea-pack`), a Rust/WASM renderer (`crates/fovea-viewer`), and a TypeScript wrapper (`packages/fovea-js`). Run the test suite with:

```sh
npm test        # cargo test for fovea-pack and fovea-viewer
npm run check   # type-check the JS package and examples
```

## License

Licensed under the [MIT License](LICENSE).
