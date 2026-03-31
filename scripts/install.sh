#!/usr/bin/env sh
set -eu

REPO="${THRONGLETS_INSTALL_REPO:-Shangri-la-0428/Thronglets}"
VERSION="${THRONGLETS_VERSION:-latest}"
INSTALL_DIR="${THRONGLETS_INSTALL_DIR:-$HOME/.local/bin}"
BIN_NAME="thronglets"
BIN_TARGET="thronglets-bin"

platform() {
  os="$(uname -s)"
  arch="$(uname -m)"

  case "${os}-${arch}" in
    Darwin-arm64)
      echo "thronglets-mcp-darwin-arm64"
      ;;
    Linux-x86_64)
      echo "thronglets-mcp-linux-amd64"
      ;;
    *)
      echo "Unsupported platform: ${os}-${arch}" >&2
      echo "Supported by this shell installer: Darwin-arm64, Linux-x86_64" >&2
      echo "For Windows, use scripts/install.ps1 or npm install -g thronglets." >&2
      exit 1
      ;;
  esac
}

download_url() {
  asset="$1"
  if [ "$VERSION" = "latest" ]; then
    echo "https://github.com/${REPO}/releases/latest/download/${asset}"
  else
    echo "https://github.com/${REPO}/releases/download/v${VERSION}/${asset}"
  fi
}

main() {
  asset="$(platform)"
  url="$(download_url "$asset")"
  tmpdir="$(mktemp -d)"
  trap 'rm -rf "$tmpdir"' EXIT INT TERM

  mkdir -p "$INSTALL_DIR"

  if command -v curl >/dev/null 2>&1; then
    curl -fsSL "$url" -o "$tmpdir/$BIN_TARGET"
  elif command -v wget >/dev/null 2>&1; then
    wget -qO "$tmpdir/$BIN_TARGET" "$url"
  else
    echo "Install requires curl or wget." >&2
    exit 1
  fi

  cat >"$tmpdir/$BIN_NAME" <<'EOF'
#!/usr/bin/env sh
set -eu

INSTALLED_BIN_DIR="__INSTALL_DIR__"
INSTALLED_BIN="$INSTALLED_BIN_DIR/thronglets-bin"

looks_like_repo_root() {
  root="$1"
  [ -f "$root/Cargo.toml" ] || return 1
  [ -f "$root/src/main.rs" ] || return 1
  grep -q 'name = "thronglets"' "$root/Cargo.toml" 2>/dev/null
}

find_repo_root() {
  if [ -n "${THRONGLETS_REPO_ROOT:-}" ] && looks_like_repo_root "$THRONGLETS_REPO_ROOT"; then
    printf '%s\n' "$THRONGLETS_REPO_ROOT"
    return 0
  fi

  dir="${PWD:-$(pwd)}"
  while [ -n "$dir" ] && [ "$dir" != "/" ]; do
    if looks_like_repo_root "$dir"; then
      printf '%s\n' "$dir"
      return 0
    fi
    dir="$(dirname "$dir")"
  done
  return 1
}

if repo_root="$(find_repo_root 2>/dev/null)"; then
  if command -v cargo >/dev/null 2>&1; then
    exec cargo run --quiet --manifest-path "$repo_root/Cargo.toml" -- "$@"
  fi
  if [ -x "$repo_root/target/debug/thronglets" ]; then
    exec "$repo_root/target/debug/thronglets" "$@"
  fi
fi

exec "$INSTALLED_BIN" "$@"
EOF
  sed -i.bak "s#__INSTALL_DIR__#$INSTALL_DIR#g" "$tmpdir/$BIN_NAME"
  rm -f "$tmpdir/$BIN_NAME.bak"

  chmod +x "$tmpdir/$BIN_TARGET" "$tmpdir/$BIN_NAME"
  mv "$tmpdir/$BIN_TARGET" "$INSTALL_DIR/$BIN_TARGET"
  mv "$tmpdir/$BIN_NAME" "$INSTALL_DIR/$BIN_NAME"

  echo "Installed $BIN_NAME to $INSTALL_DIR/$BIN_NAME"
  echo "Next: thronglets start"
}

main "$@"
