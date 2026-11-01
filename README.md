# Sketch3D

Sketch3D is an open-source, tablet-first 3D sketching app written in Rust. Draw editable strokes in space, work on flat or curved guides, and export artwork for other tools.

## Features

- Pressure-sensitive 3D brush strokes with stabilization, editable brushes, and undo/redo.
- Freely placed drawing planes and curved guides, including cylinders, revolved surfaces, and saved custom guides.
- Selection, transforms, groups, layers, fills, shapes, curves, mirroring, and partial erasing.
- Image planes and OBJ references for drawing over references or onto model surfaces.
- Automatic recovery and a local drawing library for organizing native drawings.
- Artwork image, OBJ, and GLB exports, plus animated GIF and MP4 exports.

Windows is the first supported tablet platform and uses Windows Ink. macOS and Linux currently support mouse input; native pen input and physical-device behavior have not been validated there.

## Run

Install stable Rust from [rustup.rs](https://rustup.rs). Windows builds also require Visual Studio Build Tools with the Desktop development with C++ workload and a Windows SDK.

```sh
cargo run --release
```

The app opens with an editable Japanese-style garden house. Choose **New** for a blank drawing or run `cargo run --release -- --blank` to start blank. Run with `--demo` to open the original three-ring geometry example.

On Debian/Ubuntu, install the development packages used by winit, Wayland/X11, and the file-dialog backend: `build-essential pkg-config libxkbcommon-dev libwayland-dev libx11-dev libxi-dev libxcursor-dev libxrandr-dev libxinerama-dev libvulkan-dev libdbus-1-dev`. On macOS, install Xcode Command Line Tools.

## Tablet setup

Install your tablet manufacturer's driver and enable **Windows Ink** in its application profile. Map the tablet to the intended monitor and preserve the correct aspect ratio. Sketch3D receives driver-mapped coordinates and does not provide its own tablet calibration.

Draw in the central canvas. The Input lab reports pen samples, pressure, and tilt. If it shows **WAITING FOR PEN**, input is arriving as mouse input and pressure is unavailable. Unsupported tilt is shown as not reported. Mouse drawing uses constant pressure.

## Essential controls

| Action | Input |
| --- | --- |
| Brush / erase / select | B / E / V |
| Move / rotate / scale selection | G / R / S |
| Add or remove a stroke from selection | Shift-click |
| Duplicate / delete | Ctrl/Cmd+D / Delete |
| Orbit / pan / zoom | Space+drag / Space+Shift+drag / Space+Alt+drag; right / middle mouse drag and wheel also work |
| Align view to guide / create plane from view | F / Shift+F |
| Undo / redo | Ctrl/Cmd+Z / Ctrl/Cmd+Shift+Z |
| Save / save as | Ctrl/Cmd+S / Ctrl/Cmd+Shift+S |
| Open / new drawing | Ctrl/Cmd+O / Ctrl/Cmd+N |

The **Help** menu and in-app tutorial describe the drawing tools and workflows. Essential controls do not require touch. An edge-on drawing plane cannot accept strokes; press F to align the view or orbit away from the edge.

## Drawings and exports

Native drawings use the editable `.sketch.json` format. Image and model references are copied into managed asset folders alongside the drawing. Sketch3D also supports PNG/JPEG artwork images, OBJ meshes, GLB scenes, and animated GIF/MP4. MP4 export requires FFmpeg with libx264 on `PATH`.

Recovery snapshots protect completed edits separately from deliberate drawing saves. Use **Save** to create or update your native drawing file. Camera bookmarks, view sequences, and local brush settings are stored separately from drawings.

## Releases

After setting the version in `Cargo.toml`, push the matching tag (for example, `v0.1.0`) to build Windows, Linux, and macOS archives and publish them with generated release notes and SHA-256 checksums. The workflow marks tags containing a hyphen, such as `v0.2.0-rc.1`, as prereleases.

## Project information

- [Architecture notes](ARCHITECTURE.md)
- [Contribution guide](CONTRIBUTING.md)
- [License](LICENSE) — GPL-3.0-or-later
