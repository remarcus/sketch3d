use crate::drawing_library::TrashDrawing;
#[cfg(test)]
use eframe::egui::Rect;
use eframe::egui::{self, Key};
use std::path::PathBuf;

pub(super) struct TrashWindowOutput {
    pub open: bool,
    pub restore: Option<(PathBuf, String)>,
    #[cfg(test)]
    restore_rects: Vec<(PathBuf, Rect)>,
    #[cfg(test)]
    rendered_notice: Option<String>,
}

pub(super) fn show(
    ctx: &egui::Context,
    mut open: bool,
    entries: &[TrashDrawing],
    read_error: Option<&str>,
) -> TrashWindowOutput {
    if ctx.input(|input| input.key_pressed(Key::Escape)) {
        open = false;
    }
    let mut restore = None;
    #[cfg(test)]
    let mut rendered_notice = None;
    #[cfg(test)]
    let mut restore_rects = Vec::new();
    egui::Window::new("Recoverable drawing trash")
        .open(&mut open)
        .default_width(420.0)
        .show(ctx, |ui| {
            ui.label("Deleted drawings remain here until restored. Restoring never overwrites an existing file.");
            if let Some(error) = read_error {
                ui.colored_label(super::ui_style::colors(ui.ctx()).error, error);
                #[cfg(test)]
                {
                    rendered_notice = Some(error.to_owned());
                }
            }
            if entries.is_empty() && read_error.is_none() {
                ui.weak("Trash is empty.");
            }
            egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
                for entry in entries {
                    ui.horizontal(|ui| {
                        ui.vertical(|ui| {
                            ui.label(&entry.name);
                            if let Some(deleted) = entry.deleted {
                                let age = std::time::SystemTime::now()
                                    .duration_since(deleted)
                                    .unwrap_or_default();
                                ui.small(format!("Deleted {} days ago", age.as_secs() / 86_400));
                            }
                            if let Some(issue) = &entry.issue {
                                ui.colored_label(super::ui_style::colors(ui.ctx()).error, issue);
                            }
                        });
                        let button = ui.add_enabled(
                            entry.issue.is_none(),
                            egui::Button::new("Restore…"),
                        );
                        #[cfg(test)]
                        restore_rects.push((entry.path.clone(), button.rect));
                        if button.clicked() {
                            restore = Some((entry.path.clone(), entry.name.clone()));
                        }
                    });
                    ui.separator();
                }
            });
        });
    TrashWindowOutput {
        open,
        restore,
        #[cfg(test)]
        restore_rects,
        #[cfg(test)]
        rendered_notice,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::{Event, PointerButton, Pos2, Vec2};

    fn entries() -> Vec<TrashDrawing> {
        vec![
            TrashDrawing {
                path: PathBuf::from("good.sketch-backup"),
                name: "garden.sketch.json".into(),
                deleted: Some(std::time::SystemTime::now()),
                issue: None,
            },
            TrashDrawing {
                path: PathBuf::from("bad.sketch-backup"),
                name: "Unreadable backup".into(),
                deleted: None,
                issue: Some("Backup manifest is corrupt".into()),
            },
        ]
    }

    fn frame(
        ctx: &egui::Context,
        events: Vec<Event>,
        entries: &[TrashDrawing],
    ) -> TrashWindowOutput {
        let mut output = None;
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0))),
                focused: true,
                events,
                ..Default::default()
            },
            |ctx| output = Some(show(ctx, true, entries, None)),
        );
        output.expect("trash window renders")
    }

    fn click(ctx: &egui::Context, position: Pos2, entries: &[TrashDrawing]) -> TrashWindowOutput {
        let _ = frame(ctx, vec![Event::PointerMoved(position)], entries);
        let _ = frame(
            ctx,
            vec![Event::PointerButton {
                pos: position,
                button: PointerButton::Primary,
                pressed: true,
                modifiers: Default::default(),
            }],
            entries,
        );
        frame(
            ctx,
            vec![Event::PointerButton {
                pos: position,
                button: PointerButton::Primary,
                pressed: false,
                modifiers: Default::default(),
            }],
            entries,
        )
    }

    #[test]
    fn trash_restore_action_is_clickable_and_corrupt_backups_are_disabled_in_both_themes() {
        let entries = entries();
        for visuals in [egui::Visuals::dark(), egui::Visuals::light()] {
            let ctx = egui::Context::default();
            ctx.set_visuals(visuals);
            let initial = frame(&ctx, Vec::new(), &entries);
            let good_rect = initial
                .restore_rects
                .iter()
                .find(|(path, _)| path.as_path() == std::path::Path::new("good.sketch-backup"))
                .expect("valid restore button is laid out")
                .1;
            let restored = click(&ctx, good_rect.center(), &entries);
            assert_eq!(
                restored.restore,
                Some((
                    PathBuf::from("good.sketch-backup"),
                    "garden.sketch.json".into()
                ))
            );

            let ctx = egui::Context::default();
            let initial = frame(&ctx, Vec::new(), &entries);
            let corrupt_rect = initial
                .restore_rects
                .iter()
                .find(|(path, _)| path.as_path() == std::path::Path::new("bad.sketch-backup"))
                .expect("disabled corrupt entry still has a button region")
                .1;
            let rejected = click(&ctx, corrupt_rect.center(), &entries);
            assert!(rejected.restore.is_none());
        }
    }

    #[test]
    fn escape_closes_the_trash_window_without_selecting_a_restore() {
        let ctx = egui::Context::default();
        let mut output = None;
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0))),
                focused: true,
                events: vec![Event::Key {
                    key: Key::Escape,
                    physical_key: Some(Key::Escape),
                    pressed: true,
                    repeat: false,
                    modifiers: Default::default(),
                }],
                ..Default::default()
            },
            |ctx| output = Some(show(ctx, true, &[], None)),
        );
        let output = output.expect("trash window renders");
        assert!(!output.open);
        assert!(output.restore.is_none());
    }

    #[test]
    fn trash_read_errors_remain_visible_in_light_and_dark_themes() {
        let notice = "Restore failed: destination already exists";
        for visuals in [egui::Visuals::dark(), egui::Visuals::light()] {
            let ctx = egui::Context::default();
            ctx.set_visuals(visuals);
            let mut output = None;
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0))),
                    ..Default::default()
                },
                |ctx| output = Some(show(ctx, true, &[], Some(notice))),
            );
            let output = output.expect("trash restore error renders");
            assert!(output.open);
            assert_eq!(output.rendered_notice.as_deref(), Some(notice));
        }
    }
}
