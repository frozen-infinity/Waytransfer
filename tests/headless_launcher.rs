#![cfg(all(target_os = "linux", feature = "desktop"))]
use std::{fs, os::unix::fs::PermissionsExt, process::Command};
fn run(fail: bool, long_runtime: bool) {
    let root = tempfile::tempdir().unwrap();
    let bin = root.path().join("bin");
    fs::create_dir(&bin).unwrap();
    let stub = r#"#!/usr/bin/env python3
import os, sys, socket, time, json
from pathlib import Path
name=Path(sys.argv[0]).name
root=Path(os.environ['WAYTRANSFER_LAUNCHER_TEST'])
if name=='desktop' and '--list-outputs' in sys.argv:
    print('WAYLAND-1: 1280x720');sys.exit(0)
keys=['XDG_RUNTIME_DIR','WAYLAND_DISPLAY','WAYLAND_SOCKET','DISPLAY','WLR_BACKENDS','WLR_RENDERER','LIBSEAT_BACKEND','SEATD_SOCK','WAYTRANSFER_HYPRLAND_CONFIG']
(root/(name+'.runtime')).write_text(os.environ['XDG_RUNTIME_DIR'])
(root/(name+'.json')).write_text(json.dumps({'pid':os.getpid(),'env':{k:os.environ.get(k) for k in keys},'args':sys.argv[1:]}))
if name=='desktop': sys.exit(0)
if name=='Hyprland' and os.environ.get('WAYTRANSFER_FAIL_HYPR')=='1': sys.exit(17)
if name=='Hyprland':
    runtime=Path(os.environ['XDG_RUNTIME_DIR'])
    assert len(str(runtime/'hypr'/('f'*40+'_1791915601_2147483647')/'.socket2.sock').encode()) < 108
    parent=socket.socket(socket.AF_UNIX);parent.connect(os.environ['WAYLAND_DISPLAY'])
s=socket.socket(socket.AF_UNIX);s.bind(str(Path(os.environ['XDG_RUNTIME_DIR'])/'wayland-1'));s.listen(2)
while True: time.sleep(0.05)
"#;
    for name in ["compositor", "Hyprland", "desktop"] {
        let path = bin.join(name);
        fs::write(&path, stub).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }
    let runtime = if long_runtime {
        root.path().join("deliberately-long-runtime")
    } else {
        root.path().to_path_buf()
    };
    fs::create_dir_all(&runtime).unwrap();
    let result = Command::new("bash")
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/scripts/headless-hyprland.sh"
        ))
        .args([
            "--desktop",
            bin.join("desktop").to_str().unwrap(),
            "--parent",
            bin.join("compositor").to_str().unwrap(),
        ])
        .env(
            "PATH",
            format!("{}:{}", bin.display(), std::env::var("PATH").unwrap()),
        )
        .env("XDG_RUNTIME_DIR", &runtime)
        .env("WAYTRANSFER_LAUNCHER_TEST", root.path())
        .env("WAYLAND_DISPLAY", "physical-desktop")
        .env("WAYLAND_SOCKET", "42")
        .env("DISPLAY", ":77")
        .env("WAYTRANSFER_FAIL_HYPR", if fail { "1" } else { "0" })
        .output()
        .unwrap();
    assert_eq!(
        result.status.success(),
        !fail,
        "stdout={} stderr={}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    let parent_runtime = fs::read_to_string(root.path().join("compositor.runtime")).unwrap();
    assert!(
        !std::path::Path::new(&parent_runtime)
            .parent()
            .unwrap()
            .exists(),
        "private runtime leaked"
    );
    let parent = fs::read_to_string(root.path().join("compositor.json")).unwrap();
    assert!(parent.contains("--nested-parent"));
    assert!(parent.contains("--size"));
    assert!(!parent.contains("physical-desktop"));
    let child = fs::read_to_string(root.path().join("Hyprland.json")).unwrap();
    assert!(child.contains("/p/wayland-1"));
    assert!(child.contains("/s"));
    assert!(child.contains("seatd"));
    assert!(!child.contains("physical-desktop"));
    if !fail {
        let desktop = fs::read_to_string(root.path().join("desktop.json")).unwrap();
        assert!(desktop.contains("/s/wayland-1"));
        assert!(desktop.contains("WAYLAND-1"));
    }
    assert!(
        !fs::read_dir(root.path())
            .unwrap()
            .flatten()
            .any(|entry| entry.file_name().to_string_lossy().starts_with("w")),
        "private runtime leaked after launcher exit"
    );
}
#[test]
fn launcher_uses_private_virtual_displays_and_cleans_up() {
    run(false, false);
}
#[test]
fn failed_hyprland_start_cleans_up_parent() {
    run(true, false);
}

#[test]
fn long_runtime_uses_short_socket_paths() {
    run(false, true);
}
