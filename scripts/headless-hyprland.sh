#!/usr/bin/env bash
# Starts only children owned by this invocation, in a private Wayland runtime.
set -euo pipefail
project_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
config="$project_root/examples/headless-hyprland.lua"
desktop_binary="$project_root/target/release/desktop"
listen="127.0.0.1:7878"
while (($#)); do
    case "$1" in
        --config) config="${2:?--config needs a Lua file}"; shift 2 ;;
        --desktop) desktop_binary="${2:?--desktop needs a binary path}"; shift 2 ;;
        --listen) listen="${2:?--listen needs address}"; shift 2 ;;
        --help|-h) echo 'headless-hyprland.sh [--config FILE.lua] [--desktop PATH] [--listen 127.0.0.1:7878] [-- DESKTOP_OPTIONS...]'; exit 0 ;;
        --) shift; break ;;
        *) echo "Unknown launcher option: $1" >&2; exit 2 ;;
    esac
done
command -v Hyprland >/dev/null || { echo 'Hyprland is not installed' >&2; exit 1; }
[[ -f "$config" ]] || { echo "Config not found: $config" >&2; exit 1; }
[[ -x "$desktop_binary" ]] || { echo 'Build first: cargo build --release --features desktop --bin desktop' >&2; exit 1; }
config=$(realpath -- "$config")
runtime=$(mktemp -d "${XDG_RUNTIME_DIR:?XDG_RUNTIME_DIR must name your user runtime}/waytransfer-headless.XXXXXX")
chmod 700 "$runtime"
hypr_pid=''
desktop_pid=''
cleanup() {
    trap - EXIT INT TERM
    if [[ -n "$desktop_pid" ]]; then kill "$desktop_pid" 2>/dev/null || true; wait "$desktop_pid" 2>/dev/null || true; fi
    if [[ -n "$hypr_pid" ]]; then kill "$hypr_pid" 2>/dev/null || true; wait "$hypr_pid" 2>/dev/null || true; fi
    # This is solely the private mktemp runtime created by this invocation.
    rm -rf -- "$runtime"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
echo "Starting private headless Hyprland; config: $config"
env -u WAYLAND_DISPLAY -u WAYLAND_SOCKET -u DISPLAY -u HYPRLAND_INSTANCE_SIGNATURE \
    XDG_RUNTIME_DIR="$runtime" AQ_BACKEND=headless HYPRLAND_NO_SD_VARS=1 \
    Hyprland --config "$config" >"$runtime/hyprland.log" 2>&1 &
hypr_pid=$!
socket=''
for ((attempt=0; attempt<300; attempt++)); do
    kill -0 "$hypr_pid" 2>/dev/null || { cat "$runtime/hyprland.log" >&2; exit 1; }
    for candidate in "$runtime"/wayland-*; do
        if [[ -S "$candidate" ]]; then socket=$candidate; break; fi
    done
    [[ -n "$socket" ]] && break
    sleep 0.1
done
[[ -n "$socket" ]] || { echo 'Hyprland startup timed out' >&2; cat "$runtime/hyprland.log" >&2; exit 1; }
echo "Session WAYLAND_DISPLAY=$(basename "$socket"); runtime: $runtime"
echo 'Ctrl-C ends this private desktop. Viewer disconnects leave it running.'
env -u WAYLAND_SOCKET XDG_RUNTIME_DIR="$runtime" WAYLAND_DISPLAY="$socket" \
    "$desktop_binary" --listen "$listen" "$@" &
desktop_pid=$!
wait "$desktop_pid"
