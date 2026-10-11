//! The Masking drawer's side of Subject and Background: the two actions, what
//! they are waiting for, and what went wrong. Everything stays in the drawer; a
//! selection that fails or is cancelled leaves the photo as it was.
use super::models::download_megabytes;
use super::{Failure, Feature, Prompt, Request, Target};
use crate::app::Editor;
use crate::app::retouch_tool::{control_label, hint, indented};
use eframe::egui;

impl Editor {
    /// The Select Subject and Select Background actions, with whatever the running or
    /// failed selection has to say. While the first use is being set up, the actions
    /// give way to a card that says what is needed, in order, and does it.
    pub(in crate::app) fn selection_actions(&mut self, ui: &mut egui::Ui) {
        if let Some(request) = self.selection.running() {
            self.selection_progress(ui, request);
            return;
        }
        if self.selection.prompting.is_some() {
            self.prompting_card(ui);
            return;
        }
        if self.selection.prompt.is_some() || self.selection.upgrading.is_some() {
            self.setup_card(ui);
            return;
        }
        let reason = self.selection_unavailable();
        control_label(ui, "选片", |ui| {
            let gap = 4.;
            let w = (ui.available_width() - 2. * gap) / 3.;
            for (feature, tip) in [
                (Feature::Subject, "为照片主体创建蒙版"),
                (Feature::Sky, "为天空创建蒙版"),
                (Feature::Background, "为照片主体以外的区域创建蒙版"),
            ] {
                if self.feature_tile(ui, feature, w, tip, reason) {
                    self.request_selection(Request {
                        feature,
                        target: Target::NewMask,
                    });
                }
            }
        });
        if let Some(why) = reason {
            hint(ui, why);
        }
        if let Some((request, failure)) = self.selection.failure.clone() {
            self.selection_failure(ui, request, &failure);
        }
        self.model_footer(ui);
    }
    /// One of the Select tiles: an icon over its name, as Lightroom's Add New Mask.
    /// Returns whether it was clicked; when `disabled` says why, it is dimmed and says so
    /// on hover.
    fn feature_tile(
        &self,
        ui: &mut egui::Ui,
        feature: Feature,
        width: f32,
        tip: &str,
        disabled: Option<&str>,
    ) -> bool {
        let palette = crate::app::theme::palette(ui.ctx());
        let sense = if disabled.is_some() {
            egui::Sense::hover()
        } else {
            egui::Sense::click()
        };
        let (rect, response) = ui.allocate_exact_size(egui::vec2(width, 52.), sense);
        response.widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::WidgetType::Button,
                disabled.is_none(),
                format!("选择{}", crate::app::chinese::label(feature.name())),
            )
        });
        let hot = response.hovered() && disabled.is_none();
        ui.painter()
            .rect_filled(rect, 4., palette.gray(if hot { 54 } else { 38 }));
        let ink = palette.gray(match (disabled.is_some(), hot) {
            (true, _) => 95,
            (false, true) => 245,
            (false, false) => 200,
        });
        paint_feature_icon(
            ui.painter(),
            rect.center_top() + egui::vec2(0., 20.),
            feature,
            ink,
        );
        ui.painter().text(
            rect.center_bottom() - egui::vec2(0., 9.),
            egui::Align2::CENTER_CENTER,
            crate::app::chinese::label(feature.name()),
            egui::FontId::proportional(11.),
            ink,
        );
        let response = match disabled {
            Some(why) => response.on_hover_text(why),
            None => response
                .on_hover_text(tip)
                .on_hover_cursor(egui::CursorIcon::PointingHand),
        };
        response.clicked()
    }
    /// While aiming: what to do on the photo, and how to finish.
    fn prompting_card(&mut self, ui: &mut egui::Ui) {
        let Some(p) = &self.selection.prompting else {
            return;
        };
        let (feature, points, boxed, applied) = (
            p.request.feature,
            p.points.len(),
            p.bounds.is_some(),
            p.applied.is_some(),
        );
        let accent = egui::Color32::from_rgb(96, 150, 230);
        let failure = self.selection.failure.as_ref().map(|f| f.1.message());
        egui::Frame::new()
            .fill(egui::Color32::from_rgb(34, 44, 62))
            .stroke(egui::Stroke::new(1.5, accent))
            .corner_radius(5.)
            .inner_margin(egui::Margin::same(9))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(
                    egui::RichText::new(format!(
                        "选择{}：点击照片中的对象",
                        crate::app::chinese::label(feature.name())
                    ))
                    .strong()
                    .size(12.5),
                );
                ui.add_space(3.);
                let small = |text: &str| {
                    egui::RichText::new(text)
                        .size(11.)
                        .color(egui::Color32::LIGHT_GRAY)
                };
                ui.add(
                    egui::Label::new(small(if applied {
                        "点击主体更多位置添加区域，Alt 点击排除，或拖动新选框重新开始。"
                    } else {
                        "点击主体或拖动框选。可继续点击添加区域，按住 Alt 点击排除区域。"
                    }))
                    .wrap(),
                );
                let used = format!(
                    "{points} point{}{}",
                    if points == 1 { "" } else { "s" },
                    if boxed { "和一个选框" } else { "" }
                );
                ui.add(egui::Label::new(small(&used)));
                if let Some(why) = failure {
                    ui.colored_label(egui::Color32::from_rgb(230, 170, 90), why);
                }
                ui.add_space(4.);
                ui.horizontal(|ui| {
                    let done = egui::Button::new(egui::RichText::new("完成").strong())
                        .fill(egui::Color32::from_rgb(52, 98, 170));
                    if ui.add(done).on_hover_text("退出").clicked() {
                        self.selection.end_prompting();
                    }
                    if ui
                        .button("重新开始")
                        .on_hover_text("清除点击记录，下次点击重新选择")
                        .clicked()
                    {
                        self.clear_prompt();
                    }
                });
            });
    }
    /// Clicks, drags and markers on the photo while aiming. Returns that the tool owns
    /// the pointer.
    pub(in crate::app) fn prompt_overlay(
        &mut self,
        ui: &mut egui::Ui,
        response: &egui::Response,
        rect: egui::Rect,
        to_screen: &dyn Fn([f32; 2]) -> egui::Pos2,
        to_image: &dyn Fn(egui::Pos2) -> [f32; 2],
    ) -> bool {
        let busy = self.selection.running().is_some();
        if !busy {
            if response.drag_started()
                && let Some(origin) = ui.input(|i| i.pointer.press_origin())
            {
                self.selection.drag_from = Some(to_image(origin));
            }
            if response.drag_stopped()
                && let (Some(from), Some(pos)) = (
                    self.selection.drag_from.take(),
                    response.interact_pointer_pos(),
                )
            {
                self.prompt_box(from, to_image(pos));
            }
            if response.clicked()
                && let Some(pos) = response.interact_pointer_pos()
            {
                let alt = ui.input(|i| i.modifiers.alt);
                self.prompt_click(to_image(pos), !alt);
            }
        }
        let painter = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
        let Some(p) = &self.selection.prompting else {
            return true;
        };
        if let Some(b) = p.bounds {
            let r = egui::Rect::from_two_pos(to_screen([b[0], b[1]]), to_screen([b[2], b[3]]));
            painter.rect_stroke(
                r,
                0.,
                egui::Stroke::new(1.5, egui::Color32::from_rgb(110, 200, 130)),
                egui::StrokeKind::Middle,
            );
        }
        if let (Some(from), Some(pos)) = (self.selection.drag_from, response.interact_pointer_pos())
            && response.dragged()
        {
            let r = egui::Rect::from_two_pos(to_screen(from), pos);
            painter.rect_stroke(
                r,
                0.,
                egui::Stroke::new(1.5, egui::Color32::WHITE),
                egui::StrokeKind::Middle,
            );
        }
        for point in &p.points {
            let at = to_screen([point.x, point.y]);
            let color = if point.positive {
                egui::Color32::from_rgb(110, 200, 130)
            } else {
                egui::Color32::from_rgb(230, 90, 80)
            };
            painter.circle(at, 6., color, egui::Stroke::new(1.5, egui::Color32::WHITE));
            if !point.positive {
                let d = egui::vec2(3., 3.);
                painter.line_segment(
                    [at - d, at + d],
                    egui::Stroke::new(1.5, egui::Color32::WHITE),
                );
                let e = egui::vec2(3., -3.);
                painter.line_segment(
                    [at - e, at + e],
                    egui::Stroke::new(1.5, egui::Color32::WHITE),
                );
            }
        }
        if response.hovered() {
            ui.ctx().set_cursor_icon(if busy {
                egui::CursorIcon::Progress
            } else {
                egui::CursorIcon::Crosshair
            });
        }
        true
    }
    /// What the first use needs, as a highlighted card in the place the actions were:
    /// the steps with the finished ones ticked, and the button for the next.
    fn setup_card(&mut self, ui: &mut egui::Ui) {
        let request = match self.selection.prompt {
            Some(Prompt::Model(r) | Prompt::Upgrade(r)) => Some(r),
            None => self.selection.upgrading.map(|(_, r)| r),
        };
        let Some(request) = request else { return };
        let upgraded = self
            .library
            .as_ref()
            .and_then(|l| l.session.catalog.supports_raster_masks().ok())
            .unwrap_or(false);
        let model = self.selection.models.installed();
        let upgrading = self.selection.upgrading.is_some();
        let installing = self.selection.models.busy();
        let accent = egui::Color32::from_rgb(96, 150, 230);
        egui::Frame::new()
            .fill(egui::Color32::from_rgb(34, 44, 62))
            .stroke(egui::Stroke::new(1.5, accent))
            .corner_radius(5.)
            .inner_margin(egui::Margin::same(9))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(
                    egui::RichText::new(format!(
                        "使用“选择{}”之前",
                        crate::app::chinese::label(request.feature.name())
                    ))
                    .strong()
                    .size(12.5),
                );
                ui.add_space(4.);
                let step = |ui: &mut egui::Ui, done: bool, current: bool, text: &str| {
                    ui.horizontal(|ui| {
                        let (mark, color) = if done {
                            ("✓", egui::Color32::from_rgb(110, 200, 130))
                        } else if current {
                            ("▶", accent)
                        } else {
                            ("○", egui::Color32::GRAY)
                        };
                        ui.label(egui::RichText::new(mark).color(color).strong());
                        ui.label(egui::RichText::new(text).color(if done {
                            egui::Color32::GRAY
                        } else {
                            egui::Color32::WHITE
                        }));
                    });
                };
                step(ui, upgraded, !upgraded, "升级此图库");
                step(
                    ui,
                    model,
                    upgraded && !model,
                    &format!("下载选区模型（{} MB）", download_megabytes()),
                );
                ui.add_space(6.);
                if !upgraded {
                    self.upgrade_step(ui, request, upgrading);
                } else if !model {
                    self.model_step(ui, installing);
                }
                if !super::worker::runtime_present() {
                    ui.add_space(4.);
                    ui.colored_label(
                        egui::Color32::from_rgb(230, 170, 90),
                        "此 MyAIRAW 缺少 ONNX Runtime 库，暂时无法运行智能选择。",
                    );
                }
            });
    }
    fn upgrade_step(&mut self, ui: &mut egui::Ui, request: Request, upgrading: bool) {
        let small = |text: &str| {
            egui::RichText::new(text)
                .size(11.)
                .color(egui::Color32::LIGHT_GRAY)
        };
        ui.add(
            egui::Label::new(small(
                "智能选区蒙版需要新版图库格式。MyAIRAW 会先在图库旁保存同等大小的备份，再升级图库。旧版程序无法打开升级后的图库；升级时请勿在其他电脑或程序中同时打开该图库。",
            ))
            .wrap(),
        );
        ui.add_space(6.);
        if upgrading {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("正在备份并升级图库…");
            });
            return;
        }
        ui.horizontal(|ui| {
            let go = egui::Button::new(egui::RichText::new("升级图库…").strong())
                .fill(egui::Color32::from_rgb(52, 98, 170));
            if ui.add(go).clicked() {
                self.upgrade_catalog(request);
            }
            if ui.button("暂不").clicked() {
                self.selection.prompt = None;
            }
        });
    }
    fn model_step(&mut self, ui: &mut egui::Ui, installing: bool) {
        if let Some((done, total)) = self.selection.models.progress() {
            ui.add(
                egui::ProgressBar::new(done as f32 / total.max(1) as f32)
                    .desired_width(ui.available_width())
                    .text(format!("{} of {} MB", done / 1_000_000, total / 1_000_000)),
            );
            if ui.button("取消下载").clicked() {
                self.selection.models.cancel();
            }
            return;
        }
        if installing {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("处理中…");
            });
            return;
        }
        if let Some(why) = self.selection.models.failure() {
            ui.add(
                egui::Label::new(
                    egui::RichText::new(format!("下载已停止：{why}。重新下载即可从断点继续。"))
                        .size(11.)
                        .color(egui::Color32::from_rgb(230, 170, 90)),
                )
                .wrap(),
            );
            ui.add_space(4.);
        }
        ui.add(
            egui::Label::new(
                egui::RichText::new(
                    "模型在本机运行，照片不会上传。只需下载一次，并需要约同等大小的可用磁盘空间。",
                )
                .size(11.)
                .color(egui::Color32::LIGHT_GRAY),
            )
            .wrap(),
        );
        ui.add_space(6.);
        ui.horizontal(|ui| {
            let go = egui::Button::new(egui::RichText::new("下载").strong())
                .fill(egui::Color32::from_rgb(52, 98, 170));
            if ui.add(go).clicked() {
                self.install_model();
            }
            if ui.button("暂不").clicked() {
                self.selection.prompt = None;
            }
        });
    }
    /// Remove, once the model is installed.
    fn model_footer(&mut self, ui: &mut egui::Ui) {
        if self.selection.models.installed() && self.selection.running().is_none() {
            indented(ui, |ui| {
                let remove = ui
                    .small_button("移除选区模型")
                    .on_hover_text("删除已下载的模型。已有蒙版仍可使用，再次选择时会提示下载。")
                    .clicked();
                if remove {
                    let unload = self.selection.worker.unloader();
                    self.selection
                        .models
                        .remove(unload, self.tx.clone(), self.context.clone());
                }
            });
        }
    }
    fn selection_progress(&mut self, ui: &mut egui::Ui, request: Request) {
        control_label(ui, "选片", |ui| {
            ui.spinner();
            // No percentage: the runtime reports none for a run.
            ui.label(format!(
                "Finding the {}…",
                crate::app::chinese::label(request.feature.name()).to_lowercase()
            ))
            .on_hover_text("首次选择需等待几秒，后续选择更快");
            if ui
                .button("取消")
                .on_hover_text("停止选择，照片保持原样")
                .clicked()
            {
                self.cancel_selection();
            }
        });
    }
    fn selection_failure(&mut self, ui: &mut egui::Ui, request: Request, failure: &Failure) {
        if *failure == Failure::ModelMissing {
            self.selection.failure = None;
            self.selection.prompt = Some(Prompt::Model(request));
            return;
        }
        hint(ui, &failure.message());
        if *failure == Failure::NoSubject {
            hint(ui, "自动选区可识别人和动物。其他对象请在照片中点击选择。");
        }
        indented(ui, |ui| {
            if *failure == Failure::NoSubject
                && ui
                    .add(
                        egui::Button::new(egui::RichText::new("点击主体…").strong())
                            .fill(egui::Color32::from_rgb(52, 98, 170)),
                    )
                    .clicked()
            {
                self.begin_aiming(request);
            }
            if ui.button("重试").clicked() {
                self.selection.failure = None;
                // A failed click is retried with its clicks, not as an automatic selection.
                if self.selection.prompting.is_some() {
                    self.run_prompt();
                } else {
                    self.request_selection(request);
                }
            }
            if ui.button("关闭提示").clicked() {
                self.selection.failure = None;
            }
        });
    }
    /// Subject and Background entries for a mask's Add, Subtract and Intersect menus.
    pub(in crate::app) fn selection_menu(
        &mut self,
        ui: &mut egui::Ui,
        mask: usize,
        op: crate::model::masks::MaskOp,
    ) {
        ui.separator();
        let reason = self.selection_unavailable();
        for (feature, label) in [
            (Feature::Subject, "主体"),
            (Feature::Sky, "天空"),
            (Feature::Background, "背景"),
        ] {
            let response = ui.add_enabled(
                reason.is_none() && self.selection.running().is_none(),
                egui::Button::new(label),
            );
            let response = match reason {
                Some(why) => response.on_disabled_hover_text(why),
                None => response,
            };
            if response.clicked() {
                self.request_selection(Request {
                    feature,
                    target: Target::Component { mask, op },
                });
                ui.close();
            }
        }
    }
    /// The selected generated component's source, Regenerate and, for subjects, aiming
    /// with clicks.
    pub(in crate::app) fn regenerate_ui(
        &mut self,
        ui: &mut egui::Ui,
        mask: usize,
        component: usize,
        feature: Feature,
        source: &str,
    ) {
        hint(ui, source);
        let reason = self.selection_unavailable();
        let busy = self.selection.running().is_some();
        let free = reason.is_none() && !busy && self.selection.prompting.is_none();
        indented(ui, |ui| {
            let regenerate = ui.add_enabled(free, egui::Button::new("重新生成"));
            let regenerate = match reason {
                Some(why) => regenerate.on_disabled_hover_text(why),
                None => {
                    regenerate.on_hover_text("重新选择并替换此蒙版部分，其他部分与调整保持不变")
                }
            };
            if regenerate.clicked() {
                self.request_selection(Request {
                    feature,
                    target: Target::Regenerate { mask, component },
                });
            }
            if feature != Feature::Sky {
                let refine = ui
                    .add_enabled(free, egui::Button::new("点击细化…"))
                    .on_hover_text("点击照片添加选区，按住 Alt 点击排除区域");
                if refine.clicked() {
                    self.begin_refining(mask, component);
                }
            }
        });
        if let Some(request) = self.selection.running() {
            self.selection_progress(ui, request);
        }
        if let Some((request, failure)) = self.selection.failure.clone() {
            self.selection_failure(ui, request, &failure);
        }
        if self.selection.prompt.is_some() || self.selection.upgrading.is_some() {
            self.setup_card(ui);
        }
    }
}

/// An icon for each selection, in a dashed frame as Lightroom draws them: a head and
/// shoulders, a sky over a hill, and the frame with the head and shoulders cut out.
pub(in crate::app) fn paint_feature_icon(
    painter: &egui::Painter,
    c: egui::Pos2,
    feature: Feature,
    ink: egui::Color32,
) {
    let stroke = egui::Stroke::new(1.2, ink);
    let (w, h) = (11., 9.5);
    let corners = [
        c + egui::vec2(-w, -h),
        c + egui::vec2(w, -h),
        c + egui::vec2(w, h),
        c + egui::vec2(-w, h),
        c + egui::vec2(-w, -h),
    ];
    painter.add(egui::Shape::dashed_line(&corners, stroke, 2.2, 1.8));
    // A head and shoulders, centred a little low.
    let person = |color: egui::Color32| {
        painter.circle_filled(c + egui::vec2(0., -2.6), 2.9, color);
        painter.add(egui::Shape::convex_polygon(
            (0..=12)
                .map(|i| {
                    let t = std::f32::consts::PI * i as f32 / 12.;
                    c + egui::vec2(-t.cos() * 5.8, 8.4 - t.sin() * 5.4)
                })
                .collect(),
            color,
            egui::Stroke::NONE,
        ));
    };
    match feature {
        Feature::Subject => person(ink),
        Feature::Sky => {
            painter.circle_filled(c + egui::vec2(5., -3.8), 1.9, ink);
            painter.add(egui::Shape::convex_polygon(
                vec![
                    c + egui::vec2(-8., 7.),
                    c + egui::vec2(-3., -1.),
                    c + egui::vec2(0.5, 3.),
                    c + egui::vec2(3., 0.),
                    c + egui::vec2(8., 7.),
                ],
                ink,
                egui::Stroke::NONE,
            ));
        }
        Feature::Background => {
            painter.rect_filled(
                egui::Rect::from_center_size(c, egui::vec2(2. * w - 3., 2. * h - 3.)),
                1.,
                ink.gamma_multiply(0.55),
            );
            // The subject cut out of it: drawn in the tile's own colour.
            person(egui::Color32::from_gray(38));
        }
    }
}
