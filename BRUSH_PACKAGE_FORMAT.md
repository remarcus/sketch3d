# Spatial Sketch brush packages

Spatial Sketch imports and exports one native brush definition per
`.brush.json` file. Version 1 is a self-contained JSON package for this app's
brush settings; it does not embed image assets, scripts, or external file
references. Built-in brush shapes and procedural texture kinds are represented
by the app's native brush model.

The top-level object contains `format: "spatial-sketch-brush"`, `version: 1`,
and a `preset` object with `name`, `brush`, `radius`, `color`, and
`stabilization`. Package names and settings are validated using the same limits
as local presets. Files larger than 64 KiB, malformed JSON, unknown format or
version, and out-of-range or non-finite values are rejected without changing
the local library.

Import creates a personal preset with a new local ID. If its name collides
case-insensitively, the imported entry gets an `(imported)` suffix, followed by
a number when needed. Import selects the entry in the library but does not
change the active drawing brush; use the normal **Use preset** action to apply
it. Export serializes the selected preset without its local ID, so the package
can be imported into another library without carrying local identity.

No compatibility with third-party 2D brush formats is claimed. They may lack
the geometry, pressure, tilt, or procedural texture data required by Spatial
Sketch's 3D brushes.
