#!/usr/bin/env node

import { execFileSync } from "node:child_process";
import {
  chmodSync,
  copyFileSync,
  mkdtempSync,
  mkdirSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { basename, dirname, join, resolve } from "node:path";
import { createRequire } from "node:module";
import { fileURLToPath } from "node:url";

import { build } from "esbuild";

const require = createRequire(import.meta.url);
const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const outputDir = resolve(root, process.env.LLMPROBE_DIST_DIR || "dist");
const extension = process.platform === "win32" ? ".exe" : "";
const output = resolve(
  outputDir,
  process.env.LLMPROBE_SINGLE_NAME ||
    `llmprobe-${process.platform}-${process.arch}${extension}`,
);
const temporary = mkdtempSync(join(tmpdir(), "llmprobe-sea-"));
const bundle = join(temporary, "llmprobe.cjs");
const blob = join(temporary, "llmprobe.blob");
const config = join(temporary, "sea-config.json");
const chart = join(dirname(require.resolve("chart.js")), "chart.umd.js");
const postject = join(
  dirname(require.resolve("postject/package.json")),
  "dist/cli.js",
);

function run(file, args, options = {}) {
  execFileSync(file, args, { stdio: "inherit", ...options });
}

try {
  mkdirSync(outputDir, { recursive: true });
  await build({
    entryPoints: [resolve(root, "bin/llmprobe.ts")],
    outfile: bundle,
    bundle: true,
    platform: "node",
    format: "cjs",
    target: `node${process.versions.node.split(".")[0]}`,
    define: { "import.meta.url": "__filename" },
    legalComments: "none",
  });

  writeFileSync(
    config,
    `${JSON.stringify(
      {
        main: bundle,
        output: blob,
        disableExperimentalSEAWarning: true,
        useSnapshot: false,
        useCodeCache: false,
        assets: { "chart.umd.js": chart },
      },
      null,
      2,
    )}\n`,
  );

  run(process.execPath, ["--experimental-sea-config", config]);
  if (process.platform === "darwin") {
    run("lipo", [process.execPath, "-thin", process.arch, "-output", output]);
    try {
      run("codesign", ["--remove-signature", output]);
    } catch {
      // An unsigned Node binary has no signature to remove.
    }
  } else {
    copyFileSync(process.execPath, output);
  }

  const injectArgs = [
    postject,
    output,
    "NODE_SEA_BLOB",
    blob,
    "--sentinel-fuse",
    "NODE_SEA_FUSE_fce680ab2cc467b6e072b8b5df1996b2",
  ];
  if (process.platform === "darwin") {
    injectArgs.push("--macho-segment-name", "NODE_SEA");
  }
  run(process.execPath, injectArgs);

  if (process.platform !== "win32") chmodSync(output, 0o755);
  if (process.platform === "darwin") {
    run("codesign", ["--force", "--sign", "-", output]);
  }

  const version = execFileSync(output, ["--version"], {
    encoding: "utf8",
  }).trim();
  if (!/^\d+\.\d+\.\d+/.test(version)) {
    throw new Error(`single executable smoke test returned: ${version}`);
  }
  const bytes = readFileSync(output).byteLength;
  console.log(
    `built ${basename(output)} (${(bytes / 1024 / 1024).toFixed(1)} MB)`,
  );
  console.log(output);
} finally {
  rmSync(temporary, { recursive: true, force: true });
}
