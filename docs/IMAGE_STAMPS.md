# Image stamp brushes

Leaf Scatter, Grass Scatter, and Star Stamp use small original RGBA PNG tips.
Repeated marks are placed at deterministic world-space dab distances. The
stroke stores its dab seed, spacing, pressure response, angle variation, and
the tip's source aspect ratio, so a saved stroke does not depend on the current
camera or installed brush library.

Image tips require a planar drawing target. Their orientation follows the
captured guide tangent. Camera orbit does not rotate or re-randomize existing
marks. Transparent tip pixels are omitted from click and surface picking; the
stroke bounds conservatively contain the full oriented stamp footprint.

GLB embeds each source tip PNG and assigns UVs, the saved brush tint, blended
alpha, and a double-sided material. OBJ writes the original RGB diffuse image
with an MTL `map_Kd` entry and a grayscale-alpha companion with `map_d`. OBJ
does not standardize double-sided materials, so importers may differ on stamp
back faces. Package asset limits are described in `BRUSH_PACKAGE_FORMAT.md`.

Run `cargo run --release --locked -- --verify-brushes` to exercise the built-in
stamps and generate front/orbit GPU renders at 640×360, 1280×720, and
1920×1080. The report, native fixture, GLB, and PNG sampler artifacts are
written under ignored `verification/sketch-075/`. The report records local CPU
placement, mesh, GLB, and OBJ timings over five iterations and checks geometry
and export budgets. It does not time GPU rendering or validate physical pen
input. Each tip dab currently expands to six mesh vertices; per-stroke and
document limits are 8,192 and 50,000 dabs, while GLB/OBJ enforce their shared
2,000,000-vertex per-stroke and 128 MiB file limits.
