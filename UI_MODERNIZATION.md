# UI modernization plan

Reviewed 2026-10-31. Evidence: src/app.rs (tools and fixed 240/245-point side panels), src/layers_ui.rs, src/presets_ui.rs, src/brushes.rs, src/planes.rs, src/lighting_ui.rs, Cargo.toml, and existing verification/brush-preview.png. The screenshot is a saved verification artifact, not a fresh interactive usability test. Existing color sampling, swatches, PNG export, and line tools are implemented; this plan improves their presentation rather than rebuilding them.

## Direction and example

Use a canvas-first desktop workspace with consistent typography, spacing, focus indication, and contextual properties. Retain Rust/egui. Fluent guidance informs control choice, not a WinUI migration. WAI guidance informs keyboard behavior, not a claim that web ARIA attributes implement native accessibility.

Example layout (illustrative, not a final visual design):

```text
File   Edit   View   Help                 Storefront.sketch • Unsaved
[Brush B] [Line L] [Fill P] [Erase E] [Select V]     Undo  Redo
Brush: [Round Ink ▾]  Radius [0.025 wu]  Color [■ ▾]  Stabilize [35%]
┌──────────────────────────────────────────────┬──────────────────────┐
│                                              │ Properties | Layers  │
│                 3D CANVAS                    │ Guide                │
│                                              │ [Plane][Cylinder]    │
│                                              │ [Revolved]           │
│                                              │ Position (world)     │
│                                              │ X [0] Y [0] Z [0]    │
│                                              │ Rotation (degrees)   │
│                                              │ X [0] Y [90] Z [0]   │
└──────────────────────────────────────────────┴──────────────────────┘
Ready                 Plane: Front / XY           Pen connected
```

Examples use world units (wu), not millimeters: no physical unit conversion is implied. Selecting Select replaces brush options with selection/transform controls. Diagnostics opens from Help or the pen status. Recovery and failed saves remain clearly reachable even when diagnostics is closed.

## Selector rules and examples

| Use case | Proposed control | Example and behavior |
| --- | --- | --- |
| Small, stable, exclusive choice | Labeled segmented/radio group | Brush shape: Round / Ribbon / Chisel; exactly one active; arrow-key navigation |
| Primary drawing mode | Persistent tool buttons | Brush / Line / Fill / Erase / Select, active indicator plus label and shortcut |
| Larger text-only choice | Labeled combo box | Destination layer: Details; retain existing combo, add clear label |
| Visual preset choice | Searchable preview list/grid in popover | Round Ink and Dry Chalk each show name and stroke thumbnail; explicit Use preserves current preview/apply distinction |
| Approximate continuous value | Slider plus editable value | Stabilization: slider + 35%; keyboard entry and reset |
| Precise geometry | Numeric field with optional scrubbing/stepping | Rotation Y: 90°; type, Enter commit, Escape restore; axis labels remain visible |
| Independent option | Checkbox | Pressure controls width; retain independent state |
| Immediate on/off preference | Checkbox or switch, consistently | Show guide; do not use a two-choice dropdown |
| Color | Swatch opens color popover | HSV visual picker, RGB/HEX fields, recent/saved colors, eyedropper; display base sRGB color |
| Layer collection | Selectable rows with secondary actions | Eye, lock, name; active layer distinct from selected artwork; labeled tooltips/accessibility names |
| Commands | Buttons/menu items | Export PNG, Reset rotation; never style actions as selected modes |

## Standards consulted

- [Microsoft combo/list guidance](https://learn.microsoft.com/en-us/windows/apps/develop/ui/controls/combo-box): use compact text selectors for secondary choices, and visual lists/grids for image-rich options.
- [Microsoft radio guidance](https://learn.microsoft.com/en-us/windows/apps/develop/ui/controls/radio-button): exclusive choices form one group; independent choices use checkboxes.
- [Microsoft sliders](https://learn.microsoft.com/en-us/windows/apps/develop/ui/controls/slider): continuous adjustment suits a slider.
- [Microsoft number boxes](https://learn.microsoft.com/en-us/windows/apps/develop/ui/controls/number-box): labeled numeric entry with validation and stepping.
- [WAI toolbar pattern](https://www.w3.org/WAI/ARIA/apg/patterns/toolbar/) and [keyboard interface guidance](https://www.w3.org/WAI/ARIA/apg/practices/keyboard-interface/): predictable focus, group navigation, and keyboard access. Adapt to native egui capabilities; audit its accessibility bridge before promising screen-reader support.

## Delivery and validation

Implement SKETCH-026 through 032 in order, one reviewable feature per commit. Accessibility is an acceptance condition throughout; 032 is integration verification, not the first accessibility pass. Avoid changing geometry, document formats, or undo semantics solely for styling. Preserve in-progress edits, recovery prompts, and shortcut input guards. No docking framework, command palette, animation system, or toolkit rewrite is required for this pass.

Capture before/after views for Brush, Select, Layers, color editing, and export at 1280×720 and 1920×1080, plus 150% and 200% scaling. Verify no clipped essential controls; narrow windows may collapse inspectors. Verify keyboard-only operation, visible focus, pen-sized targets (proposed 32-point minimum desktop, 40-point comfortable mode), readable contrast (target 4.5:1 normal text and 3:1 meaningful UI boundaries), and error/disabled states. These sizes are product targets, not claims of universal standards. Dedicated hardware validation remains SKETCH-006.
