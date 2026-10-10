#!/usr/bin/env node

const { constants, accessSync, existsSync, realpathSync } = require("node:fs");
const { homedir } = require("node:os");
const { delimiter, isAbsolute, join, relative } = require("node:path");
const { spawnSync } = require("node:child_process");

const supported = new Map([
  ["linux-x64", ["linux-x64", "reclaim"]],
  ["win32-x64", ["win32-x64", "reclaim.exe"]],
]);

const platform = `${process.platform}-${process.arch}`;
const target = supported.get(platform);

if (process.argv.length === 3 && process.argv[2] === "update") {
  const npm = findTrustedProgram(process.platform === "win32" ? "npm.cmd" : "npm");
  if (!npm) {
    console.error("could not find npm outside the current project directory");
    process.exit(1);
  }
  const child = spawnSync(
    npm,
    [
      "install",
      "--global",
      "@alifnrhman/reclaim@latest",
      "--registry=https://registry.npmjs.org",
    ],
    {
      cwd: homedir(),
      stdio: "inherit",
      windowsHide: false,
      shell: process.platform === "win32",
    },
  );
  if (child.error) {
    console.error(`failed to start npm: ${child.error.message}`);
    process.exit(1);
  }
  process.exit(child.status ?? 1);
}

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

function findTrustedProgram(program) {
  const root = realpathSync(process.cwd());
  for (const directory of (process.env.PATH || "").split(delimiter)) {
    if (!directory || !isAbsolute(directory)) continue;
    try {
      const resolvedDirectory = realpathSync(directory);
      const directoryFromRoot = relative(root, resolvedDirectory);
      if (directoryFromRoot === "" || (!directoryFromRoot.startsWith("..") && !isAbsolute(directoryFromRoot))) {
        continue;
      }
      const candidate = join(resolvedDirectory, program);
      const resolved = realpathSync(candidate);
      const fromRoot = relative(root, resolved);
      if (fromRoot === "" || (!fromRoot.startsWith("..") && !isAbsolute(fromRoot))) {
        continue;
      }
      accessSync(resolved, process.platform === "win32" ? constants.F_OK : constants.X_OK);
      return resolved;
    } catch {
      // Missing, inaccessible, or dangling PATH entries are ignored.
    }
  }
  return null;
}

