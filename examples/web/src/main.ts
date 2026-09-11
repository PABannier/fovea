import { FoveaViewer } from "@fovea/viewer";
import "./styles.css";

function $<T extends Element>(selector: string): T {
  const element = document.querySelector<T>(selector);

  if (!element) {
    throw new Error(`Fovea example DOM is missing ${selector}`);
  }

  return element;
}

const viewerCanvas = $<HTMLCanvasElement>("#viewer");
const viewerToolbar = $<HTMLElement>(".toolbar");
const viewerStatsPanel = $<HTMLElement>(".stats");
const resetCameraButton = $<HTMLButtonElement>("#reset-camera");
const slideUrlForm = $<HTMLFormElement>("#source-form");
const slideUrlInput = $<HTMLInputElement>("#slide-url");
const cellsUrlForm = $<HTMLFormElement>("#cells-form");
const cellsUrlInput = $<HTMLInputElement>("#cells-url");
const heatmapUrlForm = $<HTMLFormElement>("#heatmap-form");
const heatmapUrlInput = $<HTMLInputElement>("#heatmap-url");
const cellsVisibleInput = $<HTMLInputElement>("#cells-visible");
const cellsOpacityInput = $<HTMLInputElement>("#cells-opacity");
const cellClassesPanel = $<HTMLElement>("#cell-classes");
const heatmapVisibleInput = $<HTMLInputElement>("#heatmap-visible");
const heatmapOpacityInput = $<HTMLInputElement>("#heatmap-opacity");
const heatmapMinInput = $<HTMLInputElement>("#heatmap-min");
const heatmapMaxInput = $<HTMLInputElement>("#heatmap-max");
const heatmapColormapSelect = $<HTMLSelectElement>("#heatmap-colormap");
const loadStatusElement = $<HTMLElement>("#load-status");
let slideLoaded = false;
let cellsLoaded = false;
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
  const params = new URLSearchParams(window.location.search);
  const slideParam = params.get("slide");
  const cellsParam = params.get("cells");
  const heatmapParam = params.get("heatmap");
  const showControls = queryFlag(params, "controls", false);
  const showPerformance = queryFlag(params, "performance", false);

  viewerToolbar.hidden = !showControls;
  viewerStatsPanel.hidden = !showPerformance;

  if (slideParam) {
    slideUrlInput.value = slideParam;
  }

  if (cellsParam) {
    cellsUrlInput.value = cellsParam;
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
  cellsVisibleInput.addEventListener("change", () => {
    viewer.setLayerVisibility("cells", cellsVisibleInput.checked);
  });
  cellsOpacityInput.addEventListener("input", () => {
    viewer.setLayerOpacity("cells", Number(cellsOpacityInput.value));
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

  slideUrlForm.addEventListener("submit", (event) => {
    event.preventDefault();
    void loadSlideFromInput(viewer);
  });

  cellsUrlForm.addEventListener("submit", (event) => {
    event.preventDefault();
    void loadCellsFromInput(viewer);
  });

  heatmapUrlForm.addEventListener("submit", (event) => {
    event.preventDefault();
    void loadHeatmapFromInput(viewer);
  });

  viewer.start();

  if (slideUrlInput.value.trim()) {
    void loadSlideFromInput(viewer);
  }

  if (cellsUrlInput.value.trim()) {
    void loadCellsFromInput(viewer);
  }

  if (heatmapUrlInput.value.trim()) {
    void loadHeatmapFromInput(viewer);
  }
}

async function loadSlideFromInput(viewer: FoveaViewer): Promise<void> {
  const slideUrl = slideUrlInput.value.trim();

  if (!slideUrl) {
    return;
  }

  loadStatusElement.textContent = "Loading";

  try {
    await viewer.loadSlide(slideUrl);
    slideLoaded = true;
    updateLoadStatus();
  } catch (error) {
    slideLoaded = false;
    loadStatusElement.textContent = "Load failed";
    console.error(error);
  }
}

async function loadCellsFromInput(viewer: FoveaViewer): Promise<void> {
  const cellsUrl = cellsUrlInput.value.trim();

  if (!cellsUrl) {
    return;
  }

  loadStatusElement.textContent = "Cells";

  try {
    await viewer.loadCells(cellsUrl);
    cellsLoaded = true;
    renderCellClassFilter(viewer);
    updateLoadStatus();
  } catch (error) {
    cellsLoaded = false;
    loadStatusElement.textContent = "Cells failed";
    console.error(error);
  }
}

function renderCellClassFilter(viewer: FoveaViewer): void {
  const classes = viewer.getCellClasses();
  cellClassesPanel.replaceChildren();

  if (classes.length === 0) {
    cellClassesPanel.hidden = true;
    return;
  }

  const checkboxes: HTMLInputElement[] = [];
  // Visibility flags are indexed by class id; size to cover every class on the slide.
  const flagCount = classes.reduce((max, cellClass) => Math.max(max, cellClass.id), 0) + 1;

  const applyFilter = (): void => {
    const flags = new Uint8Array(flagCount);
    for (const checkbox of checkboxes) {
      if (checkbox.checked) {
        flags[Number(checkbox.value)] = 1;
      }
    }
    viewer.setCellClassVisibility(flags);
  };

  for (const cellClass of classes) {
    const label = document.createElement("label");
    label.className = "cell-toggle";

    const checkbox = document.createElement("input");
    checkbox.type = "checkbox";
    checkbox.checked = true;
    checkbox.value = String(cellClass.id);
    checkbox.addEventListener("change", applyFilter);

    label.append(checkbox, document.createTextNode(` ${cellClass.name}`));
    cellClassesPanel.append(label);
    checkboxes.push(checkbox);
  }

  // Rebuilt checkboxes default to all-checked; sync the viewer to match.
  applyFilter();
  cellClassesPanel.hidden = false;
}

async function loadHeatmapFromInput(viewer: FoveaViewer): Promise<void> {
  const heatmapUrl = heatmapUrlInput.value.trim();

  if (!heatmapUrl) {
    return;
  }

  loadStatusElement.textContent = "Heatmap";

  try {
    await viewer.loadHeatmap(heatmapUrl);
    heatmapLoaded = true;
    updateLoadStatus();
  } catch (error) {
    heatmapLoaded = false;
    loadStatusElement.textContent = "Heatmap failed";
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
  if (slideLoaded && cellsLoaded && heatmapLoaded) {
    loadStatusElement.textContent = "Slide + cells + heatmap";
  } else if (slideLoaded && cellsLoaded) {
    loadStatusElement.textContent = "Slide + cells";
  } else if (slideLoaded && heatmapLoaded) {
    loadStatusElement.textContent = "Slide + heatmap";
  } else if (slideLoaded) {
    loadStatusElement.textContent = "Slide";
  } else if (cellsLoaded) {
    loadStatusElement.textContent = "Cells";
  } else if (heatmapLoaded) {
    loadStatusElement.textContent = "Heatmap";
  } else {
    loadStatusElement.textContent = "Synthetic";
  }
}

function queryFlag(params: URLSearchParams, name: string, fallback: boolean): boolean {
  const value = params.get(name);

  if (value == null) {
    return fallback;
  }

  return ["1", "true", "yes", "on"].includes(value.toLowerCase());
}

void main();
