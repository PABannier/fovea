import { FoveaViewer } from "@fovea/viewer";
import "./styles.css";

const canvas = document.querySelector<HTMLCanvasElement>("#viewer");
const resetButton = document.querySelector<HTMLButtonElement>("#reset-camera");
const bundleForm = document.querySelector<HTMLFormElement>("#bundle-form");
const bundleInput = document.querySelector<HTMLInputElement>("#bundle-url");
const overlayForm = document.querySelector<HTMLFormElement>("#overlay-form");
const overlayInput = document.querySelector<HTMLInputElement>("#overlay-url");
const heatmapForm = document.querySelector<HTMLFormElement>("#heatmap-form");
const heatmapInput = document.querySelector<HTMLInputElement>("#heatmap-url");
const overlayVisible = document.querySelector<HTMLInputElement>("#overlay-visible");
const overlayOpacity = document.querySelector<HTMLInputElement>("#overlay-opacity");
const heatmapVisible = document.querySelector<HTMLInputElement>("#heatmap-visible");
const heatmapOpacity = document.querySelector<HTMLInputElement>("#heatmap-opacity");
const heatmapMin = document.querySelector<HTMLInputElement>("#heatmap-min");
const heatmapMax = document.querySelector<HTMLInputElement>("#heatmap-max");
const heatmapColormap = document.querySelector<HTMLSelectElement>("#heatmap-colormap");
const loadStatus = document.querySelector<HTMLElement>("#load-status");

if (
  !canvas ||
  !resetButton ||
  !bundleForm ||
  !bundleInput ||
  !overlayForm ||
  !overlayInput ||
  !heatmapForm ||
  !heatmapInput ||
  !overlayVisible ||
  !overlayOpacity ||
  !heatmapVisible ||
  !heatmapOpacity ||
  !heatmapMin ||
  !heatmapMax ||
  !heatmapColormap ||
  !loadStatus
) {
  throw new Error("Fovea example DOM is incomplete");
}

const viewerCanvas = canvas;
const resetCameraButton = resetButton;
const bundleUrlForm = bundleForm;
const bundleUrlInput = bundleInput;
const overlayUrlForm = overlayForm;
const overlayUrlInput = overlayInput;
const heatmapUrlForm = heatmapForm;
const heatmapUrlInput = heatmapInput;
const overlayVisibleInput = overlayVisible;
const overlayOpacityInput = overlayOpacity;
const heatmapVisibleInput = heatmapVisible;
const heatmapOpacityInput = heatmapOpacity;
const heatmapMinInput = heatmapMin;
const heatmapMaxInput = heatmapMax;
const heatmapColormapSelect = heatmapColormap;
const bundleLoadStatus = loadStatus;
let slideLoaded = false;
let overlayLoaded = false;
let heatmapLoaded = false;

const values = {
  fps: document.querySelector<HTMLElement>("#fps"),
  p50: document.querySelector<HTMLElement>("#p50"),
  p95: document.querySelector<HTMLElement>("#p95"),
  p99: document.querySelector<HTMLElement>("#p99"),
  frame: document.querySelector<HTMLElement>("#frame"),
  upload: document.querySelector<HTMLElement>("#upload"),
  draws: document.querySelector<HTMLElement>("#draws"),
  visible: document.querySelector<HTMLElement>("#visible"),
  cell: document.querySelector<HTMLElement>("#cell"),
  gpu: document.querySelector<HTMLElement>("#gpu"),
  cpu: document.querySelector<HTMLElement>("#cpu")
};

function setText(key: keyof typeof values, text: string): void {
  const element = values[key];

  if (element) {
    element.textContent = text;
  }
}

function formatMs(value: number): string {
  return `${value.toFixed(2)} ms`;
}

function formatBytes(value: number): string {
  return `${(value / (1024 * 1024)).toFixed(2)} MB`;
}

async function main(): Promise<void> {
  const bundleParam = new URLSearchParams(window.location.search).get("bundle");
  const overlayParam = new URLSearchParams(window.location.search).get("overlay");
  const heatmapParam = new URLSearchParams(window.location.search).get("heatmap");

  if (bundleParam) {
    bundleUrlInput.value = bundleParam;
  }

  if (overlayParam) {
    overlayUrlInput.value = overlayParam;
  }

  if (heatmapParam) {
    heatmapUrlInput.value = heatmapParam;
  }

  const viewer = await FoveaViewer.create({
    canvas: viewerCanvas,
    onStats: (stats, rolling) => {
      setText("fps", rolling.fps.toFixed(1));
      setText("p50", formatMs(rolling.frameTimeP50));
      setText("p95", formatMs(rolling.frameTimeP95));
      setText("p99", formatMs(rolling.frameTimeP99));
      setText("frame", formatMs(stats.frameTimeMs));
      setText("upload", formatMs(stats.uploadTimeMs));
      setText("draws", String(stats.drawCallCount));
      setText("visible", stats.visibleObjectCount.toLocaleString());
      setText("gpu", formatBytes(stats.gpuBufferMemoryBytes));
      setText("cpu", formatBytes(stats.cpuMemoryBytes));
    }
  });

  resetCameraButton.addEventListener("click", () => viewer.resetCamera());
  overlayVisibleInput.addEventListener("change", () => {
    viewer.setLayerVisibility("cells", overlayVisibleInput.checked);
  });
  overlayOpacityInput.addEventListener("input", () => {
    viewer.setLayerOpacity("cells", Number(overlayOpacityInput.value));
  });
  heatmapVisibleInput.addEventListener("change", () => {
    viewer.setLayerVisibility("heatmap", heatmapVisibleInput.checked);
  });
  heatmapOpacityInput.addEventListener("input", () => {
    viewer.setLayerOpacity("heatmap", Number(heatmapOpacityInput.value));
  });
  heatmapMinInput.addEventListener("input", () => updateHeatmapRange(viewer));
  heatmapMaxInput.addEventListener("input", () => updateHeatmapRange(viewer));
  heatmapColormapSelect.addEventListener("change", () => {
    viewer.setHeatmapColormap(
      "heatmap",
      heatmapColormapSelect.value as "magma" | "viridis" | "gray"
    );
  });
  viewer.on("cell-hover", (event) => {
    if (event.cellId == null) {
      setText("cell", "None");
      return;
    }

    setText("cell", `${event.cellId} / class ${event.classId ?? "?"}`);
  });

  bundleUrlForm.addEventListener("submit", (event) => {
    event.preventDefault();
    void loadBundleFromInput(viewer);
  });

  overlayUrlForm.addEventListener("submit", (event) => {
    event.preventDefault();
    void loadOverlayFromInput(viewer);
  });

  heatmapUrlForm.addEventListener("submit", (event) => {
    event.preventDefault();
    void loadHeatmapFromInput(viewer);
  });

  viewer.start();

  if (bundleUrlInput.value.trim()) {
    void loadBundleFromInput(viewer);
  }

  if (overlayUrlInput.value.trim()) {
    void loadOverlayFromInput(viewer);
  }

  if (heatmapUrlInput.value.trim()) {
    void loadHeatmapFromInput(viewer);
  }
}

async function loadBundleFromInput(viewer: FoveaViewer): Promise<void> {
  const bundleUrl = bundleUrlInput.value.trim();

  if (!bundleUrl) {
    return;
  }

  bundleLoadStatus.textContent = "Loading";

  try {
    await viewer.loadBundle(bundleUrl);
    slideLoaded = true;
    updateLoadStatus();
  } catch (error) {
    slideLoaded = false;
    bundleLoadStatus.textContent = "Load failed";
    console.error(error);
  }
}

async function loadOverlayFromInput(viewer: FoveaViewer): Promise<void> {
  const overlayUrl = overlayUrlInput.value.trim();

  if (!overlayUrl) {
    return;
  }

  bundleLoadStatus.textContent = "Overlay";

  try {
    await viewer.loadOverlay(overlayUrl);
    overlayLoaded = true;
    updateLoadStatus();
  } catch (error) {
    overlayLoaded = false;
    bundleLoadStatus.textContent = "Overlay failed";
    console.error(error);
  }
}

async function loadHeatmapFromInput(viewer: FoveaViewer): Promise<void> {
  const heatmapUrl = heatmapUrlInput.value.trim();

  if (!heatmapUrl) {
    return;
  }

  bundleLoadStatus.textContent = "Heatmap";

  try {
    await viewer.loadHeatmap(heatmapUrl);
    heatmapLoaded = true;
    updateLoadStatus();
  } catch (error) {
    heatmapLoaded = false;
    bundleLoadStatus.textContent = "Heatmap failed";
    console.error(error);
  }
}

function updateHeatmapRange(viewer: FoveaViewer): void {
  const min = Number(heatmapMinInput.value);
  const max = Number(heatmapMaxInput.value);
  viewer.setHeatmapRange("heatmap", {
    min: Math.min(min, max - 0.01),
    max: Math.max(max, min + 0.01)
  });
}

function updateLoadStatus(): void {
  if (slideLoaded && overlayLoaded && heatmapLoaded) {
    bundleLoadStatus.textContent = "Slide + cells + heatmap";
  } else if (slideLoaded && overlayLoaded) {
    bundleLoadStatus.textContent = "Slide + cells";
  } else if (slideLoaded && heatmapLoaded) {
    bundleLoadStatus.textContent = "Slide + heatmap";
  } else if (slideLoaded) {
    bundleLoadStatus.textContent = "Slide";
  } else if (overlayLoaded) {
    bundleLoadStatus.textContent = "Cells";
  } else if (heatmapLoaded) {
    bundleLoadStatus.textContent = "Heatmap";
  } else {
    bundleLoadStatus.textContent = "Synthetic";
  }
}

void main();
