use super::*;
use crate::presentation::{self, Preferences};
use std::time::{Duration, Instant};

fn preferences_or_default(loaded: Result<Preferences, String>) -> (Preferences, Option<String>) {
    match loaded {
        Ok(preferences) => (preferences, None),
        Err(error) => (
            Preferences::default(),
            Some(format!("Presentation preferences: {error}")),
        ),
    }
}

pub(super) struct PresentationUi {
    pub preferences: Preferences,
    pub artwork: bool,
    path: Result<PathBuf, String>,
    pub notice: Option<String>,
    pub verification_frame: Option<u32>,
    layout_save_due: Option<Instant>,
    remembered_background: [f32; 3],
    applied_theme: Option<presentation::UiTheme>,
    applied_system_theme: Option<egui::Theme>,
}
impl PresentationUi {
    pub fn new() -> Self {
        let verify = std::env::args().any(|a| a.starts_with("--verify"));
        let own_verify = std::env::args().any(|a| a == "--verify-presentation");
        let path = if verify {
            std::env::current_dir()
                .map(|p| p.join("verification/presentation/preferences.json"))
                .map_err(|e| e.to_string())
        } else {
            crate::presets::path().map(|p| p.with_file_name("presentation.json"))
        };
        let loaded = if verify && !own_verify {
            Ok(Preferences::default())
        } else {
            path.as_ref()
                .map_err(Clone::clone)
                .and_then(|p| Preferences::load(p))
        };
        let (preferences, notice) = preferences_or_default(loaded);
        let remembered_background = preferences.background;
        Self {
            preferences,
            artwork: false,
            path,
            notice,
            verification_frame: own_verify.then_some(0),
            layout_save_due: None,
            remembered_background,
            applied_theme: None,
            applied_system_theme: None,
        }
    }
    pub fn apply_theme(&mut self, ctx: &egui::Context) {
        let preference = self.preferences.ui_theme;
        let system_theme = (preference == presentation::UiTheme::System)
            .then(|| ctx.system_theme())
            .flatten();
        if self.applied_theme != Some(preference)
            || (preference == presentation::UiTheme::System
                && self.applied_system_theme != system_theme)
        {
            super::ui_style::apply(ctx, preference);
            self.applied_theme = Some(preference);
            self.applied_system_theme = system_theme;
        }
    }
    pub fn remember(&mut self) {
        self.preferences.version = 7;
        self.remembered_background = self.preferences.background;
        self.notice = Some(
            match self
                .path
                .as_ref()
                .map_err(Clone::clone)
                .and_then(|p| self.preferences.save(p))
            {
                Ok(()) => {
                    "Workspace preferences saved. Artwork-only mode always starts off.".into()
                }
                Err(e) => format!("Cannot save background preference: {e}"),
            },
        );
    }
    pub fn set_ui_theme(&mut self, theme: presentation::UiTheme) {
        if self.preferences.ui_theme == theme {
            return;
        }
        self.preferences.ui_theme = theme;
        self.preferences.version = 7;
        let mut persisted = self.preferences.clone();
        persisted.background = self.remembered_background;
        self.notice = Some(
            match self
                .path
                .as_ref()
                .map_err(Clone::clone)
                .and_then(|path| persisted.save(path))
            {
                Ok(()) => "Appearance preference saved.".into(),
                Err(error) => format!("Cannot save appearance preference: {error}"),
            },
        );
    }
    pub fn schedule_layout_save(&mut self) {
        self.layout_save_due = Some(Instant::now() + Duration::from_millis(350));
    }
    pub fn poll_layout_save(&mut self) {
        if self
            .layout_save_due
            .is_some_and(|due| Instant::now() >= due)
        {
            self.layout_save_due = None;
            self.preferences.version = 7;
            let mut persisted = self.preferences.clone();
            persisted.background = self.remembered_background;
            self.notice = Some(
                match self
                    .path
                    .as_ref()
                    .map_err(Clone::clone)
                    .and_then(|path| persisted.save(path))
                {
                    Ok(()) => "Workspace layout saved.".into(),
                    Err(error) => format!("Cannot save workspace layout: {error}"),
                },
            );
        }
    }
    pub fn reset_layout(&mut self) {
        self.preferences.layout = crate::presentation::LayoutPreferences::default();
        self.schedule_layout_save();
    }
    pub fn controls(
        &mut self,
        ui: &mut egui::Ui,
        palette: &mut super::palette_ui::PaletteUi,
    ) -> bool {
        let before = self.preferences.background;
        ui.strong("BACKGROUND");
        ui.horizontal(|ui| {
            ui.label("Canvas background");
            palette.color_button(
                ui,
                super::color_editor::ColorTarget::Background,
                &self.preferences.background,
            );
        });
        ui.horizontal(|ui| {
            for (label, color) in [
                ("Dark studio", Preferences::default().background),
                ("Blue", presentation::BLUE),
                ("Light paper", presentation::LIGHT),
            ] {
                if ui.button(label).clicked() {
                    self.preferences.background = color;
                }
            }
            if ui.button("Reset background").clicked() {
                self.preferences.background = Preferences::default().background;
            }
        });
        if self.preferences.background != before {
            self.notice = None;
        }
        if ui.button("Remember background").clicked() {
            self.remember();
        }
        ui.small("Background changes are session-only until remembered. Artwork and ground colors stay independent.");
        if let Some(notice) = &self.notice {
            ui.small(notice);
        }
        let focus = ui.button("Artwork-only view [Tab]").clicked();
        ui.small("Hide editing UI and guides. Orbit/pan/zoom, Save and Recovery remain available. Tab or Workspace returns to editing.");
        ui.separator();
        focus
    }
}

impl SketchApp {
    pub(super) fn leave_artwork(&mut self) {
        if self.presentation.artwork {
            self.presentation.artwork = false;
            self.mouse_down = false;
            self.resume_ink = false;
            self.pen_previous = None;
        }
    }
    pub(super) fn toggle_artwork(&mut self) {
        if self.presentation.artwork {
            self.leave_artwork();
            return;
        }
        if self.line.draft.is_some()
            || self.fills.draft.is_some()
            || self.placement.active
            || self.tutorial.practice.is_some()
            || self.tutorial.offer
            || self.show_help
            || self.recovery_prompt
            || self.confirm
        {
            self.status="Finish the current draft, preview, tutorial, or dialog before entering artwork-only view.".into();
            return;
        }
        self.finish();
        self.resume_ink = false;
        self.mouse_down = false;
        self.pen_previous = None;
        self.presentation.artwork = true;
    }
    pub(super) fn ensure_workspace(&mut self) {
        if self.confirm || self.recovery_prompt || self.show_help || self.tutorial.offer {
            self.leave_artwork();
        }
    }
    pub(super) fn presentation_shortcuts(&mut self, ctx: &egui::Context) {
        let (tab, escape, s, mods) = ctx.input(|i| {
            (
                i.key_pressed(Key::Tab),
                i.key_pressed(Key::Escape),
                i.key_pressed(Key::S),
                i.modifiers,
            )
        });
        if tab && !mods.command && !mods.alt {
            self.toggle_artwork();
        } else if self.presentation.artwork && escape {
            self.leave_artwork();
        } else if self.presentation.artwork && mods.command && s {
            self.save(mods.shift);
        }
    }
    pub(super) fn artwork_bar(&mut self, ctx: &egui::Context) {
        egui::Area::new(egui::Id::new("artwork-return"))
            .order(egui::Order::Foreground)
            .anchor(egui::Align2::RIGHT_TOP, egui::vec2(-12.0, 12.0))
            .show(ctx, |ui| {
                egui::Frame::window(ui.style()).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        if ui.button("Workspace [Tab]").clicked() {
                            self.leave_artwork();
                        }
                        if ui.button("Save").clicked() {
                            self.save(false);
                        }
                        if ui.button("Recovery").clicked() {
                            self.leave_artwork();
                            self.recovery_prompt = true;
                            self.recovery.request_scan();
                        }
                        if self.dirty {
                            ui.small("Unsaved changes");
                        }
                        if self.recovery.last_error.is_some() {
                            ui.colored_label(
                                super::ui_style::colors(ui.ctx()).error,
                                "Recovery issue",
                            );
                        }
                    });
                });
            });
    }
}

#[cfg(test)]
mod appearance_tests {
    use super::*;

    fn apply_with_system_theme(
        presentation: &mut PresentationUi,
        context: &egui::Context,
        system_theme: Option<egui::Theme>,
    ) {
        let _ = context.run(
            egui::RawInput {
                system_theme,
                ..Default::default()
            },
            |ctx| presentation.apply_theme(ctx),
        );
    }

    #[test]
    fn invalid_appearance_preferences_fall_back_to_dark_without_overwriting_file() {
        let directory = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
        let preference_path = directory.path().join("presentation.json");
        let invalid = br#"{"version":7,"ui_theme":"sepia","background":[0.1,0.2,0.3]}"#;
        std::fs::write(&preference_path, invalid).unwrap();

        let (preferences, notice) = preferences_or_default(Preferences::load(&preference_path));

        assert_eq!(preferences.ui_theme, presentation::UiTheme::Dark);
        assert!(notice
            .as_deref()
            .is_some_and(|notice| notice.starts_with("Presentation preferences:")));
        assert_eq!(std::fs::read(&preference_path).unwrap(), invalid);
    }

    #[test]
    fn appearance_changes_live_persists_and_tracks_system_theme_without_resetting_workspace_state()
    {
        let directory = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
        let preference_path = directory.path().join("presentation.json");
        let mut preferences = Preferences {
            background: [0.21, 0.43, 0.67],
            grain: 0.04,
            ..Default::default()
        };
        preferences.layout.inspector_width = 412.0;
        preferences.layout.inspector_tab = presentation::InspectorTab::Layers;
        let background = preferences.background;
        let mut presentation = PresentationUi {
            preferences,
            artwork: true,
            path: Ok(preference_path.clone()),
            notice: None,
            verification_frame: None,
            layout_save_due: None,
            remembered_background: background,
            applied_theme: None,
            applied_system_theme: None,
        };
        let context = egui::Context::default();

        presentation.apply_theme(&context);
        assert!(context.style().visuals.dark_mode);

        presentation.set_ui_theme(presentation::UiTheme::Light);
        presentation.apply_theme(&context);
        assert!(!context.style().visuals.dark_mode);
        assert!(
            presentation.artwork,
            "theme changes must not exit artwork mode"
        );
        assert_eq!(presentation.preferences.background, background);
        assert_eq!(presentation.preferences.grain, 0.04);
        assert_eq!(presentation.preferences.layout.inspector_width, 412.0);
        assert_eq!(
            presentation.preferences.layout.inspector_tab,
            presentation::InspectorTab::Layers
        );
        let saved = Preferences::load(&preference_path).unwrap();
        assert_eq!(saved, presentation.preferences);

        presentation.set_ui_theme(presentation::UiTheme::System);
        apply_with_system_theme(&mut presentation, &context, None);
        assert!(
            context.style().visuals.dark_mode,
            "System falls back to Dark when unavailable"
        );

        apply_with_system_theme(&mut presentation, &context, Some(egui::Theme::Light));
        assert!(
            !context.style().visuals.dark_mode,
            "System follows a reported light mode"
        );
        apply_with_system_theme(&mut presentation, &context, Some(egui::Theme::Dark));
        assert!(
            context.style().visuals.dark_mode,
            "System follows a reported dark mode"
        );

        let saved = Preferences::load(&preference_path).unwrap();
        assert_eq!(saved.ui_theme, presentation::UiTheme::System);
        assert_eq!(saved.background, background);
        assert_eq!(
            saved.layout.inspector_tab,
            presentation::InspectorTab::Layers
        );
    }
}
