# libmypaint surface evaluation

SKETCH-087 evaluates upstream libmypaint v1.6.1 without adding it to the app. The test adapter is [libmypaint_surface_eval.c](../examples/libmypaint_surface_eval.c); generated results are in [report.json](../verification/libmypaint-evaluation/report.json) and [preview.ppm](../verification/libmypaint-evaluation/preview.ppm).

## Source and isolated build

The official GitHub v1.6.1 tag archive was fetched from `https://github.com/mypaint/libmypaint/archive/refs/tags/v1.6.1.tar.gz`. Its SHA-256 is `9d888a0f13deeba06f2d63949426f4a5e4656f451b9052e4d8f7fd372a6f274e`. Source, generated build files, and installed libraries stayed under `target/libmypaint-apk-root`; the Docker Desktop WSL base filesystem was not modified. The library is ISC licensed.

The disposable build root was populated from Alpine 3.19 repositories with:

```sh
wsl.exe -d docker-desktop -- /bin/sh -lc 'apk --root /mnt/host/c/development/3dsketch/target/libmypaint-apk-root --initdb --keys-dir /etc/apk/keys --repositories-file /etc/apk/repositories add alpine-baselayout build-base autoconf automake libtool gettext-dev intltool pkgconf json-c-dev glib-dev gobject-introspection-dev python3'
```

This installs the compiler and libraries only beneath the workspace's ignored `target/` directory.

From the repository root, the archive was downloaded on the WSL host because the disposable chroot has no DNS configuration, then copied into the sysroot:

```sh
wsl.exe -d docker-desktop -- /bin/sh -lc 'wget -O /mnt/host/c/development/3dsketch/target/libmypaint-v1.6.1.tar.gz https://github.com/mypaint/libmypaint/archive/refs/tags/v1.6.1.tar.gz'
wsl.exe -d docker-desktop -- /bin/sh -lc 'sha256sum /mnt/host/c/development/3dsketch/target/libmypaint-v1.6.1.tar.gz'
wsl.exe -d docker-desktop -- /bin/sh -lc 'cp /mnt/host/c/development/3dsketch/target/libmypaint-v1.6.1.tar.gz /mnt/host/c/development/3dsketch/target/libmypaint-apk-root/tmp/libmypaint-v1.6.1.tar.gz'
wsl.exe -d docker-desktop -- /bin/sh -lc 'chroot /mnt/host/c/development/3dsketch/target/libmypaint-apk-root /bin/sh -lc "mkdir -p /tmp/sketch3d-libmypaint-src && tar -xzf /tmp/libmypaint-v1.6.1.tar.gz -C /tmp/sketch3d-libmypaint-src --strip-components=1"'
wsl.exe -d docker-desktop -- /bin/sh -lc 'cp /mnt/host/c/development/3dsketch/examples/libmypaint_surface_eval.c /mnt/host/c/development/3dsketch/target/libmypaint-apk-root/tmp/sketch3d-libmypaint-src/standalone-surface-eval.c'
```

Inside `target/libmypaint-apk-root`, extract to `/tmp/sketch3d-libmypaint-src`. The source was bootstrapped and built there with:

```sh
cd /tmp/sketch3d-libmypaint-src
./autogen.sh
./configure --prefix=/opt/sketch3d/libmypaint --disable-gegl --disable-docs --enable-introspection=yes
make -j2
make install
```

Copy `examples/libmypaint_surface_eval.c` to `/tmp/sketch3d-libmypaint-src/standalone-surface-eval.c` in the chroot, then compile and run it there:

```sh
gcc -O2 -g standalone-surface-eval.c -o /tmp/sketch3d-libmypaint-eval \
  -I/opt/sketch3d/libmypaint/include/libmypaint \
  -I/usr/include/json-c -I/usr/include/glib-2.0 -I/usr/lib/glib-2.0/include \
  -L/opt/sketch3d/libmypaint/lib -Wl,-rpath,/opt/sketch3d/libmypaint/lib \
  -lmypaint -ljson-c -lgobject-2.0 -lglib-2.0 -lintl -lm
mkdir -p /tmp/sketch3d-libmypaint-output
/tmp/sketch3d-libmypaint-eval /tmp/sketch3d-libmypaint-output
```

`make install` returned success but printed that `ldconfig` was unavailable; explicit rpath linking and the subsequent runtime execution succeeded. Copy `report.json` and `preview.ppm` from the output directory to `verification/libmypaint-evaluation/` to reproduce the checked-in evidence.

## Bounded adapter

The C harness uses a fixed 1024×1024 RGBA8 surface (4,194,304 bytes) plus one same-sized transaction/undo snapshot. Its input path is capped at 256 samples; it feeds 256 pressure-varying points with nonzero x/y tilt to the engine. Tilt mappings affect dab aspect and angle. The surface implements libmypaint dab drawing and color sampling callbacks, and tracks a dirty rectangle.

The harness runs one cancelled half-stroke, five timed strokes that are rolled back, and one committed stroke followed by undo. FNV-1a hashes assert that cancel and undo both restore the initial pixels. The preview is a 512×512 downsample of the accepted stroke before undo.

## Results

Measured in the WSL Alpine sysroot on 2026-11-02:

- Median CPU time for a 256-sample stroke: **38.332 ms** across five runs; accepted run: **38.743 ms**.
- Engine callbacks: **1,167 dabs**, **1,167 color reads**, and **21,304,044 shaded pixels** across the cancelled, timed, and accepted runs. The measured dab aspect ratio ranged from **3.5200 to 3.5733**, confirming tilt-driven variation.
- Cancel and undo restored the exact initial hash. The accepted stroke hash differed from blank.
- Application pixel storage: **8,388,608 bytes** (4 MiB image + 4 MiB snapshot), under the existing 32 MiB scratch/undo target. Process peak RSS was **10,580 KiB** in this run.
- Dirty rectangle estimate: **2,355,200 RGBA8 bytes** (`1024×575×4`); full texture upload would be **4,194,304 bytes**. These are byte-count estimates only: no GPU or upload path was present.

Results are specific to this Alpine musl WSL build and the simple reference dab renderer. The renderer is not a complete MyPaint surface implementation: it does not model all pigment, alpha-lock, eraser, or compositing modes. Native Windows, glibc Linux packaging, and macOS were not tested.

## Decision

Keep integration deferred. The prototype proves that the engine can drive a bounded raster surface with pressure, tilt, color sampling, dirty regions, cancellation, and undo. It does not produce the app's editable world-space stroke records; the current product has no committed raster layer or texture upload pipeline. Adding a native library and a separate raster authoring model is not justified by this evaluation alone.
