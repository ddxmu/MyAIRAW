use super::clipping::{self, ClipSide};
use super::crop_tool::{Guide, GuideShow, Ruler};
use super::dialogs::FileDialog;
use super::state::{MixerTab, Tool};
use super::targeted_tool::{hsl_target, target_button};
use super::tone_drag::tone_drag_ui;
use super::widgets::{
    SliderEvent, adjustment_section, name_history_step, parametric_curve_ui, segmented,
    setting_slider, slider, slider_with, switched_section, tone_curve_ui, toolbar_action,
};
use super::worker::AutoKind;
use super::{Editor, bulk_import::ImportKind};
use crate::app::icons::{self, Icon};
use crate::model::panels::{Panel, PanelState};
use crate::model::params::ParameterId;
use crate::model::recipe::Recipe;
use crate::model::recipe::Treatment;
use crate::model::{
    operators::{SharpeningModel, SharpeningSliders},
    white_balance::NamedWhiteBalance,
};
use crate::{app::theme, develop::targeted::Target};
use eframe::egui::{self, Color32, Pos2, Rect, Sense, Stroke, Vec2};

mod lens_profile;

/// A histogram corner's clipping triangle: its corner point, which way it
/// points (1 right, -1 left) and the area that takes its clicks.
#[derive(Clone, Copy)]
struct ClipTriangle {
    corner: Pos2,
    dir: f32,
    hit: Rect,
}
impl ClipTriangle {
    fn new(histogram: Rect, side: ClipSide) -> Self {
        let (corner, dir) = match side {
            ClipSide::Shadows => (histogram.left_top() + Vec2::new(6., 6.), 1.),
            ClipSide::Highlights => (histogram.right_top() + Vec2::new(-6., 6.), -1.),
        };
        Self {
            corner,
            dir,
            hit: Rect::from_center_size(corner + Vec2::new(4. * dir, 3.), Vec2::splat(16.)),
        }
    }
}

impl Editor {
    /// Lightroom-style histogram: filled channels whose overlaps mix to
    /// cyan, magenta, yellow and gray, with clipping indicators in the corners.
    /// Dragging in it moves Blacks, Shadows, Exposure, Highlights or Whites.
    pub(super) fn histogram_ui(&mut self, ui: &mut egui::Ui) {
        let palette = theme::palette(ui.ctx());
        let (rect, _) =
            ui.allocate_exact_size(Vec2::new(ui.available_width(), 96.), Sense::hover());
        let painter = ui.painter().clone();
        painter.rect_filled(rect, 2., palette.gray(20));
        for i in 1..5 {
            let x = rect.left() + i as f32 / 5. * rect.width();
            painter.line_segment(
                [Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())],
                Stroke::new(1., palette.gray(32)),
            );
        }
        let histogram = self.preview.histogram;
        let h = histogram.bins;
        // Light smoothing, and scaling that ignores the clipped end bins.
        let smooth = |c: usize, i: usize| {
            let at = |j: isize| h[c][j.clamp(0, 255) as usize] as f32;
            let i = i as isize;
            (at(i - 1) + 2. * at(i) + at(i + 1)) / 4.
        };
        let max = (2..254)
            .flat_map(|i| (0..3).map(move |c| (c, i)))
            .map(|(c, i)| smooth(c, i))
            .fold(1., f32::max);
        let plot = rect.shrink2(Vec2::new(0., 4.));
        let bar = plot.width() / 256.;
        let colors = [
            Color32::from_rgb(196, 58, 52),
            Color32::from_rgb(62, 170, 70),
            Color32::from_rgb(56, 104, 220),
        ];
        let pair = |a: usize, b: usize| match (a.min(b), a.max(b)) {
            (0, 1) => Color32::from_rgb(190, 176, 60),
            (1, 2) => Color32::from_rgb(58, 168, 186),
            _ => Color32::from_rgb(170, 70, 170),
        };
        for i in 0..256 {
            let mut v: Vec<(f32, usize)> = (0..3)
                .map(|c| ((smooth(c, i) / max).sqrt().min(1.) * plot.height(), c))
                .collect();
            v.sort_by(|a, b| a.0.total_cmp(&b.0));
            let x = plot.left() + i as f32 * bar;
            let segment = |from: f32, to: f32, color: Color32| {
                if to > from {
                    painter.rect_filled(
                        Rect::from_min_max(
                            Pos2::new(x, plot.bottom() - to),
                            Pos2::new(x + bar + 0.5, plot.bottom() - from),
                        ),
                        0.,
                        color,
                    );
                }
            };
            segment(0., v[0].0, palette.gray(150));
            segment(v[0].0, v[1].0, pair(v[1].1, v[2].1));
            segment(v[1].0, v[2].0, colors[v[2].1]);
        }
        // After the bars, so the region shows over them; before the triangles,
        // so their clicks stay theirs.
        let triangles = ClipSide::BOTH.map(|side| ClipTriangle::new(rect, side));
        let region = tone_drag_ui(
            ui,
            rect,
            &triangles.map(|t| t.hit),
            &mut self.view.tone_drag,
            self.document.edit.recipe_mut(),
        );
        if let Some(region) = region {
            let [from, to] = region.span();
            painter.rect_filled(
                Rect::from_x_y_ranges(
                    rect.left() + from * rect.width()..=rect.left() + to * rect.width(),
                    rect.y_range(),
                ),
                0.,
                Color32::from_white_alpha(14),
            );
        }
        // Clipping triangles, as Lightroom's: each lit in the colours of the
        // channels clipping at its end; a click toggles its warning, hovering
        // shows it while the pointer stays, and J toggles both.
        for (side, ClipTriangle { corner, dir, hit }) in ClipSide::BOTH.into_iter().zip(triangles) {
            let left = side == ClipSide::Shadows;
            let response = ui
                .interact(hit, ui.id().with(("clip", left)), Sense::click())
                .on_hover_text(if left {
                    "显示阴影剪裁 · J 同时显示高光"
                } else {
                    "显示高光剪裁 · J 同时显示阴影"
                });
            if response.clicked() {
                self.view.clipping.toggle(side);
            }
            if response.hovered() {
                self.view.clipping.set_hover(Some(side));
            }
            let on = self.view.clipping.is_on(side);
            let color = clipping::indicator_color(clipping::clipped_channels(&histogram, side))
                .unwrap_or_else(|| palette.gray(if on || response.hovered() { 150 } else { 80 }));
            painter.add(egui::Shape::convex_polygon(
                vec![
                    corner,
                    corner + Vec2::new(9. * dir, 0.),
                    corner + Vec2::new(0., 7.),
                ],
                color,
                if on {
                    Stroke::new(1., Color32::WHITE)
                } else {
                    Stroke::NONE
                },
            ));
        }
        // The region and its value take the EXIF line's place, as in Lightroom.
        let region_text = region.map(|region| {
            format!(
                "{}   {}",
                region.label(),
                region.display(region.value(self.document.edit.recipe()))
            )
        });
        // So does the RGB readout while the pointer is over the photo.
        let exif = region_text
            .or_else(|| self.view.readout.text())
            .unwrap_or_else(|| {
                self.document
                    .metadata
                    .as_ref()
                    .map_or_else(String::new, |m| {
                        let info = crate::metadata::PhotoInfo::from_metadata(m);
                        [
                            info.iso_text(),
                            info.focal_text(),
                            info.aperture_text(),
                            info.shutter_text(),
                        ]
                        .into_iter()
                        .flatten()
                        .collect::<Vec<_>>()
                        .join("     ")
                    })
            });
        ui.vertical_centered(|ui| {
            ui.label(egui::RichText::new(exif).size(11.).color(palette.gray(170)))
                .on_hover_text("整张照片的输出直方图");
        });
    }
    /// Lightroom's tool strip: Crop, Remove, Red Eye and Masking, with the open tool's
    /// drawer below it.
    pub(super) fn tool_strip(&mut self, ui: &mut egui::Ui) {
        let palette = theme::palette(ui.ctx());
        ui.add_space(6.);
        // Red Eye has no shortcut, as in Lightroom.
        const TOOLS: [(Tool, &str, &str); 4] = [
            (Tool::Crop, "裁剪", "裁剪与拉直 · R"),
            (Tool::Remove, "移除", "污点移除：修复与仿制 · Q"),
            (Tool::RedEye, "红眼", "红眼校正"),
            (Tool::Mask, "蒙版", "蒙版 · Shift+W"),
        ];
        let (strip, _) =
            ui.allocate_exact_size(Vec2::new(ui.available_width(), 28.), Sense::hover());
        let width = (strip.width() - 12.) / 4.;
        for (k, (tool, label, tip)) in TOOLS.into_iter().enumerate() {
            let rect = Rect::from_min_size(
                strip.min + Vec2::new(k as f32 * (width + 4.), 0.),
                Vec2::new(width, strip.height()),
            );
            let response = ui
                .interact(rect, ui.id().with(("tool", label)), Sense::click())
                .on_hover_text(tip);
            let active = self.view.is(tool);
            ui.painter().rect_filled(
                rect,
                3.,
                palette.gray(if active {
                    72
                } else if response.hovered() {
                    50
                } else {
                    38
                }),
            );
            let icon = rect.left_center() + Vec2::new(16., 0.);
            match tool {
                Tool::Crop => crop_icon(ui.painter(), icon, active),
                Tool::Remove => heal_icon(ui.painter(), icon, active),
                Tool::RedEye => eye_icon(ui.painter(), icon, active),
                _ => mask_icon(ui.painter(), icon, active),
            }
            ui.painter().text(
                rect.left_center() + Vec2::new(30., 0.),
                egui::Align2::LEFT_CENTER,
                label,
                egui::FontId::proportional(12.),
                palette.gray(if active { 245 } else { 200 }),
            );
            if response
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .clicked()
            {
                self.view.toggle(tool);
            }
        }
        let drawer = |ui: &mut egui::Ui, add: &mut dyn FnMut(&mut egui::Ui)| {
            ui.add_space(4.);
            egui::Frame::new()
                .fill(palette.gray(40))
                .corner_radius(3.)
                .inner_margin(egui::Margin {
                    left: 0,
                    right: 8,
                    top: 8,
                    bottom: 8,
                })
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing = Vec2::new(4., 6.);
                    add(ui)
                });
        };
        match self.view.tool {
            Tool::Remove => {
                return drawer(ui, &mut |ui| {
                    experimental(ui, UNMEASURED);
                    self.retouch_panel(ui)
                });
            }
            Tool::RedEye => {
                return drawer(ui, &mut |ui| {
                    experimental(ui, RED_EYE_NOTE);
                    self.red_eye_panel(ui)
                });
            }
            Tool::Mask => {
                return drawer(ui, &mut |ui| {
                    experimental(ui, UNMEASURED);
                    self.mask_panel(ui)
                });
            }
            Tool::Crop => {}
            _ => return,
        }
        ui.add_space(4.);
        let mut action = None;
        let mut guides = self.view.crop_guides;
        let analysing = self.document.straighten.is_running();
        let r = self.document.edit.recipe_mut();
        egui::Frame::new()
            .fill(palette.gray(40))
            .corner_radius(3.)
            .inner_margin(egui::Margin {
                left: 0,
                right: 8,
                top: 8,
                bottom: 8,
            })
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing = Vec2::new(4., 6.);
                control_row(ui, "比例", |ui| {
                    let swap = 26.;
                    egui::ComboBox::from_id_salt("crop-aspect")
                        .width(ui.available_width() - swap - 4.)
                        .selected_text(aspect_name(self.view.aspect))
                        .show_ui(ui, |ui| {
                            for (aspect, name) in ASPECTS {
                                ui.selectable_value(&mut self.view.aspect, aspect, name);
                            }
                        });
                    if ui
                        .add_sized([swap, 20.], egui::Button::new("⇄"))
                        .on_hover_text("交换横竖方向 · X")
                        .clicked()
                    {
                        action = Some(CropAction::Swap);
                    }
                });
                setting_slider(ui, ParameterId::Straighten, &mut r.straighten, 0.);
                control_row(ui, "拉直", |ui| {
                    let w = (ui.available_width() - 4.) / 2.;
                    let armed = self.view.ruler == Ruler::Armed;
                    if ui
                        .add_sized([w, 20.], egui::Button::new("标尺").selected(armed))
                        .on_hover_text("沿水平或垂直线拖动拉直 · 也可在照片上 Cmd 拖动")
                        .clicked()
                    {
                        self.view.ruler = if armed { Ruler::Off } else { Ruler::Armed };
                    }
                    if ui
                        .add_enabled(
                            !analysing,
                            egui::Button::new("自动").min_size(Vec2::new(w, 20.)),
                        )
                        .on_hover_text("仅按倾斜角度拉平照片，相当于 Upright 水平校正")
                        .clicked()
                    {
                        action = Some(CropAction::AutoStraighten);
                    }
                });
                control_row(ui, "方向", |ui| {
                    let w = ui.available_width();
                    let mut none = usize::MAX;
                    segmented(
                        ui,
                        &mut none,
                        &[(0, "左旋"), (1, "右旋"), (2, "水平翻转"), (3, "垂直翻转")],
                        w,
                    );
                    use crate::develop::{Mirror, QuarterTurn, mirror, turn};
                    match none {
                        0 => turn(r, QuarterTurn::Left),
                        1 => turn(r, QuarterTurn::Right),
                        2 => mirror(r, Mirror::Horizontal),
                        3 => mirror(r, Mirror::Vertical),
                        _ => {}
                    }
                });
                control_row(ui, "叠加", |ui| {
                    let show = 78.;
                    egui::ComboBox::from_id_salt("crop-guide")
                        .width(ui.available_width() - show - 4.)
                        .selected_text(guides.guide.name())
                        .show_ui(ui, |ui| {
                            for guide in Guide::ALL {
                                if ui
                                    .selectable_label(guides.guide == guide, guide.name())
                                    .clicked()
                                {
                                    guides.guide = guide;
                                    guides.orientation = 0;
                                }
                            }
                        })
                        .response
                        .on_hover_text("O 切换参考线，Shift+O 旋转");
                    egui::ComboBox::from_id_salt("crop-guide-show")
                        .width(show)
                        .selected_text(guides.show.name())
                        .show_ui(ui, |ui| {
                            for when in GuideShow::ALL {
                                ui.selectable_value(&mut guides.show, when, when.name());
                            }
                        })
                        .response
                        .on_hover_text("叠加显示时机：始终、指针在照片上或从不");
                });
                ui.horizontal(|ui| {
                    ui.add_space(83.);
                    ui.label(
                        egui::RichText::new("拖动照片上的裁剪框，修改实时生效。")
                            .size(10.)
                            .color(palette.gray(125)),
                    );
                });
                ui.horizontal(|ui| {
                    ui.add_space(83.);
                    let w = (ui.available_width() - 4.) / 2.;
                    if ui
                        .add_sized([w, 22.], egui::Button::new("重置"))
                        .on_hover_text("移除裁剪、拉直、旋转与翻转")
                        .clicked()
                    {
                        r.crop = [0., 0., 1., 1.];
                        r.straighten = 0.;
                        r.rotation = 0;
                        r.flip_x = false;
                        r.flip_y = false;
                    }
                    if ui
                        .add_sized(
                            [w, 22.],
                            egui::Button::new(
                                egui::RichText::new("完成").color(palette.on_accent_text(245)),
                            )
                            .fill(palette.accent()),
                        )
                        .on_hover_text("完成裁剪 · Enter 或 R")
                        .clicked()
                    {
                        self.view.tool = Tool::None;
                    }
                });
            });
        self.set_crop_guides(guides);
        match action {
            Some(CropAction::Swap) => self.swap_crop_orientation(),
            Some(CropAction::AutoStraighten) => self.start_auto_straighten(),
            None => {}
        }
    }
    pub(super) fn controls(&mut self, ui: &mut egui::Ui) {
        let palette = theme::palette(ui.ctx());
        ui.spacing_mut().item_spacing = Vec2::new(4., 3.);
        ui.spacing_mut().button_padding = Vec2::new(6., 2.);
        ui.spacing_mut().interact_size.y = 20.;
        self.histogram_ui(ui);
        self.tool_strip(ui);
        ui.add_space(6.);
        let mut import_profiles = false;
        let mut import_lens = false;
        let mut import_folder = None;
        let mut import_adobe = false;
        // Cached per camera: this scans Adobe's profile folders.
        let adobe_key = self
            .document
            .metadata
            .as_ref()
            .map(|m| egui::Id::new(("adobe-profiles", &m.make, &m.model)));
        let adobe: Vec<std::path::PathBuf> = match (adobe_key, &self.document.metadata) {
            (Some(key), Some(m)) => ui.ctx().data_mut(|d| {
                d.get_temp_mut_or_insert_with(key, || crate::camera_profiles::adobe_installed(m))
                    .clone()
            }),
            _ => Vec::new(),
        };
        let metadata = self.document.metadata.clone();
        let profiles = self.document.profiles.clone();
        let profile_errors = self.document.profile_errors.clone();
        let histogram = self.preview.histogram.bins;
        // Auto needs the decoded photo, and runs one estimate at a time.
        let auto_ready = self.document.full().is_some() && !self.document.auto.is_running();
        let auto_in_effect = self.auto_in_effect();
        let mut auto_request = None;
        // Upright analyses the decoded photo once and keeps a correction for every mode.
        let upright_ready = self.document.full().is_some() && !self.document.upright.is_running();
        let mut upright_request = false;
        let mut guided_action = None;
        let mut treatment_request = None;
        let mut auto_mix_request = false;
        let mut profile_changed_from = None;
        // A conversion to black & white waiting for the photo to decode.
        let pending_treatment = self
            .document
            .pending_treatment
            .as_ref()
            .map(|p| p.treatment);
        let grading_document = self.document.edit.history().id();
        let saved_curves = self.saved_curves();
        let mut curve_choice = None;
        // The Targeted Adjustment Tool: the sliders a drag is moving, and a target
        // button's click.
        let targeted = self.targeted_weights();
        let mut targeted_request = None;
        let view = &mut self.view;
        let (r, photo) = self.document.recipe_and_colors();

        if adjustment_section(ui, "基本", |ui| {
            let shortcut = if cfg!(target_os = "macos") {
                "⌘⇧U"
            } else {
                "Ctrl+Shift+U"
            };
            // Auto, right-aligned, styled as the toolbar's Before and Clipping.
            let (row, _) =
                ui.allocate_exact_size(Vec2::new(ui.available_width(), 32.), Sense::hover());
            ui.scope_builder(
                egui::UiBuilder::new()
                    .max_rect(row)
                    .layout(egui::Layout::right_to_left(egui::Align::Center)),
                |ui| {
                    let tip = if auto_in_effect {
                        "已应用自动设置".to_owned()
                    } else {
                        format!("自动设置色调滑块 · {shortcut}")
                    };
                    let auto =
                        toolbar_action(ui, "自动", 52., false, auto_ready && !auto_in_effect, 0);
                    if auto.on_hover_text(tip).clicked() {
                        auto_request = Some(AutoKind::Settings);
                    }
                },
            );
            // Lightroom's Treatment, above the profile; V switches it.
            // A conversion waiting for the photo to decode shows as made.
            let shown = pending_treatment.unwrap_or_else(|| r.treatment());
            let mut treatment = shown;
            control_row(ui, "处理方式", |ui| {
                let w = ui.available_width();
                segmented(
                    ui,
                    &mut treatment,
                    &[(Treatment::Color, "彩色"), (Treatment::BlackWhite, "黑白")],
                    w,
                );
            });
            if treatment != shown {
                treatment_request = Some(treatment);
            }
            let old_profile = r.profile.clone();
            // From engine 4 the matrix path renders through the DNG default look.
            let matrix = if r.engine >= 4 {
                "默认（相机矩阵）"
            } else {
                "相机矩阵"
            };
            control_row(ui, "配置文件", |ui| {
                egui::ComboBox::from_id_salt("camera-profile")
                    .width(ui.available_width())
                    .selected_text(r.profile.as_ref().map_or(matrix, |p| super::chinese::label(&p.name)))
                    .show_ui(ui, |ui| {
                        if ui.selectable_value(&mut r.profile, None, matrix).changed() {
                            r.engine = r.engine.max(3);
                        }
                        for profile in &profiles {
                            if ui
                                .selectable_value(&mut r.profile, Some(profile.clone()), super::chinese::label(&profile.name))
                                .on_hover_text(&profile.copyright)
                                .changed()
                            {
                                r.engine = r.engine.max(3);
                            }
                        }
                        ui.separator();
                        if !adobe.is_empty()
                            && ui
                                .button("导入此相机的 Adobe 配置")
                                .on_hover_text(format!(
                                    "从 Lightroom 安装目录添加 Adobe Standard 和相机匹配配置（{} 个文件），为 Adobe Color 等风格提供基础。",
                                    adobe.len()
                                ))
                                .clicked()
                        {
                            import_adobe = true;
                            ui.close();
                        }
                        if ui
                            .button("导入配置文件…")
                            .on_hover_text("导入 DCP 相机配置与 XMP 风格，请同时选择匹配的基础 DCP 和 XMP。")
                            .clicked()
                        {
                            import_profiles = true;
                            ui.close();
                        }
                        if ui
                            .button("从文件夹导入配置…")
                            .on_hover_text("导入文件夹及子文件夹内全部 DCP 配置和 XMP 风格")
                            .clicked()
                        {
                            import_folder = Some(ImportKind::CameraProfiles);
                            ui.close();
                        }
                    });
            });
            // Lightroom's Profile Amount, under the profile. Profiles without one show
            // it dimmed at 100%, so the panel doesn't move when switching.
            let supports_amount = r.profile.as_ref().is_some_and(|p| p.supports_amount());
            ui.add_enabled_ui(supports_amount, |ui| {
                let mut fixed = 1.;
                let amount = if supports_amount {
                    &mut r.profile_amount
                } else {
                    &mut fixed
                };
                ui.push_id("profile-amount", |ui| {
                    slider_with(ui, "数量", amount, 0. ..=2., 1., Some((100., 0)), None)
                });
            })
            .response
            .on_disabled_hover_text("此配置不支持强度调整");
            if !profile_errors.is_empty() {
                ui.horizontal(|ui| {
                    ui.add_space(88.);
                    ui.small(format!("{} 个配置文件不可用", profile_errors.len()))
                        .on_hover_text(profile_errors.join("\n"));
                    if !adobe.is_empty() && ui.small_button("导入 Adobe 基础配置").clicked() {
                        import_adobe = true;
                    }
                });
            }
            if old_profile != r.profile {
                // A newly chosen profile starts at 100%, as in Lightroom.
                r.profile_amount = 1.;
            }
            if old_profile != r.profile
                && let Some(m) = &metadata
            {
                r.profile_changed(m);
                profile_changed_from = Some(old_profile);
            }
            // White balance is its own group below the profile, as in Lightroom.
            ui.add_space(12.);
            let as_shot = metadata.as_ref().map(|m| {
                let mut shot = r.clone();
                shot.reset_white_balance(m);
                (shot.temperature, shot.tint)
            });
            control_row(ui, "白平衡", |ui| {
                let current = (r.temperature, r.tint);
                let near =
                    |(t, n): (f32, f32)| (t - current.0).abs() < 1. && (n - current.1).abs() < 0.5;
                let selected = if r.auto_white_balance.is_some_and(|[t, n]| near((t, n))) {
                    "自动"
                } else if as_shot.is_some_and(near) {
                    "拍摄时"
                } else {
                    NamedWhiteBalance::ALL
                        .into_iter()
                        .find(|w| near((w.values().temperature, w.values().tint)))
                        .map_or("自定义", |w| super::chinese::label(w.name()))
                };
                // The selector sits at the row's far left, so the menu lines up
                // with Profile's.
                let rect = Rect::from_center_size(
                    Pos2::new(ui.max_rect().left() - 88. + 13., ui.max_rect().center().y),
                    Vec2::new(26., 20.),
                );
                let response = ui.interact(rect, ui.id().with("wb-selector"), Sense::click());
                let picking = view.is(Tool::WhiteBalance);
                if picking || response.hovered() {
                    ui.painter()
                        .rect_filled(rect, 3., palette.gray(if picking { 72 } else { 50 }));
                }
                eyedropper_icon(ui.painter(), rect.center(), picking || response.hovered());
                if response
                    .on_hover_text("白平衡吸管（W）：点击照片中的中性区域")
                    .clicked()
                {
                    view.toggle(Tool::WhiteBalance);
                }
                egui::ComboBox::from_id_salt("white-balance")
                    .width(ui.available_width())
                    .selected_text(selected)
                    .show_ui(ui, |ui| {
                        if ui
                            .selectable_label(selected == "拍摄时", "拍摄时")
                            .clicked()
                            && let Some(m) = &metadata
                        {
                            r.wb = [1.; 3];
                            r.reset_white_balance(m);
                        }
                        if ui
                            .add_enabled(
                                auto_ready,
                                egui::Button::selectable(selected == "自动", "自动"),
                            )
                            .on_hover_text("将照片中接近中性的区域调整为中性")
                            .clicked()
                        {
                            auto_request = Some(AutoKind::WhiteBalance);
                        }
                        for named in NamedWhiteBalance::ALL {
                            if ui
                                .selectable_label(
                                    selected == super::chinese::label(named.name()),
                                    super::chinese::label(named.name()),
                                )
                                .clicked()
                                && let Some(m) = &metadata
                            {
                                let values = named.values();
                                r.temperature = values.temperature;
                                r.tint = values.tint;
                                r.update_wb(m);
                                r.auto_white_balance = None;
                            }
                        }
                        ui.add_enabled(
                            false,
                            egui::Button::selectable(selected == "自定义", "自定义"),
                        );
                    });
            });
            let photo = metadata.as_ref();
            setting_control(ui, r, ParameterId::Temperature, 6500., photo);
            setting_control(ui, r, ParameterId::Tint, 0., photo);
            subheading(ui, "色调");
            for id in [
                ParameterId::Exposure,
                ParameterId::Contrast,
                ParameterId::Highlights,
                ParameterId::Shadows,
                ParameterId::Whites,
                ParameterId::Blacks,
            ] {
                setting_control(ui, r, id, 0., photo);
            }
            subheading(ui, "质感");
            for id in [
                ParameterId::Texture,
                ParameterId::Clarity,
                ParameterId::Dehaze,
                ParameterId::Vibrance,
                ParameterId::Saturation,
            ] {
                setting_control(ui, r, id, 0., photo);
            }
        }) {
            r.wb = [1.; 3];
            r.tint = 0.;
            if let Some(m) = &metadata {
                r.reset_white_balance(m);
            }
            r.effects.monochrome = false;
            r.effects.texture = 0.;
            r.effects.clarity = 0.;
            r.effects.dehaze = 0.;
            r.vibrance = 0.;
            r.saturation = 0.;
            r.exposure = Recipe::default().exposure;
            r.contrast = 0.;
            r.highlights = 0.;
            r.shadows = 0.;
            r.whites = 0.;
            r.blacks = 0.;
        }

        let mut switch = PanelSwitch::new(r, Panel::ToneCurve);
        if switched_section(ui, "色调曲线", &mut switch.state, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.;
                let (button, _) = ui.allocate_exact_size(Vec2::new(22., 22.), Sense::hover());
                if target_button(
                    ui,
                    button,
                    view.is(Tool::Targeted(Target::ToneCurve)),
                    &format!(
                        "目标调整：在照片上上下拖动，调整对应区域的色调曲线 · {}",
                        targeted_shortcut("T")
                    ),
                ) {
                    targeted_request = Some(Target::ToneCurve);
                }
                segmented(
                    ui,
                    &mut view.parametric_curve,
                    &[(true, "参数"), (false, "点")],
                    112.,
                );
                if !view.parametric_curve {
                    let w = ui.available_width();
                    segmented(
                        ui,
                        &mut view.selected_curve,
                        &[(0, "RGB"), (1, "R"), (2, "G"), (3, "B")],
                        w,
                    );
                }
            });
            ui.add_space(4.);
            if view.parametric_curve {
                let moving = targeted.filter(|w| w.target == Target::ToneCurve);
                parametric_curve_ui(
                    ui,
                    &mut r.effects,
                    r.parametric_model,
                    &histogram,
                    moving.and_then(|w| (0..4).find(|i| w.shares[*i] > 0.)),
                );
                subheading(ui, "区域");
                // Lightroom lists them lightest first.
                for (i, id) in ParameterId::PARAMETRIC.into_iter().enumerate().rev() {
                    let row = ui.push_id(("parametric", i), |ui| {
                        setting_slider(ui, id, id.value_mut(r), 0.)
                    });
                    highlight_targeted(ui, row.response.rect, moving.map_or(0., |w| w.shares[i]));
                }
            } else {
                ui.push_id(view.selected_curve, |ui| {
                    tone_curve_ui(
                        ui,
                        if view.selected_curve == 0 {
                            &mut r.curve
                        } else {
                            &mut r.effects.channels[view.selected_curve - 1]
                        },
                        &histogram,
                        view.selected_curve,
                    );
                });
                // Lightroom's Point Curve menu: the built-in curves, saved ones, Save….
                use crate::presets::curves::ShownCurve;
                let chosen = ShownCurve::of(r, &saved_curves);
                let shown = chosen.name(&saved_curves).to_string();
                control_row(ui, "点曲线", |ui| {
                    egui::ComboBox::from_id_salt("point-curve")
                        .width(ui.available_width())
                        .selected_text(super::chinese::label(&shown))
                        .show_ui(ui, |ui| {
                            use super::curve_menu::CurveChoice;
                            use crate::presets::curves::BuiltinCurve;
                            for curve in BuiltinCurve::ALL {
                                if ui
                                    .selectable_label(
                                        chosen == ShownCurve::Builtin(curve),
                                        super::chinese::label(curve.name()),
                                    )
                                    .clicked()
                                {
                                    curve_choice = Some(CurveChoice::Builtin(curve));
                                }
                            }
                            if !saved_curves.is_empty() {
                                ui.separator();
                            }
                            for (i, saved) in saved_curves.iter().enumerate() {
                                if ui
                                    .selectable_label(chosen == ShownCurve::Saved(i), &saved.name)
                                    .clicked()
                                {
                                    curve_choice = Some(CurveChoice::Saved(saved.clone()));
                                }
                            }
                            ui.separator();
                            if ui
                                .selectable_label(false, "保存…")
                                .on_hover_text("命名保存 RGB、红、绿、蓝曲线")
                                .clicked()
                            {
                                curve_choice = Some(CurveChoice::Save);
                            }
                        });
                });
                // Lightroom's Refine Saturation, under the RGB point curve it acts on;
                // the channel curves keep the same height so nothing below moves.
                subheading(ui, "微调");
                // Camera Raw renders 101–200 as 100: shown there, and kept as imported
                // until the slider is moved.
                let mut shown = r.curve_saturation.min(1.);
                let before = shown;
                ui.add_enabled_ui(view.selected_curve == 0, |ui| {
                    setting_slider(ui, ParameterId::CurveSaturation, &mut shown, 1.);
                });
                if shown != before {
                    r.curve_saturation = shown;
                }
            }
            subheading(ui, "色阶");
            slider(
                ui,
                "黑场",
                &mut r.black_point,
                0. ..=r.white_point - 0.01,
                0.,
            );
            slider(
                ui,
                "白场",
                &mut r.white_point,
                r.black_point + 0.01..=1.,
                1.,
            );
            setting_slider(ui, ParameterId::Midtone, &mut r.midtone, 1.);
        }) {
            r.black_point = 0.;
            r.white_point = 1.;
            r.midtone = 1.;
            r.curve = Recipe::default().curve;
            r.effects.channels = std::array::from_fn(|_| Default::default());
            r.curve_saturation = 1.;
            r.effects.parametric = [0.; 4];
        }
        switch.finish(r);

        // The B&W panel replaces the Color Mixer whenever the photo renders black &
        // white, by its Treatment or by a black & white profile.
        let black_white = r.treatment() == Treatment::BlackWhite;
        let (mixer_title, mixer_panel) = if black_white {
            ("B&W", Panel::BlackWhiteMix)
        } else {
            ("颜色混合器", Panel::ColorMixer)
        };
        let mut switch = PanelSwitch::new(r, mixer_panel);
        if switched_section(ui, mixer_title, &mut switch.state, |ui| {
            if black_white {
                let heading = subheading(ui, "黑白混合");
                // Measured (once) only while this panel is open.
                let auto_mix = photo
                    .spread()
                    .zip(photo.metadata)
                    .map(|(spread, metadata)| {
                        crate::develop::AutoMix {
                            spread: &spread,
                            metadata,
                        }
                        .for_recipe(r)
                    });
                let button = Rect::from_min_size(
                    Pos2::new(heading.right() - 52., heading.top() - 4.),
                    Vec2::new(52., 24.),
                );
                // Greyed out while the mix is Auto's, as the Basic panel's Auto.
                let enabled = auto_mix.is_some_and(|mix| mix != r.effects.gray_mix);
                let auto = ui
                    .scope_builder(egui::UiBuilder::new().max_rect(button), |ui| {
                        ui.add_enabled(enabled, egui::Button::new("自动").small())
                    })
                    .inner;
                if auto
                    .on_hover_text("根据照片颜色设置混合")
                    .on_disabled_hover_text("已应用自动混合")
                    .clicked()
                {
                    auto_mix_request = true;
                }
                let target = Rect::from_min_size(
                    Pos2::new(button.left() - 28., button.top() + 1.),
                    Vec2::splat(22.),
                );
                if target_button(
                    ui,
                    target,
                    view.is(Tool::Targeted(Target::BlackWhite)),
                    &format!(
                        "目标调整：在照片上上下拖动，改变该颜色的明暗 · {}",
                        targeted_shortcut("G")
                    ),
                ) {
                    targeted_request = Some(Target::BlackWhite);
                }
                let moving = targeted.filter(|w| w.target == Target::BlackWhite);
                for (i, name) in BANDS.iter().enumerate() {
                    let row = ui.push_id(("bw", i), |ui| {
                        slider_with(
                            ui,
                            name,
                            &mut r.effects.gray_mix[i],
                            -1. ..=1.,
                            0.,
                            None,
                            Some((palette.gray(40), band_color(i))),
                        )
                    });
                    highlight_targeted(ui, row.response.rect, moving.map_or(0., |w| w.shares[i]));
                }
                return;
            }
            // Lightroom's tabs: the HSL and Color mixer, and Point Color.
            ui.horizontal(|ui| {
                let w = ui.available_width();
                segmented(
                    ui,
                    &mut view.mixer_tab,
                    &[
                        (MixerTab::Mixer, "混合器"),
                        (MixerTab::PointColor, "点颜色"),
                    ],
                    w,
                );
            });
            if view.mixer_tab == MixerTab::PointColor {
                // Only the current process renders Point Color.
                let supported = super::point_color_panel::renders_point_color(r);
                if let Some(steps) = super::point_color_panel::point_color_steps(r) {
                    hint_row(ui, &format!("请在校准中{steps}，以使用点颜色。"));
                }
                ui.add_enabled_ui(supported, |ui| {
                    super::point_color_panel::point_color_panel(ui, &mut r.point_colors, view)
                });
                return;
            }
            control_row(ui, "混合器", |ui| {
                let w = ui.available_width();
                segmented(
                    ui,
                    &mut view.mixer_color,
                    &[(false, "HSL"), (true, "彩色")],
                    w,
                );
            });
            if view.mixer_color {
                ui.horizontal(|ui| {
                    for (i, band) in BANDS.iter().enumerate() {
                        let (rect, response) =
                            ui.allocate_exact_size(Vec2::splat(27.), Sense::click());
                        ui.painter().circle_filled(rect.center(), 7., band_color(i));
                        if view.selected_band == i {
                            ui.painter().circle_stroke(
                                rect.center(),
                                10.,
                                Stroke::new(1.5, palette.gray(215)),
                            );
                        }
                        if response.on_hover_text(*band).clicked() {
                            view.selected_band = i;
                        }
                    }
                });
                let i = view.selected_band;
                ui.push_id(("hsl", i), |ui| {
                    for (c, name) in ["色相", "饱和度", "明亮度"].into_iter().enumerate() {
                        slider_with(
                            ui,
                            name,
                            &mut r.hsl[i][c],
                            -1. ..=1.,
                            0.,
                            None,
                            Some(hsl_gradient(&palette, i, c)),
                        );
                    }
                });
            } else {
                let all = view.mixer_adjust == 3;
                let channel_tip = |c: usize| {
                    let name = ["hue", "saturation", "luminance"][c];
                    format!(
                        "目标调整：在照片上上下拖动，改变该颜色的{name} · {}",
                        targeted_shortcut(["H", "S", "L"][c])
                    )
                };
                control_row(ui, "调整", |ui| {
                    ui.spacing_mut().item_spacing.x = 6.;
                    if !all {
                        let (button, _) =
                            ui.allocate_exact_size(Vec2::new(22., 22.), Sense::hover());
                        let target = hsl_target(view.mixer_adjust);
                        if target_button(
                            ui,
                            button,
                            view.is(Tool::Targeted(target)),
                            &channel_tip(view.mixer_adjust),
                        ) {
                            targeted_request = Some(target);
                        }
                    }
                    let w = ui.available_width();
                    segmented(
                        ui,
                        &mut view.mixer_adjust,
                        &[(0, "色相"), (1, "饱和度"), (2, "明亮度"), (3, "全部")],
                        w,
                    );
                });
                // Again after the tabs, which may have just changed.
                let all = view.mixer_adjust == 3;
                let channels = if all {
                    0..3
                } else {
                    view.mixer_adjust..view.mixer_adjust + 1
                };
                for c in channels {
                    let target = hsl_target(c);
                    if all {
                        let heading = subheading(ui, ["色相", "饱和度", "明亮度"][c]);
                        let button = Rect::from_min_size(
                            Pos2::new(heading.right() - 24., heading.top() - 3.),
                            Vec2::splat(22.),
                        );
                        if target_button(
                            ui,
                            button,
                            view.is(Tool::Targeted(target)),
                            &channel_tip(c),
                        ) {
                            targeted_request = Some(target);
                        }
                    }
                    let moving = targeted.filter(|w| w.target == target);
                    for (i, name) in BANDS.iter().enumerate() {
                        let row = ui.push_id(("hsl-all", c, i), |ui| {
                            slider_with(
                                ui,
                                name,
                                &mut r.hsl[i][c],
                                -1. ..=1.,
                                0.,
                                None,
                                Some(hsl_gradient(&palette, i, c)),
                            );
                        });
                        highlight_targeted(
                            ui,
                            row.response.rect,
                            moving.map_or(0., |w| w.shares[i]),
                        );
                    }
                }
            }
        }) {
            if black_white {
                r.effects.gray_mix = [0.; 8];
            } else {
                // Both tabs: the mixer and Point Color.
                r.hsl = [[0.; 3]; 8];
                r.point_colors.clear();
            }
        }
        switch.finish(r);

        let mut switch = PanelSwitch::new(r, Panel::ColorGrading);
        if switched_section(ui, "颜色分级", &mut switch.state, |ui| {
            ui.push_id(grading_document, |ui| {
                super::color_grading::color_grading_ui(ui, r, &mut view.grading);
            });
        }) {
            r.grading = [[0.; 3]; 3];
            r.effects.global_grade = [0.; 3];
            r.effects.balance = 0.;
            r.effects.blending = 0.5;
        }
        switch.finish(r);

        let mut switch = PanelSwitch::new(r, Panel::Detail);
        let sharpening = SharpeningSliders::defaults(r.sharpening_model);
        if switched_section(ui, "细节", &mut switch.state, |ui| {
            let radius_and_masking = r.engine >= 3;
            let color_default = if r.noise_model.is_original() {
                0.
            } else {
                0.25
            };
            let mut control = |ui: &mut egui::Ui, id: ParameterId, default: f32| {
                setting_slider(ui, id, id.value_mut(r), default);
            };
            subheading(ui, "锐化");
            control(ui, ParameterId::SharpeningAmount, sharpening.amount);
            if radius_and_masking {
                control(ui, ParameterId::SharpeningRadius, sharpening.radius);
                control(ui, ParameterId::SharpeningDetail, sharpening.detail);
                control(ui, ParameterId::SharpeningMasking, sharpening.masking);
            }
            subheading(ui, "降噪");
            control(ui, ParameterId::LuminanceNoise, 0.);
            ui.push_id("luma-nr", |ui| {
                control(ui, ParameterId::LuminanceDetail, 0.5);
                control(ui, ParameterId::LuminanceContrast, 0.);
            });
            control(ui, ParameterId::ColorNoise, color_default);
            ui.push_id("color-nr", |ui| {
                control(ui, ParameterId::ColorNoiseDetail, 0.5);
                control(ui, ParameterId::ColorNoiseSmoothness, 0.5);
            });
        }) {
            let d = Recipe::default().effects;
            r.noise_luma = 0.;
            r.effects.luma_detail = d.luma_detail;
            r.effects.luma_contrast = d.luma_contrast;
            // The oldest engines render only the original filter.
            r.set_color_noise_defaults(if r.engine >= 3 {
                crate::model::operators::NoiseModel::Measured
            } else {
                crate::model::operators::NoiseModel::Original
            });
            // Reset brings the current defaults, with the measured operator.
            r.set_sharpening_defaults(SharpeningModel::Measured);
        }
        switch.finish(r);

        let mut switch = PanelSwitch::new(r, Panel::LensCorrections);
        if switched_section(ui, "镜头校正", &mut switch.state, |ui| {
            let photo = metadata.as_ref();
            subheading(ui, "配置文件");
            ui.add_enabled_ui(r.engine >= 4, |ui| {
                control_row(ui, "", |ui| {
                    ui.checkbox(&mut r.lens_ca, "移除色差")
                        .on_hover_text("根据照片测量并移除边缘的红／青和蓝／黄色差。")
                        .on_disabled_hover_text("请在校准中更新处理版本以使用镜头校正。");
                });
                control_row(ui, "", |ui| {
                    let mut on = r.lens_profile;
                    if ui
                        .checkbox(&mut on, "启用配置文件校正")
                        .on_hover_text("使用 Adobe 镜头配置或 RAW 中保存的镜头数据校正畸变与暗角。")
                        .on_disabled_hover_text("请在校准中更新处理版本以使用镜头校正。")
                        .changed()
                        && let Some(m) = &metadata
                    {
                        let state = if on {
                            crate::model::recipe::ProfileCorrections::On
                        } else {
                            crate::model::recipe::ProfileCorrections::Off
                        };
                        r.set_profile_corrections(m, state);
                    }
                });
            });
            let lens = metadata
                .as_ref()
                .map(|m| m.lens_model.clone())
                .filter(|l| !l.is_empty());
            control_row(ui, "镜头", |ui| {
                ui.label(
                    egui::RichText::new(lens.as_deref().unwrap_or("未知"))
                        .size(11.)
                        .color(palette.gray(200)),
                );
            });
            lens_profile::profile_menus(ui, r, metadata.as_ref());
            // Said here as well as on import, so a missing profile is never silent.
            if let Some(missing) = metadata.as_ref().and_then(|m| r.missing_lens_profile(m)) {
                control_row(ui, "", |ui| {
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(missing)
                                .size(11.)
                                .color(palette.gray(150)),
                        )
                        .wrap(),
                    );
                });
            }
            if r.lens_profile {
                subheading(ui, "数量");
                ui.push_id("lens-amount", |ui| {
                    setting_control(ui, r, ParameterId::LensDistortion, 1., photo);
                    setting_control(ui, r, ParameterId::LensVignetting, 1., photo);
                });
            }
            control_row(ui, "", |ui| {
                if ui
                    .small_button("导入镜头配置…")
                    .on_hover_text("导入 Adobe .lcp 镜头配置，例如 /Library/Application Support/Adobe/CameraRaw/LensProfiles")
                    .clicked()
                {
                    import_lens = true;
                }
                if ui
                    .small_button("导入文件夹…")
                    .on_hover_text("导入文件夹及子文件夹内全部 .lcp 镜头配置")
                    .clicked()
                {
                    import_folder = Some(ImportKind::LensProfiles);
                }
            });
            // Lightroom's Manual tab: Distortion, then Defringe and Vignetting.
            subheading(ui, "畸变");
            ui.push_id("manual-distortion", |ui| {
                ui.add_enabled_ui(r.engine >= 4, |ui| {
                    setting_control(ui, r, ParameterId::ManualDistortion, 0., photo);
                });
            });
            let row = subheading(ui, "去边");
            // The Fringe Color Selector, at the row's far left as White Balance's.
            let rect = Rect::from_center_size(
                Pos2::new(row.left() + 13., row.center().y),
                Vec2::new(26., 20.),
            );
            let response = ui.interact(rect, ui.id().with("fringe-selector"), Sense::click());
            let picking = view.is(Tool::Defringe);
            if picking || response.hovered() {
                ui.painter()
                    .rect_filled(rect, 3., palette.gray(if picking { 72 } else { 50 }));
            }
            eyedropper_icon(ui.painter(), rect.center(), picking || response.hovered());
            if response
                .on_hover_text("去边颜色吸管：点击照片中的紫色或绿色边缘")
                .clicked()
            {
                view.toggle(Tool::Defringe);
            }
            defringe_sliders(ui, &mut r.effects);
            subheading(ui, "暗角");
            ui.push_id("lens-vignette", |ui| {
                setting_control(ui, r, ParameterId::LensVignetteAmount, 0., photo);
                setting_control(ui, r, ParameterId::LensVignetteMidpoint, 0.5, photo);
            });
        }) {
            if let Some(m) = &metadata {
                r.lens_builtin = m.lens.as_ref().is_none_or(|l| l.default_on);
            }
            let defaults = Recipe::default();
            r.lens_ca = defaults.lens_ca;
            r.lens_profile = defaults.lens_profile;
            r.lens_profile_choice = defaults.lens_profile_choice;
            r.lens_distortion = defaults.lens_distortion;
            r.lens_vignetting = defaults.lens_vignetting;
            r.lens_manual_distortion = defaults.lens_manual_distortion;
            let d = Recipe::default().effects;
            r.effects.defringe = d.defringe;
            r.effects.defringe_ranges = d.defringe_ranges;
            r.effects.lens_vignette = d.lens_vignette;
            r.effects.lens_vignette_midpoint = d.lens_vignette_midpoint;
        }
        switch.finish(r);

        let mut switch = PanelSwitch::new(r, Panel::Transform);
        if switched_section(ui, "变换", &mut switch.state, |ui| {
            let supported = r.engine >= 4;
            if !supported {
                hint_row(ui, "请在校准中更新处理版本以使用变换。");
            }
            ui.add_enabled_ui(supported, |ui| {
                use crate::model::transform::UprightMode;
                // As Lightroom: Update beside the heading, then the modes in two rows.
                control_row(ui, "透视校正", |ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        // Guided solves its guides again; without guides it has nothing to go by.
                        let analysed = match r.upright.mode {
                            UprightMode::Off => false,
                            UprightMode::Guided => !r.upright.guides.is_empty(),
                            _ => true,
                        };
                        if ui
                            .add_enabled(upright_ready && analysed, egui::Button::new("更新"))
                            .on_hover_text("重新分析照片，例如更改镜头校正后")
                            .clicked()
                        {
                            upright_request = true;
                        }
                    });
                });
                let u = &mut r.upright;
                let before = u.mode;
                for row in [
                    [
                        (UprightMode::Off, "关闭"),
                        (UprightMode::Auto, "自动"),
                        (UprightMode::Guided, "引导式"),
                    ],
                    [
                        (UprightMode::Level, "水平"),
                        (UprightMode::Vertical, "垂直"),
                        (UprightMode::Full, "全面"),
                    ],
                ] {
                    let w = ui.available_width();
                    segmented(ui, &mut u.mode, &row, w);
                }
                if u.mode != before {
                    if u.mode == UprightMode::Guided {
                        // As in Lightroom, choosing Guided picks up its tool.
                        guided_action = Some(GuidedAction::Choose);
                    } else if u.mode != UprightMode::Off && u.corrections.len() <= u.mode.code() {
                        upright_request = true;
                    }
                }
                if u.mode == UprightMode::Guided {
                    let count = u.guides.len();
                    control_row(ui, "引导线", |ui| {
                        let drawing = view.is(Tool::Guided);
                        if ui
                            .selectable_label(drawing, "绘制")
                            .on_hover_text("沿垂直及水平线绘制最多四条引导线 · Shift+T")
                            .clicked()
                        {
                            guided_action = Some(GuidedAction::Toggle);
                        }
                        ui.label(
                            egui::RichText::new(format!(
                                "{count} of {}",
                                crate::model::transform::MAX_GUIDES
                            ))
                            .color(palette.gray(170)),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui
                                .add_enabled(count > 0, egui::Button::new("清除"))
                                .on_hover_text("移除所有引导线")
                                .clicked()
                            {
                                guided_action = Some(GuidedAction::Clear);
                            }
                        });
                    });
                    if view.is(Tool::Guided) {
                        control_row(ui, "", |ui| {
                            ui.checkbox(&mut view.guided.loupe, "显示放大镜")
                                .on_hover_text("放置引导线端点时放大照片");
                            ui.checkbox(&mut view.guided.grid, "网格");
                        });
                    }
                } else if view.is(Tool::Guided) {
                    view.tool = Tool::None;
                }
                control_row(ui, "", |ui| {
                    ui.checkbox(&mut r.constrain_crop, "约束裁剪")
                        .on_hover_text("保持比例缩小裁剪，排除照片外的白色区域");
                });
                subheading(ui, "变换");
                // Stored as Camera Raw applies them, before the photo is turned for
                // display; shown, as in Lightroom, along the displayed photo's axes.
                let turns = metadata.as_ref().map_or(0, |m| {
                    crate::model::image_frame::ImageFrame::for_metadata(m).turns
                });
                let axes = crate::model::transform::display_axes(
                    (turns + r.rotation) % 4,
                    r.flip_x,
                    r.flip_y,
                );
                let mut shown = r.transform.displayed(axes);
                let t = &mut shown;
                setting_slider(ui, ParameterId::TransformVertical, &mut t.vertical, 0.);
                setting_slider(ui, ParameterId::TransformHorizontal, &mut t.horizontal, 0.);
                setting_slider(ui, ParameterId::TransformRotate, &mut t.rotate, 0.);
                setting_slider(ui, ParameterId::TransformAspect, &mut t.aspect, 0.);
                setting_slider(ui, ParameterId::TransformScale, &mut t.scale, 1.);
                setting_slider(ui, ParameterId::TransformOffsetX, &mut t.offset_x, 0.);
                setting_slider(ui, ParameterId::TransformOffsetY, &mut t.offset_y, 0.);
                if shown != r.transform.displayed(axes) {
                    r.transform = shown.recorded(axes);
                }
            });
        }) {
            r.transform = Default::default();
            r.upright = Default::default();
            r.constrain_crop = false;
        }
        switch.finish(r);

        let mut switch = PanelSwitch::new(r, Panel::Effects);
        if switched_section(ui, "效果", &mut switch.state, |ui| {
            let photo = metadata.as_ref();
            subheading(ui, "裁剪后暗角");
            setting_control(ui, r, ParameterId::VignetteAmount, 0., photo);
            setting_control(ui, r, ParameterId::VignetteMidpoint, 0.5, photo);
            setting_control(ui, r, ParameterId::VignetteFeather, 0.5, photo);
            subheading(ui, "颗粒");
            ui.push_id("grain", |ui| {
                setting_control(ui, r, ParameterId::GrainAmount, 0., photo);
                setting_control(ui, r, ParameterId::GrainSize, 0.25, photo);
                setting_control(ui, r, ParameterId::GrainRoughness, 0.5, photo);
            });
        }) {
            r.effects.reset_post_crop();
        }
        switch.finish(r);

        let mut switch = PanelSwitch::new(r, Panel::Calibration);
        if switched_section(ui, "校准", &mut switch.state, |ui| {
            subheading(ui, "处理版本");
            if r.engine < 4 {
                ui.horizontal(|ui| {
                    ui.small(format!("版本 {}（保留旧版渲染）", r.engine));
                    if ui
                        .small_button("更新")
                        .on_hover_text("使用当前处理版本渲染，包括内置镜头校正")
                        .clicked()
                    {
                        r.update_process(metadata.as_ref());
                    }
                });
            } else {
                ui.small("当前版本");
            }
            if r.profile.is_some() {
                ui.checkbox(&mut r.profile_tone, "配置文件色调渲染")
                    .on_hover_text("使用相机配置的基础色调曲线。启用前，旧编辑保留原有渲染。");
                if let Some(m) = &metadata {
                    let baseline = crate::camera_profiles::reference::baseline_exposure(m);
                    if baseline != r.camera_exposure
                        && ui
                            .small_button("使用相机曝光基准")
                            .on_hover_text("应用相机参考曝光偏移，不改变曝光滑块。")
                            .clicked()
                    {
                        r.camera_exposure = baseline;
                    }
                }
            }
            if ui.checkbox(&mut r.reference_curves, "参考色调曲线")
                .on_hover_text("自然三次曲线、保色相主曲线处理及 ProPhoto RGB 中的 sRGB 转换。启用前，旧编辑保留已保存的曲线行为。")
                .changed()
                && r.reference_curves
            {
                r.curve.natural = true;
                r.curve.smooth = true;
                for curve in &mut r.effects.channels {
                    curve.natural = true;
                    curve.smooth = true;
                }
            }
            if !r.reference_curves {
                ui.checkbox(&mut r.wide_gamut_curves, "旧版广色域曲线");
            }
            ui.checkbox(&mut r.reference_color, "参考色彩渲染")
                .on_hover_text("改进自然饱和度、颜色混合器明亮度及 RGB 色相分离色调。启用前，旧编辑保留原有渲染。");
            ui.checkbox(&mut r.reference_calibration, "参考校准")
                .on_hover_text(
                    "保留中性色的原色调整与改进的阴影色调。启用前，旧编辑保留原有行为。",
                );
            subheading(ui, "阴影");
            ui.push_id("calibration-shadows", |ui| {
                setting_slider(ui, ParameterId::ShadowTint, &mut r.effects.shadow_tint, 0.);
            });
            for (i, name) in ["红原色", "绿原色", "蓝原色"].iter().enumerate() {
                subheading(ui, name);
                ui.push_id(("calibration", i), |ui| {
                    let primary = ParameterId::PRIMARIES[i];
                    for id in primary {
                        setting_slider(ui, id, id.value_mut(r), 0.);
                    }
                });
            }
        }) {
            r.effects.calibration = [[0.; 2]; 3];
            r.effects.shadow_tint = 0.;
        }
        switch.finish(r);
        if let Some(kind) = auto_request {
            self.start_auto(kind);
        }
        if let Some(old) = profile_changed_from {
            self.follow_profile_treatment(old.as_deref());
        }
        if let Some(treatment) = treatment_request {
            self.set_treatment(treatment);
        }
        if auto_mix_request {
            self.auto_black_white_mix();
        }
        if let Some(choice) = curve_choice {
            self.choose_point_curve(choice);
        }
        if let Some(target) = targeted_request {
            self.toggle_targeted(target);
        }
        if upright_request {
            self.start_upright();
        }
        match guided_action {
            Some(GuidedAction::Choose) => {
                if self.view.is(Tool::Guided) {
                    self.choose_guided();
                } else {
                    self.toggle_guided_tool();
                }
            }
            Some(GuidedAction::Toggle) => self.toggle_guided_tool(),
            Some(GuidedAction::Clear) => {
                self.view.guided.selected = None;
                self.set_guides(Vec::new(), "清除引导线");
            }
            None => {}
        }
        if import_profiles {
            self.dialog(FileDialog::CameraProfile, &ui.ctx().clone());
        }
        if import_lens {
            self.dialog(FileDialog::LensProfile, &ui.ctx().clone());
        }
        if let Some(kind) = import_folder {
            self.dialog(FileDialog::ImportFolder(kind), &ui.ctx().clone());
        }
        if import_adobe && let Some(m) = self.document.metadata.clone() {
            if let Some(key) = adobe_key {
                ui.ctx()
                    .data_mut(|d| d.remove::<Vec<std::path::PathBuf>>(key));
            }
            match crate::camera_profiles::import_files(&adobe) {
                Ok(done) => {
                    let (profiles, errors) = crate::camera_profiles::installed(&m);
                    // As the loader's: the raw defaults are resolved again
                    // after this frame, so changing an unedited photo's recipe
                    // is not taken for an edit.
                    let _ = self.tx.send(super::worker::Event::Profiles {
                        id: self.load.id(),
                        profiles,
                        errors,
                    });
                    ui.ctx().request_repaint();
                    self.refresh_library_defaults();
                    self.status = format!(
                        "已为 {} 个 Adobe 配置导入 {} {} 的配置，请从配置文件菜单中选择。",
                        done.len(),
                        m.make,
                        m.model
                    );
                }
                Err(e) => self.status = format!("未导入配置文件：{e:#}"),
            }
        }
    }
}

/// A Crop panel button that acts once the panel is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CropAction {
    Swap,
    AutoStraighten,
}

/// The aspect menu's name for `aspect`: a preset either way round (X swaps them),
/// Original or Free, or Custom.
fn aspect_name(aspect: f32) -> &'static str {
    let near = |a: f32, b: f32| (a - b).abs() < 1e-4;
    ASPECTS
        .iter()
        .find(|(a, _)| near(*a, aspect) || (*a > 0. && near(1. / *a, aspect)))
        .map_or("自定义", |(_, name)| *name)
}

/// The crop's aspect presets, long side over short.
pub(super) const ASPECTS: [(f32, &str); 9] = [
    (-1., "原始"),
    (0., "自由"),
    (1., "1 x 1"),
    (1.25, "4 x 5"),
    (1.4, "5 x 7"),
    (1.5, "2 x 3"),
    (4. / 3., "3 x 4"),
    (16. / 9., "16 x 9"),
    (65. / 24., "65 x 24 (XPan)"),
];
pub(super) const BANDS: [&str; 8] = [
    "红色", "橙色", "黄色", "绿色", "青色", "蓝色", "紫色", "洋红",
];
fn band_color(i: usize) -> Color32 {
    [
        Color32::from_rgb(208, 69, 61),
        Color32::from_rgb(225, 137, 55),
        Color32::from_rgb(213, 193, 61),
        Color32::from_rgb(90, 160, 92),
        Color32::from_rgb(72, 178, 178),
        Color32::from_rgb(78, 125, 207),
        Color32::from_rgb(141, 103, 191),
        Color32::from_rgb(193, 92, 165),
    ][i]
}
/// Rail colors for a mixer slider: neighbouring hues for Hue, gray to color
/// for Saturation and dark to light for Luminance.
fn hsl_gradient(palette: &theme::Palette, band: usize, channel: usize) -> (Color32, Color32) {
    let color = band_color(band);
    match channel {
        0 => (band_color((band + 7) % 8), band_color((band + 1) % 8)),
        1 => (palette.gray(110), color),
        _ => (palette.gray(25), color.lerp_to_gamma(Color32::WHITE, 0.45)),
    }
}
/// A panel's switch while its section is drawn; a click on it is stored after.
pub(super) struct PanelSwitch {
    panel: Panel,
    pub(super) state: PanelState,
}
impl PanelSwitch {
    pub(super) fn new(r: &Recipe, panel: Panel) -> Self {
        Self {
            panel,
            state: r.panels.state(panel),
        }
    }
    pub(super) fn finish(self, r: &mut Recipe) {
        r.panels.set(self.panel, self.state);
    }
}
/// Marks a slider row a Targeted Adjustment Tool drag is moving, more strongly the
/// larger its `share` of the drag.
fn highlight_targeted(ui: &egui::Ui, row: Rect, share: f32) {
    if share <= 0. {
        return;
    }
    let painter = ui.painter();
    painter.rect_filled(row, 3., Color32::from_white_alpha((6. + 16. * share) as u8));
    painter.rect_filled(
        Rect::from_min_size(row.left_top(), Vec2::new(3., row.height())),
        1.5,
        theme::palette(ui.ctx())
            .gray(235)
            .gamma_multiply(0.4 + 0.6 * share),
    );
}
/// The Targeted Adjustment Tool's shortcut ending in `key`, as the tooltips show it.
fn targeted_shortcut(key: &str) -> String {
    if cfg!(target_os = "macos") {
        format!("⌘⌥⇧{key}")
    } else {
        format!("Ctrl+Alt+Shift+{key}")
    }
}
/// Group caption (Tone, Presence…) starting where the slider rails start.
fn subheading(ui: &mut egui::Ui, text: &str) -> Rect {
    super::widgets::set_edit_context(ui, text);
    ui.add_space(8.);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 16.), Sense::hover());
    ui.painter().text(
        rect.left_center() + Vec2::new(88., 0.),
        egui::Align2::LEFT_CENTER,
        text,
        egui::FontId::proportional(11.),
        theme::palette(ui.ctx()).gray(165),
    );
    rect
}
/// A labelled control row on the slider grid: caption right-aligned in the
/// 83 px label column, controls from the rail start (88 px) to the right edge,
/// and the same height as a slider row. Text too long for the row, such as a lens
/// or profile name, is cut short: wider rows push the panel past the window.
fn control_row<R>(ui: &mut egui::Ui, label: &str, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let (row, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 26.), Sense::hover());
    ui.painter().text(
        Pos2::new(row.left() + 78., row.center().y),
        egui::Align2::RIGHT_CENTER,
        label,
        egui::FontId::proportional(11.),
        theme::palette(ui.ctx()).gray(190),
    );
    let rect = Rect::from_min_max(Pos2::new(row.left() + 88., row.top()), row.max);
    ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(rect)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
        |ui| {
            ui.spacing_mut().item_spacing.x = 4.;
            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
            add(ui)
        },
    )
    .inner
}
/// Defringe's Amount and hue range for Purple, then Green. Double-clicking a hue
/// resets it to that color's own default range.
pub(super) fn defringe_sliders(ui: &mut egui::Ui, e: &mut crate::model::effects::Effects) {
    for (i, name, hue) in [(0, "紫色", [0.55, 0.9]), (1, "绿色", [0.2, 0.5])] {
        ui.push_id(("defringe", i), |ui| {
            slider_with(
                ui,
                &format!("{name}数量"),
                &mut e.defringe[i],
                0. ..=1.,
                0.,
                Some((20., 0)),
                None,
            );
            let defaults = crate::model::effects::DEFRINGE_RANGES[i];
            let [lo, hi] = &mut e.defringe_ranges[i];
            let colors = hue.map(|h| {
                let c = crate::develop::color::hue_rgb(h).map(|v| (v * 180.) as u8);
                Color32::from_rgb(c[0], c[1], c[2])
            });
            let low = slider_with(
                ui,
                &format!("{name}色相"),
                lo,
                0. ..=(*hi - 0.1).max(0.),
                defaults[0],
                None,
                Some((colors[0], colors[1])),
            );
            let high = ui
                .push_id("hi", |ui| {
                    slider_with(
                        ui,
                        "",
                        hi,
                        (*lo + 0.1).min(1.)..=1.,
                        defaults[1],
                        None,
                        Some((colors[0], colors[1])),
                    )
                })
                .inner;
            // One range, as in Lightroom: resetting either end resets both, so the
            // other end never keeps the default out of reach.
            if [low, high].contains(&SliderEvent::Reset) {
                [*lo, *hi] = defaults;
                // Named after the whole range, not the end the slider clamped.
                name_history_step(
                    ui,
                    format!("去边{name}色相"),
                    format!("{:.0} / {:.0}", defaults[0] * 100., defaults[1] * 100.),
                );
            }
        });
    }
}
fn eyedropper_icon(painter: &egui::Painter, c: Pos2, strong: bool) {
    let color = theme::palette(painter.ctx()).gray(if strong { 235 } else { 170 });
    icons::paint_at(painter, Icon::Eyedropper, c, 14., color);
}
/// The line that opens the Remove and Masking drawers: both tools are new.
/// What the experimental label's tooltip says about a tool.
const UNMEASURED: &str =
    "尚未与 Lightroom 对比验证。修复点和蒙版独立保存，旧版程序打开照片时不会应用它们。";
const RED_EYE_NOTE: &str =
    "通过模拟眼睛拟合 Camera Raw 红眼校正。校正独立保存，旧版程序打开照片时不会应用此校正。";
fn experimental(ui: &mut egui::Ui, note: &str) {
    ui.horizontal(|ui| {
        ui.add_space(83.);
        ui.label(
            egui::RichText::new("实验功能 · 早期版本")
                .size(10.)
                .color(theme::palette(ui.ctx()).warning()),
        )
        .on_hover_text(note);
    });
}
/// A circle with an arrow leaving it: the Remove tool.
fn heal_icon(painter: &egui::Painter, c: Pos2, strong: bool) {
    let stroke = Stroke::new(
        1.4,
        theme::palette(painter.ctx()).gray(if strong { 240 } else { 170 }),
    );
    painter.circle_stroke(c + Vec2::new(-2., 2.), 4.5, stroke);
    painter.line_segment([c + Vec2::new(1.5, -1.5), c + Vec2::new(6., -6.)], stroke);
    painter.line_segment([c + Vec2::new(6., -6.), c + Vec2::new(2.5, -6.)], stroke);
    painter.line_segment([c + Vec2::new(6., -6.), c + Vec2::new(6., -2.5)], stroke);
}
/// An eye: the Red Eye tool.
fn eye_icon(painter: &egui::Painter, c: Pos2, strong: bool) {
    let color = theme::palette(painter.ctx()).gray(if strong { 240 } else { 170 });
    let stroke = Stroke::new(1.4, color);
    let lid = |sign: f32| -> Vec<Pos2> {
        (0..=12)
            .map(|i| {
                let t = i as f32 / 12. * std::f32::consts::PI;
                c + Vec2::new(-6.5 * t.cos(), sign * 4. * t.sin())
            })
            .collect()
    };
    painter.add(egui::Shape::line(lid(1.), stroke));
    painter.add(egui::Shape::line(lid(-1.), stroke));
    painter.circle_filled(c, 2.2, color);
}
/// A dashed circle over a square: the Masking tool.
fn mask_icon(painter: &egui::Painter, c: Pos2, strong: bool) {
    let color = theme::palette(painter.ctx()).gray(if strong { 240 } else { 170 });
    let stroke = Stroke::new(1.4, color);
    painter.rect_stroke(
        Rect::from_center_size(c, Vec2::splat(12.)),
        1.,
        stroke,
        egui::StrokeKind::Inside,
    );
    painter.circle_filled(c + Vec2::new(1., 1.), 3.5, color);
}
fn crop_icon(painter: &egui::Painter, c: Pos2, strong: bool) {
    let color = theme::palette(painter.ctx()).gray(if strong { 240 } else { 170 });
    icons::paint_at(painter, Icon::Crop, c, 15., color);
}

/// A small note aligned with the slider rails.
fn hint_row(ui: &mut egui::Ui, text: &str) {
    ui.horizontal(|ui| {
        ui.add_space(88.);
        ui.label(
            egui::RichText::new(text)
                .size(11.)
                .color(theme::palette(ui.ctx()).gray(140)),
        );
    });
}

/// What the Transform panel asks of the Guided Upright tool.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GuidedAction {
    /// Guided was chosen among the modes.
    Choose,
    /// Draw: open or close the tool.
    Toggle,
    Clear,
}

/// The slider for develop setting `id`, and what changing it implies for the rest
/// of the recipe (`model::edit`).
fn setting_control(
    ui: &mut egui::Ui,
    r: &mut Recipe,
    id: ParameterId,
    default: f32,
    photo: Option<&crate::camera_data::Metadata>,
) {
    crate::model::edit::change_setting(r, id, photo, |r| {
        setting_slider(ui, id, id.value_mut(r), default)
    });
}
