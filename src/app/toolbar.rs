use super::Editor;
use super::before_after::{Compare, Transfer};
use super::dialogs::FileDialog;
use super::icons::Icon;
use super::widgets::{
    ButtonKind, action_button, menu_item, menu_separator, toolbar_action, toolbar_divider,
};
use crate::app::theme;
use crate::model::recipe::Recipe;
use eframe::egui::{self, Stroke, Vec2};

impl Editor {
    pub(super) fn toolbar(&mut self, ui: &mut egui::Ui) {
        let palette = theme::palette(ui.ctx());
        let ctx = ui.ctx().clone();
        // The buttons follow the shared log, which also holds Library changes.
        self.sync_undo();
        let (can_undo, can_redo) = (self.undo_log.can_undo(), self.undo_log.can_redo());
        egui::Panel::top("toolbar")
            .frame(
                egui::Frame::new()
                    .fill(palette.gray(29))
                    .inner_margin(egui::Margin::symmetric(14, 10)),
            )
            .show(ui, |ui| {
                ui.spacing_mut().button_padding = Vec2::new(12., 8.);
                ui.spacing_mut().interact_size.y = 32.;
                ui.spacing_mut().item_spacing.x = 8.;
                ui.visuals_mut().button_frame = true;
                ui.visuals_mut().widgets.inactive.bg_fill = palette.gray(38);
                ui.visuals_mut().widgets.inactive.weak_bg_fill = palette.gray(38);
                ui.visuals_mut().widgets.inactive.bg_stroke = Stroke::new(1., palette.gray(53));
                ui.visuals_mut().widgets.hovered.bg_fill = palette.gray(52);
                ui.visuals_mut().widgets.hovered.weak_bg_fill = palette.gray(52);
                ui.horizontal(|ui| {
                    if toolbar_action(ui, "", 32., false, can_undo, 1)
                        .on_hover_text("撤销 · Ctrl+Z")
                        .clicked()
                    {
                        self.undo();
                    }
                    if toolbar_action(ui, "", 32., false, can_redo, 2)
                        .on_hover_text("重做 · Ctrl+Shift+Z")
                        .clicked()
                    {
                        self.redo();
                    }
                    toolbar_divider(ui);
                    let before_only = self.view.compare.before_only();
                    if toolbar_action(ui, "编辑前", 70., before_only, true, 3)
                        .on_hover_text("显示编辑前 · 反斜杠")
                        .clicked()
                    {
                        let view = self.view.compare.toggled(Compare::BeforeOnly);
                        self.set_compare(view);
                    }
                    self.before_after_menu(ui);
                    self.reference_controls(ui);
                    if toolbar_action(ui, "剪裁警告", 78., self.view.clipping.both_on(), true, 0)
                        .on_hover_text("显示阴影与高光剪裁 · J")
                        .clicked()
                    {
                        self.view.clipping.toggle_both();
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if action_button(
                            ui,
                            "导出",
                            Some(Icon::Export),
                            ButtonKind::Primary,
                            self.document.full().is_some(),
                        )
                        .on_hover_text(if cfg!(target_os = "macos") {
                            "导出… · ⇧⌘E"
                        } else {
                            "导出… · Ctrl+Shift+E"
                        })
                        .clicked()
                        {
                            self.open_export_dialog();
                        }
                        ui.add_space(8.);
                        let settings = action_button(ui, "设置", None, ButtonKind::Secondary, true);
                        egui::Popup::menu(&settings).show(|ui| {
                            ui.set_width(270.);
                            ui.spacing_mut().item_spacing.y = 0.;
                            let (cmd, shift) = if cfg!(target_os = "macos") {
                                ("⌘ ", "Shift ")
                            } else {
                                ("Ctrl+", "Shift+")
                            };
                            let copy = format!("{cmd}{shift}C");
                            let paste = format!("{cmd}{shift}V");
                            if menu_item(ui, "复制设置…", &copy, true, false) {
                                self.open_copy_dialog(super::settings_transfer::Transfer::Copy);
                                ui.close();
                            }
                            if menu_item(ui, "粘贴设置", &paste, self.clipboard.is_some(), false)
                            {
                                self.paste_settings();
                                ui.close();
                            }
                            let previous = format!(
                                "{cmd}{}V",
                                if cfg!(target_os = "macos") {
                                    "⌥ "
                                } else {
                                    "Alt+"
                                }
                            );
                            if menu_item(
                                ui,
                                "粘贴上一张设置",
                                &previous,
                                self.previous_settings.is_some(),
                                false,
                            ) {
                                self.paste_previous();
                                ui.close();
                            }
                            let sync = format!("{cmd}{shift}S");
                            let targets = self.sync_targets().len();
                            if menu_item(
                                ui,
                                "同步设置…",
                                &sync,
                                targets > 0 && !self.activity.is_busy(),
                                false,
                            ) {
                                self.open_copy_dialog(super::settings_transfer::Transfer::Sync);
                                ui.close();
                            }
                            // Lightroom's Match Total Exposures, for the photos selected
                            // with this one, from their aperture, shutter speed and ISO.
                            let matchable = targets > 0
                                && !self.activity.is_busy()
                                && self
                                    .document
                                    .metadata
                                    .as_ref()
                                    .and_then(super::sync::capture_stops)
                                    .is_some();
                            let match_keys = format!(
                                "{cmd}{shift}{}M",
                                if cfg!(target_os = "macos") {
                                    "⌥ "
                                } else {
                                    "Alt+"
                                }
                            );
                            if menu_item(ui, "匹配总曝光", &match_keys, matchable, false) {
                                self.start_sync(super::sync::BatchChange::MatchTotalExposures);
                                ui.close();
                            }
                            let reset = format!("{cmd}{shift}R");
                            if menu_item(ui, "重置所有设置", &reset, true, false) {
                                self.reset_settings();
                                ui.close();
                            }
                            menu_separator(ui);
                            let new_preset = format!("{cmd}{shift}N");
                            if menu_item(ui, "新建显影预设…", &new_preset, true, false) {
                                self.open_copy_dialog(
                                    super::settings_transfer::Transfer::NewPreset,
                                );
                                ui.close();
                            }
                            if menu_item(ui, "保存预设文件…", "", true, false) {
                                self.dialog(FileDialog::SavePreset, &ctx);
                                ui.close();
                            }
                            if menu_item(ui, "加载预设文件…", "", true, false) {
                                self.dialog(FileDialog::LoadPreset, &ctx);
                                ui.close();
                            }
                            menu_separator(ui);
                            let (export, previous) = if cfg!(target_os = "macos") {
                                ("⇧⌘E", "⌥⇧⌘E")
                            } else {
                                ("Ctrl+Shift+E", "Ctrl+Alt+Shift+E")
                            };
                            let photo = self.document.full().is_some();
                            if menu_item(ui, "导出…", export, photo, false) {
                                self.open_export_dialog();
                                ui.close();
                            }
                            if menu_item(ui, "使用上次设置导出", previous, photo, false) {
                                self.export_with_previous();
                                ui.close();
                            }
                            menu_separator(ui);
                            let prefs = if cfg!(target_os = "macos") {
                                "⌘ ,"
                            } else {
                                "Ctrl+,"
                            };
                            // Profiles, display and engine choices apply to every photo.
                            if menu_item(ui, "偏好设置…", prefs, true, false) {
                                self.open_preferences(super::preferences::Tab::General);
                                ui.close();
                            }
                        });
                        if self.load.is_running() || self.preview.task.is_running() {
                            ui.spinner();
                        }
                    });
                });
            });
    }
    /// Reference View's button, and while it shows, its layout and lock, as in
    /// Lightroom's toolbar.
    fn reference_controls(&mut self, ui: &mut egui::Ui) {
        use super::before_after::Axis;
        let shown = self.reference_view();
        if toolbar_action(ui, "参考", 84., shown, true, 0)
            .on_hover_text("参考视图 · Shift+R")
            .clicked()
        {
            self.toggle_reference_view();
        }
        if !shown {
            return;
        }
        for (layout, label, width) in [
            (Axis::LeftRight, "左右", 92.),
            (Axis::TopBottom, "上下", 98.),
        ] {
            let chosen = self.view.compare == Compare::Reference(layout);
            if toolbar_action(ui, label, width, chosen, true, 0).clicked() {
                self.set_reference_layout(layout);
            }
        }
        let locked = self.reference.locked;
        if toolbar_action(ui, "锁定", 54., locked, self.reference.photo.is_some(), 0)
            .on_hover_text(if locked {
                "离开显影时仍保留参考照片"
            } else {
                "离开显影时保留参考照片"
            })
            .clicked()
        {
            self.reference.locked = !locked;
        }
    }
    /// Lightroom's Before/After menu: the views, with their keys, and copying or
    /// swapping settings between the two sides.
    fn before_after_menu(&mut self, ui: &mut egui::Ui) {
        let two_up = self.view.compare.two_up() && self.view.compare.shows_before();
        let button = toolbar_action(ui, "前后对比", 104., two_up, true, 0)
            .on_hover_text("前后对比视图 · Y、Option+Y、Shift+Y");
        let mac = cfg!(target_os = "macos");
        let alt = if mac { "⌥ " } else { "Alt+" };
        let transfer = if mac { "⌥⇧⌘ " } else { "Ctrl+Alt+Shift+" };
        egui::Popup::menu(&button).show(|ui| {
            ui.set_width(300.);
            ui.spacing_mut().item_spacing.y = 0.;
            for (view, name) in super::before_after::VIEWS {
                let keys = match view {
                    Compare::BeforeOnly => "\\".to_string(),
                    Compare::SideBySide(super::before_after::Axis::LeftRight) => "Y".into(),
                    Compare::SideBySide(_) => format!("{alt}Y"),
                    Compare::Split(super::before_after::Axis::LeftRight) => "Shift+Y".into(),
                    _ => String::new(),
                };
                if menu_item(ui, name, &keys, true, self.view.compare == view) {
                    let view = self.view.compare.toggled(view);
                    self.set_compare(view);
                    ui.close();
                }
            }
            menu_separator(ui);
            for (transfer_kind, name, key) in [
                (Transfer::BeforeToAfter, "将编辑前设置复制到编辑后", "→"),
                (Transfer::AfterToBefore, "将编辑后设置复制到编辑前", "←"),
                (Transfer::Swap, "交换编辑前后设置", "↑"),
            ] {
                let keys = format!("{transfer}{key}");
                if menu_item(ui, name, &keys, self.document.full().is_some(), false) {
                    self.transfer(transfer_kind);
                    ui.close();
                }
            }
        });
    }
    /// Back to the raw defaults, like Lightroom's Reset.
    pub(super) fn reset_settings(&mut self) {
        self.document
            .edit
            .name_next_step(super::history::Step::new("重置设置", ""));
        let Some(defaults) = self.photo_defaults() else {
            self.document.edit.replace(Recipe::default());
            return;
        };
        if let Some(note) = defaults.note {
            self.status = note;
        }
        self.document.edit.replace(defaults.recipe);
    }
}
