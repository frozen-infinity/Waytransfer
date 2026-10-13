#!/usr/bin/env bash
# Starts only children owned by this invocation, in a private Wayland runtime.
set -euo pipefail
project_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
config="$project_root/examples/headless-hyprland.lua"
desktop_binary="$project_root/target/release/desktop"
parent_binary="$project_root/target/release/compositor"
listen="127.0.0.1:7878"
while (($#)); do
    case "$1" in
        --config) config="${2:?--config needs a Lua file}"; shift 2 ;;
        --desktop) desktop_binary="${2:?--desktop needs a binary path}"; shift 2 ;;
        --parent) parent_binary="${2:?--parent needs a binary path}"; shift 2 ;;
        --listen) listen="${2:?--listen needs address}"; shift 2 ;;
        --help|-h) echo 'headless-hyprland.sh [--config FILE.lua] [--desktop PATH] [--parent PATH] [--listen 127.0.0.1:7878] [-- DESKTOP_OPTIONS...]'; exit 0 ;;
        --) shift; break ;;
        *) echo "Unknown launcher option: $1" >&2; exit 2 ;;
    esac
done
command -v Hyprland >/dev/null || { echo 'Hyprland is not installed' >&2; exit 1; }
[[ -f "$config" ]] || { echo "Config not found: $config" >&2; exit 1; }
[[ -x "$desktop_binary" ]] || { echo 'Build first: cargo build --release --features desktop,gpu --bin desktop --bin compositor' >&2; exit 1; }
[[ -x "$parent_binary" ]] || { echo 'Build the GPU parent: cargo build --release --features desktop,gpu --bin desktop --bin compositor' >&2; exit 1; }
config=$(realpath -- "$config")
runtime_root="${XDG_RUNTIME_DIR:?XDG_RUNTIME_DIR must name your user runtime}"
# Hyprland adds /hypr/<40-char commit>_<timestamp>_<random>/.socket2.sock.
# Reserve 82 bytes for that suffix, keeping the full Unix socket path below 108.
if ((${#runtime_root} > 15)); then runtime_root=/tmp; fi
runtime=$(mktemp -d "$runtime_root/wXXXXXX")
chmod 700 "$runtime"
parent_pid=''
hypr_pid=''
desktop_pid=''
cleanup() {
    trap - EXIT INT TERM
    if [[ -n "$desktop_pid" ]]; then kill "$desktop_pid" 2>/dev/null || true; wait "$desktop_pid" 2>/dev/null || true; fi
    if [[ -n "$hypr_pid" ]]; then kill "$hypr_pid" 2>/dev/null || true; wait "$hypr_pid" 2>/dev/null || true; fi
    if [[ -n "$parent_pid" ]]; then kill "$parent_pid" 2>/dev/null || true; wait "$parent_pid" 2>/dev/null || true; fi
    # This is solely the private mktemp runtime created by this invocation.
    rm -rf -- "$runtime"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
mkdir -m 700 "$runtime/p" "$runtime/s"
# Smithay provides xdg-shell v6, wl_seat v9 and GPU DMA-BUF feedback.
# This parent has no physical output backend or TCP viewer listener.
env -u WAYLAND_DISPLAY -u WAYLAND_SOCKET -u DISPLAY \
    XDG_RUNTIME_DIR="$runtime/p" \
    "$parent_binary" --nested-parent --size 1280x720 --socket wayland-0 >"$runtime/p.log" 2>&1 &
parent_pid=$!
wait_socket() {
    local directory=$1 process=$2 logfile=$3 candidate
    for ((attempt=0; attempt<300; attempt++)); do
        kill -0 "$process" 2>/dev/null || { cat "$logfile" >&2; return 1; }
        for candidate in "$directory"/wayland-*; do
            if [[ -S "$candidate" ]]; then printf '%s' "$candidate"; return 0; fi
        done
        sleep 0.1
    done
    echo 'Virtual display startup timed out' >&2; cat "$logfile" >&2; return 1
}
parent_socket=$(wait_socket "$runtime/p" "$parent_pid" "$runtime/p.log")
desktop_binary=$(realpath -- "$desktop_binary")
# Only this capture binary is granted screencopy in the new Hyprland session.
cat > "$runtime/remote.lua" <<'LUA'
local capture = os.getenv("WAYTRANSFER_DESKTOP_BINARY")
local exact = "^" .. capture:gsub("([\\.^$|?*+()%[%]{}])", "\\%1") .. "$"
hl.permission(exact, "screencopy", "allow")
dofile(os.getenv("WAYTRANSFER_HYPRLAND_CONFIG"))
LUA
echo "Starting private headless Hyprland; config: $config"
# Disable the DRM/logind acquisition path. Only the explicit private Wayland
# parent is available, so this Hyprland cannot take over a physical monitor.
env -u WAYLAND_SOCKET -u DISPLAY -u HYPRLAND_INSTANCE_SIGNATURE \
    XDG_RUNTIME_DIR="$runtime/s" WAYLAND_DISPLAY="$parent_socket" \
    LIBSEAT_BACKEND=seatd SEATD_SOCK="$runtime/no-seatd-socket" \
    WAYTRANSFER_DESKTOP_BINARY="$desktop_binary" WAYTRANSFER_HYPRLAND_CONFIG="$config" \
    Hyprland --config "$runtime/remote.lua" >"$runtime/hyprland.log" 2>&1 &
hypr_pid=$!
socket=$(wait_socket "$runtime/s" "$hypr_pid" "$runtime/hyprland.log")
for ((attempt=0; attempt<300; attempt++)); do
    kill -0 "$hypr_pid" 2>/dev/null || { cat "$runtime/hyprland.log" >&2; exit 1; }
    if timeout 2s env -u WAYLAND_SOCKET XDG_RUNTIME_DIR="$runtime/s" WAYLAND_DISPLAY="$socket" \
        "$desktop_binary" --list-outputs >"$runtime/outputs.log" 2>/dev/null && [[ "$(cat "$runtime/outputs.log")" == *"WAYLAND-1:"* ]]; then break; fi
    sleep 0.1
done
[[ "$(cat "$runtime/outputs.log")" == *"WAYLAND-1:"* ]] || { echo 'Hyprland has no output' >&2; cat "$runtime/hyprland.log" >&2; exit 1; }
echo "Session WAYLAND_DISPLAY=$(basename "$socket"); runtime: $runtime"
echo 'Ctrl-C ends this private desktop. Viewer disconnects leave it running.'
env -u WAYLAND_SOCKET XDG_RUNTIME_DIR="$runtime/s" WAYLAND_DISPLAY="$socket" \
    "$desktop_binary" --listen "$listen" --output WAYLAND-1 "$@" &
desktop_pid=$!
wait "$desktop_pid"
