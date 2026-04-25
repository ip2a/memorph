#!/usr/bin/env node
const { spawnSync } = require("child_process");
const path = require("path");

function getBinaryPath() {
  const platform = process.platform;
  const arch = process.arch;

  const mapping = {
    "darwin arm64": "memorph-darwin-arm64",
    "darwin x64": "memorph-darwin-x64",
    "linux x64": "memorph-linux-x64-gnu",
    "win32 x64": "memorph-win32-x64-msvc",
  };

  const key = `${platform} ${arch}`;
  const pkgName = mapping[key];

  if (!pkgName) {
    console.error(`Unsupported platform: ${platform} ${arch}`);
    process.exit(1);
  }

  try {
    const pkgPath = require.resolve(`${pkgName}/package.json`);
    const pkgDir = path.dirname(pkgPath);
    const binaryName = platform === "win32" ? "memorph.exe" : "memorph";
    return path.join(pkgDir, "bin", binaryName);
  } catch (e) {
    console.error(`Platform package not found: ${pkgName}`);
    console.error(`Please install the appropriate platform package or build from source.`);
    process.exit(1);
  }
}

const binaryPath = getBinaryPath();
const result = spawnSync(binaryPath, process.argv.slice(2), {
  stdio: "inherit",
  shell: false,
});

if (result.error) {
  console.error(`Failed to run memorph: ${result.error.message}`);
  process.exit(1);
}

process.exit(result.status ?? 0);
