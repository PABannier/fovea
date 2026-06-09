import { spawnSync } from "node:child_process";
import { existsSync, mkdirSync } from "node:fs";
import { dirname, resolve } from "node:path";

const root = resolve(dirname(new URL(import.meta.url).pathname), "..");
const wasmTarget = "wasm32-unknown-unknown";
const wasmBindgenVersion = "0.2.108";

function run(command, args, options = {}) {
  const result = spawnSync(command, args, {
    cwd: root,
    stdio: "inherit",
    shell: process.platform === "win32",
    ...options
  });

  if (result.status !== 0) {
    process.exit(result.status ?? 1);
  }
}

function commandExists(command) {
  const result = spawnSync(command, ["--version"], {
    cwd: root,
    stdio: "ignore",
    shell: process.platform === "win32"
  });

  return result.status === 0;
}

function commandVersion(command) {
  const result = spawnSync(command, ["--version"], {
    cwd: root,
    encoding: "utf8",
    shell: process.platform === "win32"
  });

  if (result.status !== 0) {
    return "";
  }

  return result.stdout.trim();
}

const installedTargets = spawnSync("rustup", ["target", "list", "--installed"], {
  cwd: root,
  encoding: "utf8",
  shell: process.platform === "win32"
});

if (!installedTargets.stdout?.includes(wasmTarget)) {
  run("rustup", ["target", "add", wasmTarget]);
}

if (
  !commandExists("wasm-bindgen") ||
  !commandVersion("wasm-bindgen").includes(wasmBindgenVersion)
) {
  run("cargo", [
    "install",
    "wasm-bindgen-cli",
    "--version",
    wasmBindgenVersion,
    "--locked",
    "--force"
  ]);
}

const outDir = resolve(root, "packages/fovea-js/pkg");
mkdirSync(outDir, { recursive: true });

run("cargo", [
  "build",
  "--release",
  "--target",
  wasmTarget,
  "-p",
  "fovea-viewer"
]);

const wasmPath = resolve(root, "target/wasm32-unknown-unknown/release/fovea_viewer.wasm");

if (!existsSync(wasmPath)) {
  console.error(`Missing WASM artifact: ${wasmPath}`);
  process.exit(1);
}

run("wasm-bindgen", [
  "--target",
  "web",
  "--out-dir",
  outDir,
  "--out-name",
  "fovea_viewer",
  wasmPath
]);
