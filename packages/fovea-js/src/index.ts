import initWasm, { FoveaViewer as WasmFoveaViewer, type FrameStats } from "../pkg/fovea_viewer.js";

export type BenchmarkPointCount = 10_000 | 100_000 | 500_000 | 1_000_000;

export interface FoveaViewerOptions {
  canvas: HTMLCanvasElement;
  pointCount?: BenchmarkPointCount;
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

export interface RollingFrameStats {
  frameTimeP50: number;
  frameTimeP95: number;
  frameTimeP99: number;
  fps: number;
}

export interface PerformanceStats {
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
  inflightCellRequests: number;
  inflightHeatmapRequests: number;
}

export interface CellEvent {
  cellId: number | null;
  classId: number | null;
  slideX: number | null;
  slideY: number | null;
}

export interface CellClass {
  id: number;
  name: string;
}

export interface SelectionChangeEvent {
  count: number;
}

export interface ViewportChangeEvent {
  centerX: number;
  centerY: number;
  zoom: number;
}

export interface FoveaViewerEvents {
  "cell-hover": CellEvent;
  "cell-click": CellEvent;
  "selection-change": SelectionChangeEvent;
  "viewport-change": ViewportChangeEvent;
}

type EventCallback<T> = (event: T) => void;

type RawViewerEvent =
  | ({ type: "cell-hover" } & CellEvent)
  | ({ type: "cell-click" } & CellEvent)
  | ({ type: "selection-change" } & SelectionChangeEvent)
  | ({ type: "viewport-changed" } & ViewportChangeEvent);

/**
 * A tile/chunk request from the wasm `visible*Requests` calls, already sorted by
 * priority. Cell chunk requests only carry `x`, `y` and `path`.
 */
interface LayerRequest {
  level: number;
  x: number;
  y: number;
  width: number;
  height: number;
  path: string;
}

/** Streaming state shared by the slide, cells and heatmap layers. */
interface Layer {
  readonly name: string;
  version: number;
  baseUrl: string | null;
  readonly inflight: Map<string, AbortController>;
  readonly maxConcurrent: number;
  readonly requests: () => string;
  readonly loadManifest: (json: string) => void;
  readonly upload: (
    request: LayerRequest,
    response: Response,
    isCurrent: () => boolean
  ) => Promise<void>;
}

export class FoveaViewer {
  private animationFrame = 0;
  private destroyed = false;
  private lastPointer: PointerEvent | null = null;
  private pointerDown: { clientX: number; clientY: number } | null = null;
  private readonly slide: Layer;
  private readonly cells: Layer;
  private readonly heatmap: Layer;
  private readonly eventListeners = new Map<keyof FoveaViewerEvents, Set<EventCallback<any>>>();
  private readonly frameTimes: number[] = [];
  private readonly resizeObserver: ResizeObserver;
  private lastStats: FrameStats | null = null;
  private lastRollingStats: RollingFrameStats = emptyRollingStats();

  private constructor(
    private readonly wasm: WasmFoveaViewer,
    private readonly canvas: HTMLCanvasElement,
    options: FoveaViewerOptions
  ) {
    const tileBatch = options.tileRequestBatchSize ?? 96;
    const cellsBatch = options.cellsRequestBatchSize ?? 64;
    const heatmapBatch = options.heatmapRequestBatchSize ?? 64;
    this.slide = {
      name: "slide",
      version: 0,
      baseUrl: null,
      inflight: new Map(),
      maxConcurrent: options.maxConcurrentTileRequests ?? 8,
      requests: () => wasm.visibleTileRequests(tileBatch),
      loadManifest: (json) => wasm.loadManifest(json),
      upload: async (request, response, isCurrent) => {
        const blob = await response.blob();

        if (!isCurrent()) {
          return;
        }

        const bitmap = await createImageBitmap(blob);

        try {
          const rgba = decodeBitmapRgba(bitmap, request.width, request.height);

          if (!isCurrent()) {
            return;
          }

          wasm.uploadTileRgba(
            request.level,
            request.x,
            request.y,
            rgba.width,
            rgba.height,
            rgba.data
          );
        } finally {
          bitmap.close();
        }
      }
    };
    this.cells = {
      name: "cell",
      version: 0,
      baseUrl: null,
      inflight: new Map(),
      maxConcurrent: options.maxConcurrentCellsRequests ?? 6,
      requests: () => wasm.visibleCellChunkRequests(cellsBatch),
      loadManifest: (json) => wasm.loadCellManifest(json),
      upload: async (request, response, isCurrent) => {
        const bytes = new Uint8Array(await response.arrayBuffer());

        if (isCurrent()) {
          wasm.uploadCellChunkBytes(request.x, request.y, bytes);
        }
      }
    };
    this.heatmap = {
      name: "heatmap",
      version: 0,
      baseUrl: null,
      inflight: new Map(),
      maxConcurrent: options.maxConcurrentHeatmapRequests ?? 6,
      requests: () => wasm.visibleHeatmapTileRequests(heatmapBatch),
      loadManifest: (json) => wasm.loadHeatmapManifest(json),
      upload: async (request, response, isCurrent) => {
        const bytes = new Uint8Array(await response.arrayBuffer());

        if (isCurrent()) {
          wasm.uploadHeatmapTileBytes(
            request.level,
            request.x,
            request.y,
            request.width,
            request.height,
            bytes
          );
        }
      }
    };
    this.resizeObserver = new ResizeObserver(() => this.resize());
    this.resizeObserver.observe(canvas);
    this.bindInput();
    this.resize();
    this.onStats = options.onStats;
  }

  private readonly onStats?: (stats: FrameStats, rolling: RollingFrameStats) => void;

  static async create(options: FoveaViewerOptions): Promise<FoveaViewer> {
    await initWasm();

    const wasm = await WasmFoveaViewer.create(options.canvas);
    const viewer = new FoveaViewer(wasm, options.canvas, options);

    if (options.pointCount) {
      viewer.setPointCount(options.pointCount);
    }

    if (options.slideUrl) {
      await viewer.loadSlide(options.slideUrl);
    }

    if (options.cellsUrl) {
      await viewer.loadCells(options.cellsUrl);
    }

    if (options.heatmapUrl) {
      await viewer.loadHeatmap(options.heatmapUrl);
    }

    return viewer;
  }

  start(): void {
    if (this.animationFrame !== 0) {
      return;
    }

    const tick = () => {
      if (this.destroyed) {
        return;
      }

      for (const layer of [this.slide, this.heatmap, this.cells]) {
        this.pump(layer);
      }
      const stats = this.wasm.render();
      this.dispatchDrainedEvents();
      this.observeFrame(stats);
      this.animationFrame = requestAnimationFrame(tick);
    };

    this.animationFrame = requestAnimationFrame(tick);
  }

  stop(): void {
    if (this.animationFrame !== 0) {
      cancelAnimationFrame(this.animationFrame);
      this.animationFrame = 0;
    }
  }

  destroy(): void {
    this.destroyed = true;
    this.stop();
    for (const layer of [this.slide, this.cells, this.heatmap]) {
      abortInflight(layer);
    }
    this.resizeObserver.disconnect();
  }

  loadSlide(slideUrl: string): Promise<void> {
    return this.loadLayer(this.slide, slideUrl);
  }

  loadCells(cellsUrl: string): Promise<void> {
    return this.loadLayer(this.cells, cellsUrl);
  }

  loadHeatmap(heatmapUrl: string): Promise<void> {
    return this.loadLayer(this.heatmap, heatmapUrl);
  }

  setPointCount(count: BenchmarkPointCount): void {
    this.wasm.setPointCount(count);
  }

  setLayerVisibility(layerId: "cells" | "heatmap" | string, visible: boolean): void {
    if (layerId === "cells") {
      this.wasm.setCellVisibility(visible);
    } else {
      this.wasm.setHeatmapVisibility(visible);
    }
  }

  setLayerOpacity(layerId: "cells" | "heatmap" | string, opacity: number): void {
    if (layerId === "cells") {
      this.wasm.setCellOpacity(opacity);
    } else {
      this.wasm.setHeatmapOpacity(opacity);
    }
  }

  setHeatmapRange(_layerId: "heatmap" | string, range: { min: number; max: number }): void {
    this.wasm.setHeatmapRange(range.min, range.max);
  }

  setHeatmapColormap(_layerId: "heatmap" | string, colormap: "magma" | "viridis" | "gray"): void {
    this.wasm.setHeatmapColormap(colormap);
  }

  setCellPointSize(sizePx: number): void {
    this.wasm.setCellPointSize(sizePx);
  }

  setCellOutlineWidth(widthPx: number): void {
    this.wasm.setCellOutlineWidth(widthPx);
  }

  /**
   * Returns the cell classes present on the loaded slide, as declared in the
   * cells manifest. Available once `loadCells()` resolves; returns `[]` before
   * any cells are loaded.
   */
  getCellClasses(): CellClass[] {
    try {
      return JSON.parse(this.wasm.getCellClasses()) as CellClass[];
    } catch {
      return [];
    }
  }

  /**
   * Set per-class cell colors. `rgba` is a flat array of 4 numbers (r, g, b, a in
   * 0..1) per class, indexed by classId. Up to 64 classes are stored.
   */
  setCellClassColors(rgba: ArrayLike<number>): void {
    const data = rgba instanceof Float32Array ? rgba : Float32Array.from(rgba);
    this.wasm.setCellClassColors(data);
  }

  /**
   * Set per-class cell visibility, indexed by classId. A truthy entry shows the
   * class; hidden classes are neither drawn nor hoverable.
   */
  setCellClassVisibility(flags: ArrayLike<boolean | number>): void {
    const data = new Uint8Array(flags.length);
    for (let i = 0; i < flags.length; i += 1) {
      data[i] = flags[i] ? 1 : 0;
    }
    this.wasm.setCellClassVisibility(data);
  }

  on<K extends keyof FoveaViewerEvents>(
    eventName: K,
    callback: EventCallback<FoveaViewerEvents[K]>
  ): () => void {
    let listeners = this.eventListeners.get(eventName);

    if (!listeners) {
      listeners = new Set();
      this.eventListeners.set(eventName, listeners);
    }

    listeners.add(callback as EventCallback<any>);
    return () => listeners?.delete(callback as EventCallback<any>);
  }

  resetCamera(): void {
    this.wasm.resetCamera();
  }

  panByScreenDelta(deltaX: number, deltaY: number): void {
    this.wasm.panByScreenDelta(deltaX, deltaY);
  }

  zoomAtCanvasPoint(x: number, y: number, wheelDeltaY: number): void {
    this.wasm.zoomAt(x, y, wheelDeltaY);
  }

  /**
   * Current camera in slide-pixel coordinates. `zoom` is CSS-px per slide-px
   * (the visible slide width in pixels is `canvasCssWidth / zoom`).
   */
  getCamera(): { centerX: number; centerY: number; zoom: number } {
    const c = this.wasm.getCamera();
    return { centerX: c[0], centerY: c[1], zoom: c[2] };
  }

  /**
   * Apply a camera directly (slide-pixel center + CSS-px-per-slide-px zoom),
   * clamped to the world. Does NOT emit a `viewport-change` event — use it to
   * apply a remote/programmatic viewport without echoing it back.
   */
  setCamera(centerX: number, centerY: number, zoom: number): void {
    this.wasm.setCamera(centerX, centerY, zoom);
  }

  /** Convert canvas/CSS-pixel coordinates to slide-pixel coordinates. */
  screenToSlide(x: number, y: number): { x: number; y: number } {
    const p = this.wasm.screenToSlide(x, y);
    return { x: p[0], y: p[1] };
  }

  /** Convert slide-pixel coordinates to canvas/CSS-pixel coordinates. */
  slideToScreen(x: number, y: number): { x: number; y: number } {
    const p = this.wasm.slideToScreen(x, y);
    return { x: p[0], y: p[1] };
  }

  getPerformanceStats(): PerformanceStats {
    const stats = this.lastStats;
    const rolling = this.lastRollingStats;
    const gpuBytes = stats?.gpuBufferMemoryBytes ?? 0;
    const cpuBytes = stats?.cpuMemoryBytes ?? 0;

    return {
      fps: rolling.fps,
      frameTimeP50Ms: rolling.frameTimeP50,
      frameTimeP95Ms: rolling.frameTimeP95,
      frameTimeP99Ms: rolling.frameTimeP99,
      frameTimeMs: stats?.frameTimeMs ?? 0,
      uploadTimeMs: stats?.uploadTimeMs ?? 0,
      drawCalls: stats?.drawCallCount ?? 0,
      visibleTiles: stats?.visibleTileCount ?? 0,
      loadedTiles: stats?.loadedTileCount ?? 0,
      visibleHeatmapTiles: stats?.visibleHeatmapTileCount ?? 0,
      loadedHeatmapTiles: stats?.loadedHeatmapTileCount ?? 0,
      visibleCellChunks: stats?.visibleCellChunkCount ?? 0,
      loadedCellChunks: stats?.loadedCellChunkCount ?? 0,
      visibleCells: stats?.visibleCellCount ?? 0,
      visibleObjects: stats?.visibleObjectCount ?? 0,
      gpuMemoryMb: bytesToMiB(gpuBytes),
      cpuMemoryMbEstimate: bytesToMiB(cpuBytes),
      gpuBufferMemoryBytes: gpuBytes,
      cpuMemoryBytes: cpuBytes,
      inflightTileRequests: this.slide.inflight.size,
      inflightCellRequests: this.cells.inflight.size,
      inflightHeatmapRequests: this.heatmap.inflight.size
    };
  }

  private resize(): void {
    const rect = this.canvas.getBoundingClientRect();
    const dpr = window.devicePixelRatio || 1;
    const width = Math.max(1, Math.round(rect.width * dpr));
    const height = Math.max(1, Math.round(rect.height * dpr));

    if (this.canvas.width !== width || this.canvas.height !== height) {
      this.canvas.width = width;
      this.canvas.height = height;
    }

    this.wasm.resize(width, height, dpr);
  }

  private bindInput(): void {
    this.canvas.addEventListener("pointerdown", (event) => {
      this.canvas.setPointerCapture(event.pointerId);
      this.lastPointer = event;
      this.pointerDown = {
        clientX: event.clientX,
        clientY: event.clientY
      };
    });

    this.canvas.addEventListener("pointermove", (event) => {
      if (!this.lastPointer) {
        this.hoverAtEvent(event);
        return;
      }

      this.wasm.panByScreenDelta(
        event.clientX - this.lastPointer.clientX,
        event.clientY - this.lastPointer.clientY
      );
      this.lastPointer = event;
    });

    this.canvas.addEventListener("pointerup", (event) => {
      if (this.pointerDown) {
        const moved = Math.hypot(
          event.clientX - this.pointerDown.clientX,
          event.clientY - this.pointerDown.clientY
        );

        if (moved <= 4) {
          this.clickAtEvent(event);
        } else {
          this.hoverAtEvent(event);
        }
      }

      this.lastPointer = null;
      this.pointerDown = null;
    });

    this.canvas.addEventListener("pointercancel", () => {
      this.lastPointer = null;
      this.pointerDown = null;
    });

    this.canvas.addEventListener(
      "wheel",
      (event) => {
        event.preventDefault();
        const point = this.eventCanvasPoint(event);
        this.wasm.zoomAt(point.x, point.y, event.deltaY);
      },
      { passive: false }
    );
  }

  private hoverAtEvent(event: PointerEvent): void {
    const point = this.eventCanvasPoint(event);
    this.wasm.hoverAt(point.x, point.y);
  }

  private clickAtEvent(event: PointerEvent): void {
    const point = this.eventCanvasPoint(event);
    this.wasm.clickAt(point.x, point.y);
  }

  private eventCanvasPoint(event: Pick<MouseEvent, "clientX" | "clientY">): {
    x: number;
    y: number;
  } {
    const rect = this.canvas.getBoundingClientRect();

    return {
      x: event.clientX - rect.left,
      y: event.clientY - rect.top
    };
  }

  private async loadLayer(layer: Layer, url: string): Promise<void> {
    const version = ++layer.version;
    abortInflight(layer);

    const manifestUrl = manifestUrlForSource(url);
    const response = await fetch(manifestUrl, { cache: "no-cache" });

    if (!response.ok) {
      throw new Error(
        `Failed to fetch ${layer.name} manifest: ${response.status} ${response.statusText}`
      );
    }

    const manifestJson = await response.text();

    if (version !== layer.version || this.destroyed) {
      return;
    }

    layer.baseUrl = new URL(".", manifestUrl).toString();
    layer.loadManifest(manifestJson);
    this.frameTimes.length = 0;
  }

  private pump(layer: Layer): void {
    const baseUrl = layer.baseUrl;

    if (!baseUrl || this.destroyed) {
      return;
    }

    // Requests arrive priority-sorted from wasm.
    const raw = layer.requests();
    let requests: LayerRequest[];

    try {
      requests = JSON.parse(raw) as LayerRequest[];
    } catch {
      requests = [];
    }

    const wanted = new Set(requests.map(requestKey));

    for (const [key, controller] of layer.inflight) {
      if (!wanted.has(key)) {
        controller.abort();
        layer.inflight.delete(key);
      }
    }

    for (const request of requests) {
      if (layer.inflight.size >= layer.maxConcurrent) {
        break;
      }

      const key = requestKey(request);

      if (layer.inflight.has(key)) {
        continue;
      }

      const controller = new AbortController();
      const version = layer.version;
      const isCurrent = () =>
        !controller.signal.aborted && version === layer.version && !this.destroyed;
      layer.inflight.set(key, controller);
      void (async () => {
        const response = await fetch(new URL(request.path, baseUrl), {
          cache: "force-cache",
          signal: controller.signal
        });

        if (!response.ok) {
          throw new Error(`Failed to fetch ${layer.name} data ${request.path}: ${response.status}`);
        }

        await layer.upload(request, response, isCurrent);
      })()
        .catch((error: unknown) => {
          if (!controller.signal.aborted) {
            console.error(error);
          }
        })
        .finally(() => {
          if (layer.inflight.get(key) === controller) {
            layer.inflight.delete(key);
          }
        });
    }
  }

  private observeFrame(stats: FrameStats): void {
    this.lastStats = stats;
    this.frameTimes.push(stats.frameTimeMs);

    if (this.frameTimes.length > 180) {
      this.frameTimes.shift();
    }

    const sorted = [...this.frameTimes].sort((a, b) => a - b);
    const percentile = (p: number) =>
      sorted[Math.min(sorted.length - 1, Math.floor((sorted.length - 1) * p))] ?? 0;
    const p50 = percentile(0.5);
    const rolling = {
      frameTimeP50: p50,
      frameTimeP95: percentile(0.95),
      frameTimeP99: percentile(0.99),
      fps: p50 > 0 ? 1000 / p50 : 0
    };
    this.lastRollingStats = rolling;

    if (!this.onStats) {
      return;
    }

    this.onStats(stats, rolling);
  }

  private dispatchDrainedEvents(): void {
    let events: RawViewerEvent[];

    try {
      events = JSON.parse(this.wasm.drainEvents()) as RawViewerEvent[];
    } catch {
      return;
    }

    for (const event of events) {
      if (event.type === "viewport-changed") {
        this.dispatchEvent("viewport-change", {
          centerX: event.centerX,
          centerY: event.centerY,
          zoom: event.zoom
        });
        continue;
      }

      this.dispatchEvent(event.type, event as FoveaViewerEvents[typeof event.type]);
    }
  }

  private dispatchEvent<K extends keyof FoveaViewerEvents>(
    eventName: K,
    event: FoveaViewerEvents[K]
  ): void {
    const listeners = this.eventListeners.get(eventName);

    if (!listeners) {
      return;
    }

    for (const listener of listeners) {
      listener(event);
    }
  }
}

export type { FrameStats };

function emptyRollingStats(): RollingFrameStats {
  return {
    frameTimeP50: 0,
    frameTimeP95: 0,
    frameTimeP99: 0,
    fps: 0
  };
}

function bytesToMiB(bytes: number): number {
  return bytes / (1024 * 1024);
}

function manifestUrlForSource(slideUrl: string): URL {
  const url = new URL(slideUrl, window.location.href);

  if (url.pathname.endsWith(".json")) {
    return url;
  }

  const pathname = url.pathname.endsWith("/") ? url.pathname : `${url.pathname}/`;
  return new URL(`${pathname}manifest.json${url.search}`, url);
}

function requestKey(request: LayerRequest): string {
  return `${request.level}/${request.x}/${request.y}`;
}

function abortInflight(layer: Layer): void {
  for (const controller of layer.inflight.values()) {
    controller.abort();
  }

  layer.inflight.clear();
}

function decodeBitmapRgba(
  bitmap: ImageBitmap,
  expectedWidth: number,
  expectedHeight: number
): { data: Uint8Array; width: number; height: number } {
  const width = bitmap.width || expectedWidth;
  const height = bitmap.height || expectedHeight;
  const canvas =
    typeof OffscreenCanvas !== "undefined"
      ? new OffscreenCanvas(width, height)
      : document.createElement("canvas");

  canvas.width = width;
  canvas.height = height;

  const context = canvas.getContext("2d", { willReadFrequently: true });

  if (!context) {
    throw new Error("2D canvas context unavailable for tile decode");
  }

  context.clearRect(0, 0, width, height);
  context.drawImage(bitmap, 0, 0, width, height);

  const imageData = context.getImageData(0, 0, width, height);
  return {
    data: new Uint8Array(
      imageData.data.buffer,
      imageData.data.byteOffset,
      imageData.data.byteLength
    ),
    width,
    height
  };
}
