#!/usr/bin/env node

const { existsSync } = require("node:fs");
const { join } = require("node:path");
const { spawnSync } = require("node:child_process");

const supported = new Map([
  ["linux-x64", ["linux-x64", "reclaim"]],
  ["win32-x64", ["win32-x64", "reclaim.exe"]],
]);

const platform = `${process.platform}-${process.arch}`;
const target = supported.get(platform);

if (!target && !process.env.RECLAIM_BINARY) {
  console.error(
    `reclaim does not yet provide a binary for ${platform}. Supported platforms: Linux x64 and Windows x64.`,
  );
  process.exit(1);
}

const binary =
  process.env.RECLAIM_BINARY || join(__dirname, "..", "vendor", ...target);

if (!existsSync(binary)) {
  console.error(`reclaim native binary is missing: ${binary}`);
  console.error("Reinstall @alifnrhman/reclaim. If the problem continues, report the package version and platform.");
  process.exit(1);
}

const child = spawnSync(binary, process.argv.slice(2), {
  stdio: "inherit",
  windowsHide: false,
});

if (child.error) {
  console.error(`failed to start reclaim: ${child.error.message}`);
  process.exit(1);
}

process.exit(child.status ?? 1);

