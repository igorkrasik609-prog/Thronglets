#!/usr/bin/env node
"use strict";

const { execFileSync } = require("child_process");
const path = require("path");
const fs = require("fs");

const binPath = path.join(__dirname, "thronglets-bin");

if (!fs.existsSync(binPath)) {
  console.error("Thronglets binary not found. Run: npm rebuild thronglets");
  process.exit(1);
}

try {
  execFileSync(binPath, process.argv.slice(2), { stdio: "inherit" });
} catch (err) {
  process.exit(err.status || 1);
}
