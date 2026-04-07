#!/usr/bin/env node
"use strict";

const fs = require("fs");
const path = require("path");
const https = require("https");
const http = require("http");

const { version: PACKAGE_VERSION } = require("../package.json");

const VERSION = process.env.THRONGLETS_INSTALL_VERSION || PACKAGE_VERSION;
const REPO = process.env.THRONGLETS_INSTALL_REPO || "Shangri-la-0428/Thronglets";

const PLATFORMS = {
  "darwin-arm64": {
    asset: `thronglets-mcp-darwin-arm64`,
    binName: "thronglets-bin",
  },
  "linux-x64": {
    asset: `thronglets-mcp-linux-amd64`,
    binName: "thronglets-bin",
  },
  "win32-x64": {
    asset: `thronglets-mcp-windows-amd64.exe`,
    binName: "thronglets-bin.exe",
  },
};

const platform = `${process.platform}-${process.arch}`;
const target = PLATFORMS[platform];

if (!target) {
  console.error(`Unsupported platform: ${platform}`);
  console.error(`Supported: ${Object.keys(PLATFORMS).join(", ")}`);
  console.error(
    "Install from an official release binary. Source builds are for Thronglets development only."
  );
  process.exit(1);
}

const binDir = path.join(__dirname, "..", "bin");
const binPath = path.join(binDir, target.binName);
const url = `https://github.com/${REPO}/releases/download/v${VERSION}/${target.asset}`;

function download(url, dest) {
  return new Promise((resolve, reject) => {
    const follow = (url) => {
      const client = url.startsWith("https") ? https : http;
      client.get(url, (res) => {
        if (res.statusCode >= 300 && res.statusCode < 400 && res.headers.location) {
          follow(res.headers.location);
          return;
        }
        if (res.statusCode !== 200) {
          reject(new Error(`Download failed: HTTP ${res.statusCode}`));
          return;
        }
        const file = fs.createWriteStream(dest);
        res.pipe(file);
        file.on("finish", () => {
          file.close();
          resolve();
        });
      }).on("error", reject);
    };
    follow(url);
  });
}

async function main() {
  console.log(`Downloading thronglets v${VERSION} for ${platform}...`);

  fs.mkdirSync(binDir, { recursive: true });

  try {
    await download(url, binPath);
    fs.chmodSync(binPath, 0o755);

    // Ad-hoc codesign on macOS so the firewall doesn't prompt on every install
    if (process.platform === "darwin") {
      try {
        require("child_process").execFileSync("codesign", ["-s", "-", "--force", binPath], {
          stdio: "ignore",
        });
      } catch (_) {
        // Non-critical — codesign may not be available
      }
    }

    console.log("Thronglets installed successfully.");
  } catch (err) {
    console.error(`Failed to download: ${err.message}`);
    console.error(
      "Download a matching release asset or see the README for the supported prebuilt install paths."
    );
    process.exit(1);
  }
}

main();
