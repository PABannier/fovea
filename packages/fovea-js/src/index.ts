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

interface TileRequest {
  level: number;
  x: number;
  y: number;
  width: number;
  height: number;
  path: string;
  priority: number;
}

interface CellChunkRequest {
  x: number;
  y: number;
  path: string;
  cellCount: number;
  byteSize: number;
  priority: number;
}

interface HeatmapTileRequest {
  level: number;
  x: number;
  y: number;
  width: number;
  height: number;
  path: string;
  byteSize: number;
  priority: number;
}

export class FoveaViewer {
  private animationFrame = 0;
  private destroyed = false;
  private slideVersion = 0;
  private cellsVersion = 0;
  private heatmapVersion = 0;
  private tileBaseUrl: string | null = null;
  private cellsBaseUrl: string | null = null;
  private heatmapBaseUrl: string | null = null;
  private lastPointer: PointerEvent | null = null;
  private pointerDown: { clientX: number; clientY: number } | null = null;
  private readonly tileRequestBatchSize: number;
  private readonly cellsRequestBatchSize: number;
  private readonly heatmapRequestBatchSize: number;
  private readonly maxConcurrentTileRequests: number;
  private readonly maxConcurrentCellsRequests: number;
  private readonly maxConcurrentHeatmapRequests: number;
  private readonly inflightTiles = new Map<string, AbortController>();
  private readonly inflightCellChunks = new Map<string, AbortController>();
  private readonly inflightHeatmapTiles = new Map<string, AbortController>();
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
    this.tileRequestBatchSize = options.tileRequestBatchSize ?? 96;
    this.cellsRequestBatchSize = options.cellsRequestBatchSize ?? 64;
    this.heatmapRequestBatchSize = options.heatmapRequestBatchSize ?? 64;
    this.maxConcurrentTileRequests = options.maxConcurrentTileRequests ?? 8;
    this.maxConcurrentCellsRequests = options.maxConcurrentCellsRequests ?? 6;
    this.maxConcurrentHeatmapRequests = options.maxConcurrentHeatmapRequests ?? 6;
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

      this.pumpTileRequests();
      this.pumpHeatmapRequests();
      this.pumpCellRequests();
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
    this.abortInflightTiles();
    this.abortInflightCellChunks();
    this.abortInflightHeatmapTiles();
    this.resizeObserver.disconnect();
  }

  async loadSlide(slideUrl: string): Promise<void> {
    const version = ++this.slideVersion;
    this.abortInflightTiles();

    const manifestUrl = manifestUrlForSource(slideUrl);
    const response = await fetch(manifestUrl, { cache: "no-cache" });

    if (!response.ok) {
      throw new Error(`Failed to fetch slide manifest: ${response.status} ${response.statusText}`);
    }

    const manifestJson = await response.text();

    if (version !== this.slideVersion || this.destroyed) {
      return;
    }

    this.tileBaseUrl = new URL(".", manifestUrl).toString();
    this.wasm.loadManifest(manifestJson);
    this.frameTimes.length = 0;
  }

  async loadCells(cellsUrl: string): Promise<void> {
    const version = ++this.cellsVersion;
    this.abortInflightCellChunks();

    const manifestUrl = manifestUrlForSource(cellsUrl);
    const response = await fetch(manifestUrl, { cache: "no-cache" });

    if (!response.ok) {
      throw new Error(`Failed to fetch cell manifest: ${response.status} ${response.statusText}`);
    }

    const manifestJson = await response.text();

    if (version !== this.cellsVersion || this.destroyed) {
      return;
    }

    this.cellsBaseUrl = new URL(".", manifestUrl).toString();
    this.wasm.loadCellManifest(manifestJson);
    this.frameTimes.length = 0;
  }

  async loadHeatmap(heatmapUrl: string): Promise<void> {
    const version = ++this.heatmapVersion;
    this.abortInflightHeatmapTiles();

    const manifestUrl = manifestUrlForSource(heatmapUrl);
    const response = await fetch(manifestUrl, { cache: "no-cache" });

    if (!response.ok) {
      throw new Error(
        `Failed to fetch heatmap manifest: ${response.status} ${response.statusText}`
      );
    }

    const manifestJson = await response.text();

    if (version !== this.heatmapVersion || this.destroyed) {
      return;
    }

    this.heatmapBaseUrl = new URL(".", manifestUrl).toString();
    this.wasm.loadHeatmapManifest(manifestJson);
    this.frameTimes.length = 0;
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
      inflightTileRequests: this.inflightTiles.size,
      inflightCellRequests: this.inflightCellChunks.size,
      inflightHeatmapRequests: this.inflightHeatmapTiles.size
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

  private pumpTileRequests(): void {
    if (!this.tileBaseUrl || this.destroyed) {
      return;
    }

    const requests = this.parseVisibleTileRequests();
    const wanted = new Set(requests.map((request) => tileKey(request)));

    for (const [key, controller] of this.inflightTiles) {
      if (!wanted.has(key)) {
        controller.abort();
        this.inflightTiles.delete(key);
      }
    }

    for (const request of requests) {
      if (this.inflightTiles.size >= this.maxConcurrentTileRequests) {
        break;
      }

      const key = tileKey(request);

      if (this.inflightTiles.has(key)) {
        continue;
      }

      const controller = new AbortController();
      const version = this.slideVersion;
      this.inflightTiles.set(key, controller);
      void this.loadTile(request, controller, version)
        .catch((error: unknown) => {
          if (!controller.signal.aborted) {
            console.error(error);
          }
        })
        .finally(() => {
          if (this.inflightTiles.get(key) === controller) {
            this.inflightTiles.delete(key);
          }
        });
    }
  }

  private parseVisibleTileRequests(): TileRequest[] {
    const raw = this.wasm.visibleTileRequests(this.tileRequestBatchSize);

    try {
      const requests = JSON.parse(raw) as TileRequest[];
      return requests.sort((a, b) => a.priority - b.priority);
    } catch {
      return [];
    }
  }

  private pumpCellRequests(): void {
    if (!this.cellsBaseUrl || this.destroyed) {
      return;
    }

    const requests = this.parseVisibleCellChunkRequests();
    const wanted = new Set(requests.map((request) => cellChunkKey(request)));

    for (const [key, controller] of this.inflightCellChunks) {
      if (!wanted.has(key)) {
        controller.abort();
        this.inflightCellChunks.delete(key);
      }
    }

    for (const request of requests) {
      if (this.inflightCellChunks.size >= this.maxConcurrentCellsRequests) {
        break;
      }

      const key = cellChunkKey(request);

      if (this.inflightCellChunks.has(key)) {
        continue;
      }

      const controller = new AbortController();
      const version = this.cellsVersion;
      this.inflightCellChunks.set(key, controller);
      void this.loadCellChunk(request, controller, version)
        .catch((error: unknown) => {
          if (!controller.signal.aborted) {
            console.error(error);
          }
        })
        .finally(() => {
          if (this.inflightCellChunks.get(key) === controller) {
            this.inflightCellChunks.delete(key);
          }
        });
    }
  }

  private pumpHeatmapRequests(): void {
    if (!this.heatmapBaseUrl || this.destroyed) {
      return;
    }

    const requests = this.parseVisibleHeatmapTileRequests();
    const wanted = new Set(requests.map((request) => heatmapTileKey(request)));

    for (const [key, controller] of this.inflightHeatmapTiles) {
      if (!wanted.has(key)) {
        controller.abort();
        this.inflightHeatmapTiles.delete(key);
      }
    }

    for (const request of requests) {
      if (this.inflightHeatmapTiles.size >= this.maxConcurrentHeatmapRequests) {
        break;
      }

      const key = heatmapTileKey(request);

      if (this.inflightHeatmapTiles.has(key)) {
        continue;
      }

      const controller = new AbortController();
      const version = this.heatmapVersion;
      this.inflightHeatmapTiles.set(key, controller);
      void this.loadHeatmapTile(request, controller, version)
        .catch((error: unknown) => {
          if (!controller.signal.aborted) {
            console.error(error);
          }
        })
        .finally(() => {
          if (this.inflightHeatmapTiles.get(key) === controller) {
            this.inflightHeatmapTiles.delete(key);
          }
        });
    }
  }

  private parseVisibleHeatmapTileRequests(): HeatmapTileRequest[] {
    const raw = this.wasm.visibleHeatmapTileRequests(this.heatmapRequestBatchSize);

    try {
      const requests = JSON.parse(raw) as HeatmapTileRequest[];
      return requests.sort((a, b) => a.priority - b.priority);
    } catch {
      return [];
    }
  }

  private async loadHeatmapTile(
    request: HeatmapTileRequest,
    controller: AbortController,
    version: number
  ): Promise<void> {
    const heatmapBaseUrl = this.heatmapBaseUrl;

    if (!heatmapBaseUrl) {
      return;
    }

    const tileUrl = new URL(request.path, heatmapBaseUrl);
    const response = await fetch(tileUrl, {
      cache: "force-cache",
      signal: controller.signal
    });

    if (!response.ok) {
      throw new Error(`Failed to fetch heatmap tile ${request.path}: ${response.status}`);
    }

    const bytes = new Uint8Array(await response.arrayBuffer());

    if (controller.signal.aborted || version !== this.heatmapVersion || this.destroyed) {
      return;
    }

    this.wasm.uploadHeatmapTileBytes(
      request.level,
      request.x,
      request.y,
      request.width,
      request.height,
      bytes
    );
  }

  private parseVisibleCellChunkRequests(): CellChunkRequest[] {
    const raw = this.wasm.visibleCellChunkRequests(this.cellsRequestBatchSize);

    try {
      const requests = JSON.parse(raw) as CellChunkRequest[];
      return requests.sort((a, b) => a.priority - b.priority);
    } catch {
      return [];
    }
  }

  private async loadCellChunk(
    request: CellChunkRequest,
    controller: AbortController,
    version: number
  ): Promise<void> {
    const cellsBaseUrl = this.cellsBaseUrl;

    if (!cellsBaseUrl) {
      return;
    }

    const chunkUrl = new URL(request.path, cellsBaseUrl);
    const response = await fetch(chunkUrl, {
      cache: "force-cache",
      signal: controller.signal
    });

    if (!response.ok) {
      throw new Error(`Failed to fetch cell chunk ${request.path}: ${response.status}`);
    }

    const bytes = new Uint8Array(await response.arrayBuffer());

    if (controller.signal.aborted || version !== this.cellsVersion || this.destroyed) {
      return;
    }

    this.wasm.uploadCellChunkBytes(request.x, request.y, bytes);
  }

  private async loadTile(
    request: TileRequest,
    controller: AbortController,
    version: number
  ): Promise<void> {
    const tileBaseUrl = this.tileBaseUrl;

    if (!tileBaseUrl) {
      return;
    }

    const tileUrl = new URL(request.path, tileBaseUrl);
    const response = await fetch(tileUrl, {
      cache: "force-cache",
      signal: controller.signal
    });

    if (!response.ok) {
      throw new Error(`Failed to fetch tile ${request.path}: ${response.status}`);
    }

    const blob = await response.blob();

    if (controller.signal.aborted || version !== this.slideVersion || this.destroyed) {
      return;
    }

    const bitmap = await createImageBitmap(blob);

    try {
      const rgba = decodeBitmapRgba(bitmap, request.width, request.height);

      if (controller.signal.aborted || version !== this.slideVersion || this.destroyed) {
        return;
      }

      this.wasm.uploadTileRgba(
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

  private abortInflightTiles(): void {
    for (const controller of this.inflightTiles.values()) {
      controller.abort();
    }

    this.inflightTiles.clear();
  }

  private abortInflightCellChunks(): void {
    for (const controller of this.inflightCellChunks.values()) {
      controller.abort();
    }

    this.inflightCellChunks.clear();
  }

  private abortInflightHeatmapTiles(): void {
    for (const controller of this.inflightHeatmapTiles.values()) {
      controller.abort();
    }

    this.inflightHeatmapTiles.clear();
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

function tileKey(request: TileRequest): string {
  return `${request.level}/${request.x}/${request.y}`;
}

function cellChunkKey(request: CellChunkRequest): string {
  return `${request.x}/${request.y}`;
}

function heatmapTileKey(request: HeatmapTileRequest): string {
  return `${request.level}/${request.x}/${request.y}`;
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
