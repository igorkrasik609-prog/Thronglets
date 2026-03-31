#!/usr/bin/env node
"use strict";

const { execFileSync } = require("child_process");
const path = require("path");
const fs = require("fs");

const candidates =
  process.platform === "win32"
    ? ["thronglets-bin.exe", "thronglets-bin"]
    : ["thronglets-bin"];
const binPath = candidates
  .map((name) => path.join(__dirname, name))
  .find((candidate) => fs.existsSync(candidate));

if (!binPath) {
  console.error("Thronglets binary not found. Run: npm rebuild thronglets");
  process.exit(1);
}

function looksLikeRepoRoot(dir) {
  const cargoToml = path.join(dir, "Cargo.toml");
  const mainRs = path.join(dir, "src", "main.rs");
  if (!fs.existsSync(cargoToml) || !fs.existsSync(mainRs)) {
    return false;
  }
  try {
    return fs.readFileSync(cargoToml, "utf8").includes('name = "thronglets"');
  } catch {
    return false;
  }
}

function findRepoRoot() {
  const envRoot = process.env.THRONGLETS_REPO_ROOT;
  if (envRoot && looksLikeRepoRoot(envRoot)) {
    return envRoot;
  }

  let current = process.cwd();
  while (true) {
    if (looksLikeRepoRoot(current)) {
      return current;
    }
    const parent = path.dirname(current);
    if (parent === current) {
      return null;
    }
    current = parent;
  }
}

function tryRepoLocal(repoRoot) {
  const cargo = process.platform === "win32" ? "cargo.exe" : "cargo";
  const builtBinary =
    process.platform === "win32"
      ? path.join(repoRoot, "target", "debug", "thronglets.exe")
      : path.join(repoRoot, "target", "debug", "thronglets");

  try {
    execFileSync(cargo, ["run", "--quiet", "--manifest-path", path.join(repoRoot, "Cargo.toml"), "--", ...process.argv.slice(2)], {
      stdio: "inherit",
    });
    return true;
  } catch (err) {
    if (err.code === "ENOENT" && fs.existsSync(builtBinary)) {
      execFileSync(builtBinary, process.argv.slice(2), { stdio: "inherit" });
      return true;
    }
    if (typeof err.status === "number") {
      process.exit(err.status || 1);
    }
  }

  return false;
}

try {
  const repoRoot = findRepoRoot();
  if (repoRoot && tryRepoLocal(repoRoot)) {
    process.exit(0);
  }
  execFileSync(binPath, process.argv.slice(2), { stdio: "inherit" });
} catch (err) {
  process.exit(err.status || 1);
}
