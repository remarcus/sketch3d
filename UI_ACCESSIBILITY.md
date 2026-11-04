# UI accessibility notes

The app uses eframe/egui 0.31.1. The optional `accesskit` feature is enabled so the native Windows backend can publish egui's accessibility tree to screen readers. Standard egui buttons, radios, checkboxes, sliders, and text edits expose their built-in roles and values; custom actions receive explicit names in the UI modules.

The compact tool rail exposes Brush, Line, Curve, Shape, Fill, Erase, Tint, and Select as 44×44 controls with vector icons, selected state, tooltips, and explicit AccessKit button names. The current tool is also shown as text above the icons. Shape choices remain labeled Circle and Ellipse in the contextual bar. Screen-reader speech, keyboard focus order, and theme contrast still require a manual review.

The inspector can collapse to a tab rail with Properties, Layers, References, and Images buttons, plus named Expand and Hide actions. Its selected tab and expanded width are separate preferences, so collapsing preserves both. Older layout preferences default to the expanded state.

This is an implementation note, not a claim of full assistive-technology conformance. The 3D canvas is primarily visual and does not expose individual strokes as navigable accessibility nodes. Some status and error messages are visible but may not be announced automatically. Color-picker modal behavior, dynamic layer actions, and the complete focus order still need a Windows Narrator walkthrough.

## Remaining manual checks

- With Windows Narrator, open the brush browser from the keyboard, search, move among preview cards, apply a preset, and confirm focus returns to its trigger without drawing.
- Navigate to a numeric field, edit it with Enter and Escape, and confirm canvas/tool shortcuts do not fire while editing.
- Open and cancel the color editor; check its target name, swatch names, and focus restoration. Sample artwork with the pen and confirm no stray mark is created.
- Use the layer Actions menu from the keyboard; verify active layer, selected artwork, visibility, and lock states are announced distinctly.
- Check modal dialogs and save/export/recovery failures for focus retention and actionable spoken feedback.
- Repeat key flows at 1280×720 and 150%/200% display scaling; record clipping or unusable focus targets.

No Windows screen-reader session has been run as part of this change. The native bridge and egui widget metadata are enabled, but runtime Narrator behavior remains unverified.
