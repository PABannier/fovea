import initWasm, {
  FoveaViewer as WasmFoveaViewer,
  type FrameStats
} from "../pkg/fovea_viewer.js";

export type BenchmarkPointCount = 10_000 | 100_000 | 500_000 | 1_000_000;

export interface FoveaViewerOptions {
  canvas: HTMLCanvasElement;
  pointCount?: BenchmarkPointCount;
  bundleUrl?: string;
  tileRequestBatchSize?: number;
  maxConcurrentTileRequests?: number;
  onStats?: (stats: FrameStats, rolling: RollingFrameStats) => void;
}

export interface RollingFrameStats {
  frameTimeP50: number;
  frameTimeP95: number;
  frameTimeP99: number;
  fps: number;
}

interface TileRequest {
  level: number;
  x: number;
  y: number;
  width: number;
  height: number;
  path: string;
  priority: number;
}

export class FoveaViewer {
  private animationFrame = 0;
  private destroyed = false;
  private bundleVersion = 0;
  private tileBaseUrl: string | null = null;
  private lastPointer: PointerEvent | null = null;
  private readonly tileRequestBatchSize: number;
  private readonly maxConcurrentTileRequests: number;
  private readonly inflightTiles = new Map<string, AbortController>();
  private readonly frameTimes: number[] = [];
  private readonly resizeObserver: ResizeObserver;

  private constructor(
    private readonly wasm: WasmFoveaViewer,
    private readonly canvas: HTMLCanvasElement,
    options: FoveaViewerOptions
  ) {
    this.tileRequestBatchSize = options.tileRequestBatchSize ?? 96;
    this.maxConcurrentTileRequests = options.maxConcurrentTileRequests ?? 8;
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

    if (options.bundleUrl) {
      await viewer.loadBundle(options.bundleUrl);
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
      const stats = this.wasm.render();
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
    this.resizeObserver.disconnect();
  }

  async loadBundle(bundleUrl: string): Promise<void> {
    const version = ++this.bundleVersion;
    this.abortInflightTiles();

    const manifestUrl = manifestUrlForBundle(bundleUrl);
    const response = await fetch(manifestUrl, { cache: "no-cache" });

    if (!response.ok) {
      throw new Error(`Failed to fetch manifest: ${response.status} ${response.statusText}`);
    }

    const manifestJson = await response.text();

    if (version !== this.bundleVersion || this.destroyed) {
      return;
    }

    this.tileBaseUrl = new URL(".", manifestUrl).toString();
    this.wasm.loadManifest(manifestJson);
    this.frameTimes.length = 0;
  }

  setPointCount(count: BenchmarkPointCount): void {
    this.wasm.setPointCount(count);
  }

  resetCamera(): void {
    this.wasm.resetCamera();
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
    });

    this.canvas.addEventListener("pointermove", (event) => {
      if (!this.lastPointer) {
        return;
      }

      const dpr = window.devicePixelRatio || 1;
      this.wasm.panByScreenDelta(
        (event.clientX - this.lastPointer.clientX) * dpr,
        (event.clientY - this.lastPointer.clientY) * dpr
      );
      this.lastPointer = event;
    });

    this.canvas.addEventListener("pointerup", () => {
      this.lastPointer = null;
    });

    this.canvas.addEventListener("pointercancel", () => {
      this.lastPointer = null;
    });

    this.canvas.addEventListener(
      "wheel",
      (event) => {
        event.preventDefault();
        const rect = this.canvas.getBoundingClientRect();
        const dpr = window.devicePixelRatio || 1;
        this.wasm.zoomAt(
          (event.clientX - rect.left) * dpr,
          (event.clientY - rect.top) * dpr,
          event.deltaY
        );
      },
      { passive: false }
    );
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
      const version = this.bundleVersion;
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

    if (controller.signal.aborted || version !== this.bundleVersion || this.destroyed) {
      return;
    }

    const bitmap = await createImageBitmap(blob);

    try {
      const rgba = decodeBitmapRgba(bitmap, request.width, request.height);

      if (controller.signal.aborted || version !== this.bundleVersion || this.destroyed) {
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
    this.frameTimes.push(stats.frameTimeMs);

    if (this.frameTimes.length > 180) {
      this.frameTimes.shift();
    }

    if (!this.onStats) {
      return;
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

    this.onStats(stats, rolling);
  }

  private abortInflightTiles(): void {
    for (const controller of this.inflightTiles.values()) {
      controller.abort();
    }

    this.inflightTiles.clear();
  }
}

export type { FrameStats };

function manifestUrlForBundle(bundleUrl: string): URL {
  const url = new URL(bundleUrl, window.location.href);

  if (url.pathname.endsWith(".json")) {
    return url;
  }

  const pathname = url.pathname.endsWith("/") ? url.pathname : `${url.pathname}/`;
  return new URL(`${pathname}manifest.json${url.search}`, url);
}

function tileKey(request: TileRequest): string {
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
