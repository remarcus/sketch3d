# Application icons

Generated from the repository's `sketch3d_icon_2048_transparent.png`.
The original opaque and transparent artwork remains at the repository root.

- `sketch3d.png`: 256px RGBA window icon, embedded in the application.
- `sketch3d.ico`: 16–256px Windows executable resource, embedded by `build.rs`.
- `sketch3d.icns`: macOS application bundle icon used by the release workflow.

Regenerate with `python scripts/generate_icons.py` after installing Pillow.
Normal Cargo builds use the checked-in generated files and do not need Python.
Linux archives include the PNG; installing desktop launchers is a separate step.
macOS archives include a Sketch3D.app bundle and the standalone binary. The
bundle is not code signed or notarized by this workflow.
