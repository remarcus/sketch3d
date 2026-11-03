use super::*;
use crate::{
    document::model_references::ObjReferenceResource, export::ExportSummary, image_plane_storage,
    image_planes::ImagePlane, obj_reference_storage,
};
use glam::Quat;
use image::{ImageFormat, Rgba, RgbaImage};
use serde::Serialize;
use std::{io::Cursor, path::Path};

#[derive(Clone, Debug, Serialize)]
pub(super) struct ReferenceExportArtifact {
    pub fixture: String,
    pub glb: String,
    pub summary: ExportSummary,
    pub glb_bytes: u64,
}

impl SketchApp {
    pub(super) fn prepare_capability_reference_export(
        &mut self,
        rect: Rect,
        output_dir: &Path,
    ) -> Result<ReferenceExportArtifact, String> {
        let fixture_dir = output_dir.join("reference-export");
        if fixture_dir.exists() {
            return Err(format!(
                "Refusing to overwrite reference-export fixture folder: {}",
                fixture_dir.display()
            ));
        }
        std::fs::create_dir_all(&fixture_dir).map_err(|error| error.to_string())?;

        let drawing_path = fixture_dir.join("reference-export.sketch.json");
        let inserted_path = fixture_dir.join("inserted-lantern.sketch.json");
        let glb_path = fixture_dir.join("reference-export.glb");
        let asset_root = image_plane_storage::asset_root(&drawing_path)?;
        std::fs::create_dir_all(&asset_root).map_err(|error| error.to_string())?;

        self.doc = crate::scenes::japanese_house();
        self.next_id = self
            .doc
            .strokes
            .iter()
            .map(|stroke| stroke.id)
            .max()
            .unwrap_or(0)
            + 1;
        self.selected.clear();
        self.selected_model_reference = None;
        self.undo.clear();
        self.redo.clear();
        self.select_mode = false;
        self.mirror_ui.enabled = false;
        self.image_plane_assets.clear();
        self.obj_reference_models.clear();
        self.obj_surface_indices.clear();

        let inserted_drawing = crate::scenes::cylindrical_lantern();
        let inserted_bytes = serde_json::to_vec_pretty(&inserted_drawing)
            .map_err(|error| format!("Could not serialize insert fixture: {error}"))?;
        std::fs::write(&inserted_path, inserted_bytes)
            .map_err(|error| format!("Could not write insert fixture: {error}"))?;
        self.capability_replay
            .as_mut()
            .expect("reference-export capture is active")
            .reference_insert_path = Some(inserted_path.clone());

        let source_root = tempfile::tempdir_in(&fixture_dir).map_err(|error| error.to_string())?;
        let reference_png = source_root.path().join("garden-reference.png");
        let pixels = RgbaImage::from_fn(96, 64, |x, y| {
            let horizon = 40;
            if y < horizon {
                let sky = (y as u8).saturating_mul(2).saturating_add(96);
                Rgba([70, sky, 188, 255])
            } else {
                let edge = (x.min(95 - x) as u8).saturating_mul(2);
                Rgba([48, 112_u8.saturating_add(edge), 62, 255])
            }
        });
        let mut png_bytes = Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(pixels)
            .write_to(&mut png_bytes, ImageFormat::Png)
            .map_err(|error| error.to_string())?;
        std::fs::write(&reference_png, png_bytes.into_inner())
            .map_err(|error| format!("Could not write image reference fixture: {error}"))?;
        let image_id = self.next_id;
        let (asset_file, image_asset) =
            crate::image_planes::copy_asset(&reference_png, &asset_root, image_id)?;
        let mut image_plane = ImagePlane::from_asset(
            image_id,
            "Garden reference",
            asset_file,
            image_asset.size,
            Vec3::new(-4.7, 5.4, 2.2),
            Quat::IDENTITY,
            3.2,
        )?;
        image_plane.opacity = 0.94;
        self.doc.image_planes.push(image_plane);
        self.image_plane_assets.insert(image_id, image_asset);
        self.next_id = image_id.saturating_add(1);

        let obj_path = source_root.path().join("sapling.obj");
        let mtl_path = source_root.path().join("sapling.mtl");
        std::fs::write(
            &obj_path,
            concat!(
                "mtllib sapling.mtl\n",
                "o Sapling\n",
                "v -0.22 0 0\n",
                "v 0.22 0 0\n",
                "v 0.22 1.25 0\n",
                "v -0.22 1.25 0\n",
                "v -0.22 0 -0.22\n",
                "v 0.22 0 -0.22\n",
                "v 0.22 1.25 -0.22\n",
                "v -0.22 1.25 -0.22\n",
                "v -0.78 1.0 0.0\n",
                "v 0.0 3.2 0.0\n",
                "v 0.78 1.0 0.0\n",
                "v 0.0 1.0 0.78\n",
                "v 0.0 1.0 -0.78\n",
                "usemtl Bark\n",
                "f 1 2 3 4\n",
                "f 5 8 7 6\n",
                "f 1 5 6 2\n",
                "f 2 6 7 3\n",
                "f 3 7 8 4\n",
                "f 4 8 5 1\n",
                "usemtl Leaf\n",
                "f 9 10 11\n",
                "f 11 10 12\n",
                "f 12 10 9\n",
                "f 9 13 11\n",
                "f 11 13 12\n",
                "f 12 13 9\n"
            ),
        )
        .map_err(|error| format!("Could not write OBJ reference fixture: {error}"))?;
        std::fs::write(
            &mtl_path,
            concat!(
                "newmtl Bark\nKd 0.42 0.20 0.08\n",
                "newmtl Leaf\nKd 0.16 0.58 0.23\n"
            ),
        )
        .map_err(|error| format!("Could not write OBJ material fixture: {error}"))?;
        let injected_obj_path = fixture_dir.join("sapling-import.obj");
        std::fs::copy(&obj_path, &injected_obj_path)
            .map_err(|error| format!("Could not retain injected OBJ fixture: {error}"))?;
        std::fs::copy(&mtl_path, fixture_dir.join("sapling.mtl"))
            .map_err(|error| format!("Could not retain injected MTL fixture: {error}"))?;
        let model_id = self.next_id;
        let (model_asset, model) =
            obj_reference_storage::import_source_bundle(&obj_path, &asset_root, model_id)?;
        let extent = (model.bounds[1] - model.bounds[0]).max_element();
        if !extent.is_finite() || extent <= 1.0e-5 {
            return Err("OBJ fixture has no usable bounds".into());
        }
        let scale = 2.5 / extent;
        let model_rotation = Quat::IDENTITY;
        let model_center = Vec3::new(6.8, 4.2, 2.2)
            - model_rotation * ((model.bounds[0] + model.bounds[1]) * 0.5 * scale);
        let model_triangle_count = model.triangles.len();
        self.doc.model_references.push(ObjReferenceResource {
            id: model_id,
            name: "Sapling reference".into(),
            visible: true,
            locked: false,
            center: model_center,
            rotation: model_rotation,
            scale,
            live_line_art: None,
            asset: model_asset,
        });
        self.obj_reference_models.insert(model_id, model);
        self.next_id = model_id.saturating_add(1);
        self.image_asset_root = asset_root.clone();
        self.obj_reference_verification_source = Some(Some(injected_obj_path));
        self.refresh_image_plane_textures();
        self.refresh_obj_reference_meshes();

        self.camera = crate::scenes::house_camera();
        self.camera.target = Vec3::new(0.0, 3.0, 0.8);
        self.camera.distance = 15.5;
        self.guide = DrawingPlane::from_preset(Plane::Front);
        self.guide.depth = 0.0;
        self.obj_reference_window_open = true;
        self.status = "Garden reference and OBJ model are ready. Insert the lantern, then export the whole scene.".into();
        self.dirty = true;
        self.renderer.invalidate();

        let serialized = serde_json::to_vec_pretty(&self.doc)
            .map_err(|error| format!("Could not serialize reference-export scene: {error}"))?;
        std::fs::write(&drawing_path, serialized)
            .map_err(|error| format!("Could not save reference-export scene: {error}"))?;
        let summary =
            crate::export::write_glb_with_images(&self.doc, None, &asset_root, &glb_path)?;
        if summary.strokes != self.doc.strokes.len()
            || summary.image_planes != 1
            || summary.model_references != 1
            || model_triangle_count == 0
            || summary.triangles == 0
        {
            return Err(format!(
                "Unexpected reference-export fixture summary: {summary:?}"
            ));
        }
        let glb_bytes = std::fs::metadata(&glb_path)
            .map_err(|error| error.to_string())?
            .len();
        let _: Document = Document::from_json(
            &std::fs::read_to_string(&drawing_path).map_err(|error| error.to_string())?,
        )?;
        if crate::gizmo::project(&self.camera, rect, Vec3::new(0.0, 2.0, 0.0)).is_none() {
            return Err("Reference-export fixture does not fit the capture viewport".into());
        }
        Ok(ReferenceExportArtifact {
            fixture: "reference-export/reference-export.sketch.json".into(),
            glb: "reference-export/reference-export.glb".into(),
            summary,
            glb_bytes,
        })
    }

    pub(super) fn insert_capability_lantern(&mut self) -> Result<usize, String> {
        let path = self
            .capability_replay
            .as_ref()
            .and_then(|replay| replay.reference_insert_path.clone())
            .ok_or_else(|| "Reference-export replay has no lantern fixture path".to_owned())?;
        let next_layer_id = self
            .doc
            .layers
            .iter()
            .map(|layer| layer.id)
            .max()
            .unwrap_or(0)
            .saturating_add(1);
        let inserted = super::native_merge::prepare_insert(
            &self.doc,
            &path,
            &self.image_asset_root,
            self.next_id,
            next_layer_id,
            self.doc.next_group_resource_id(),
            Vec3::new(5.2, 0.0, 1.0),
            false,
            &self.obj_reference_models,
        )?;
        let stroke_count = super::apply_inserted_drawing_state(
            super::InsertedDrawingTarget {
                doc: &mut self.doc,
                selected: &mut self.selected,
                undo: &mut self.undo,
                redo: &mut self.redo,
                next_id: &mut self.next_id,
                dirty: &mut self.dirty,
                image_assets: &mut self.image_plane_assets,
                obj_models: &mut self.obj_reference_models,
            },
            inserted,
        );
        self.record_recovery_edit_labeled("Insert drawing");
        self.refresh_image_plane_textures();
        self.refresh_obj_reference_meshes();
        self.renderer.invalidate();
        self.status = format!("Inserted {stroke_count} lantern strokes from the native fixture.");
        Ok(stroke_count)
    }
}
