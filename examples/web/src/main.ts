import { FoveaViewer } from "@fovea/js";
import "./styles.css";

const canvas = document.querySelector<HTMLCanvasElement>("#viewer");
const resetButton = document.querySelector<HTMLButtonElement>("#reset-camera");
const bundleForm = document.querySelector<HTMLFormElement>("#bundle-form");
const bundleInput = document.querySelector<HTMLInputElement>("#bundle-url");
const overlayForm = document.querySelector<HTMLFormElement>("#overlay-form");
const overlayInput = document.querySelector<HTMLInputElement>("#overlay-url");
const loadStatus = document.querySelector<HTMLElement>("#load-status");

if (
  !canvas ||
  !resetButton ||
  !bundleForm ||
  !bundleInput ||
  !overlayForm ||
  !overlayInput ||
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
const bundleLoadStatus = loadStatus;
let slideLoaded = false;
let overlayLoaded = false;

const values = {
  fps: document.querySelector<HTMLElement>("#fps"),
  p50: document.querySelector<HTMLElement>("#p50"),
  p95: document.querySelector<HTMLElement>("#p95"),
  p99: document.querySelector<HTMLElement>("#p99"),
  frame: document.querySelector<HTMLElement>("#frame"),
  upload: document.querySelector<HTMLElement>("#upload"),
  draws: document.querySelector<HTMLElement>("#draws"),
  visible: document.querySelector<HTMLElement>("#visible"),
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

  if (bundleParam) {
    bundleUrlInput.value = bundleParam;
  }

  if (overlayParam) {
    overlayUrlInput.value = overlayParam;
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

  bundleUrlForm.addEventListener("submit", (event) => {
    event.preventDefault();
    void loadBundleFromInput(viewer);
  });

  overlayUrlForm.addEventListener("submit", (event) => {
    event.preventDefault();
    void loadOverlayFromInput(viewer);
  });

  if (bundleUrlInput.value.trim()) {
    await loadBundleFromInput(viewer);
  }

  if (overlayUrlInput.value.trim()) {
    await loadOverlayFromInput(viewer);
  }

  viewer.start();
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
    throw error;
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
    throw error;
  }
}

function updateLoadStatus(): void {
  if (slideLoaded && overlayLoaded) {
    bundleLoadStatus.textContent = "Slide + cells";
  } else if (slideLoaded) {
    bundleLoadStatus.textContent = "Slide";
  } else if (overlayLoaded) {
    bundleLoadStatus.textContent = "Cells";
  } else {
    bundleLoadStatus.textContent = "Synthetic";
  }
}

void main();
