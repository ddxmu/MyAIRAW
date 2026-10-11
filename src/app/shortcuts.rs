//! A reference card of every shortcut, grouped like Lightroom's: Cmd+/ (as in
//! Lightroom) or the keyboard
//! button in the top bar. Each shortcut's keys are drawn as key caps.
use super::Editor;
use super::icons::{self, Icon};
use super::theme;
use super::widgets::modal_frame;
use crate::app::Module;
use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, Sense, Stroke, Vec2};

/// A shortcut: its keys and what it does. Keys are written "Cmd+Shift+Z";
/// " / " separates alternatives, and a word in lower case ("click") is an
/// action rather than a key.
type Shortcut = (&'static str, &'static str);
/// A titled group of shortcuts.
type Group<'a> = (&'a str, &'a [Shortcut]);

const LIBRARY: [Shortcut; 9] = [
    ("方向键", "移动当前照片"),
    ("Home / End", "第一张／最后一张"),
    ("Shift+方向键 / Shift+点击", "扩展选区"),
    ("Cmd+点击", "添加或移除照片"),
    ("Cmd+A", "全选"),
    ("Cmd+D", "全不选"),
    ("/", "取消选择当前照片"),
    ("Cmd+L", "关闭／开启筛选"),
    ("J", "切换网格样式"),
];
const LOUPE: [Shortcut; 7] = [
    ("E / 回车 / 双击", "打开放大视图"),
    ("G / Esc / 双击", "返回网格"),
    ("Z / click", "适合窗口或上次缩放"),
    ("Cmd+= / Cmd+-", "放大／缩小"),
    ("Cmd+Alt+0", "100%"),
    ("drag", "平移"),
    ("I", "切换照片信息"),
];
const COMPARE: [Shortcut; 6] = [
    ("C", "比较当前与下一张照片"),
    ("左右", "更换候选照片"),
    ("上", "将候选设为选片"),
    ("下", "交换选片与候选"),
    ("click", "激活照片以使用评分键"),
    ("G / Esc", "返回网格"),
];
const SURVEY: [Shortcut; 5] = [
    ("N", "筛选所选照片"),
    ("方向键", "移动当前照片"),
    ("click", "激活照片以使用评分键"),
    ("Cmd+点击", "移除照片"),
    ("G / Esc", "返回网格"),
];
const RATING: [Shortcut; 8] = [
    ("0 / 1 / 2 / 3 / 4 / 5", "星级评分"),
    ("[ / ]", "降低／提高评分"),
    ("6 / 7 / 8 / 9", "红、黄、绿、蓝"),
    ("P / X / U", "选中／拒绝／清除旗标"),
    ("`", "切换选中状态"),
    ("Cmd+Up / Cmd+Down", "提高／降低旗标"),
    ("Shift+按键", "应用并切换到下一张"),
    ("Cmd+Z / Cmd+Shift+Z", "全局撤销／重做"),
];
const QUICK: [Shortcut; 3] = [
    ("B", "添加或移除"),
    ("Cmd+B", "显示"),
    ("Cmd+Shift+B", "清除"),
];
const GENERAL: [Shortcut; 8] = [
    ("G", "照片库"),
    ("D", "显影"),
    ("Tab", "隐藏／显示侧边面板"),
    ("Shift+Tab", "隐藏／显示侧边面板与胶片栏"),
    ("F7 / F8 / F6", "左面板／右面板／胶片栏"),
    ("Cmd+'", "创建虚拟副本"),
    ("Cmd+,", "偏好设置"),
    ("Cmd+/", "这些快捷键"),
];
const DEVELOP: [Shortcut; 47] = [
    ("R", "裁剪与拉直"),
    ("回车", "完成裁剪"),
    ("X", "交换裁剪方向"),
    ("O / Shift+O", "下一个裁剪参考线／旋转"),
    ("Cmd+拖动", "沿线拉直"),
    ("Shift+T", "引导式透视：绘制引导线"),
    ("删除", "删除所选污点、引导线或红眼"),
    ("Q", "污点移除"),
    ("Shift+W", "蒙版"),
    ("W", "白平衡吸管"),
    ("Cmd+Option+Shift+T", "目标调整：色调曲线"),
    (
        "Cmd+Option+Shift+H / S / L",
        "目标调整：色相／饱和度／明亮度",
    ),
    ("Cmd+Option+Shift+G", "目标调整：黑白混合"),
    ("Esc", "关闭当前工具"),
    ("[ / ]", "画笔／红眼大小（Shift：羽化）"),
    ("滚动", "照片上的画笔／红眼大小（Shift：羽化）"),
    ("/", "为所选污点选新来源"),
    ("H", "隐藏污点图钉"),
    ("A", "显示污点"),
    ("空格", "工具开启时平移"),
    ("\\", "仅编辑前"),
    ("Y", "前后左右对比"),
    ("Option+Y", "前后上下对比"),
    ("Shift+Y", "前后分割对比"),
    ("Cmd+Option+Shift+→ / ←", "复制编辑前到后／后到前"),
    ("Cmd+Option+Shift+↑", "交换编辑前后"),
    ("Shift+R", "参考视图"),
    ("I", "照片信息叠加：信息 1、信息 2、关闭"),
    ("J", "显示阴影与高光剪裁"),
    ("Z / F", "切换缩放／适合"),
    ("左右", "上一张／下一张照片"),
    ("Cmd+Shift+C / Cmd+Shift+V", "复制／粘贴设置"),
    ("Cmd+Option+V", "粘贴上一张照片的设置"),
    ("Cmd+Shift+S", "将设置同步到所选照片"),
    ("Cmd+Option+Shift+M", "匹配总曝光"),
    ("Cmd+Shift+N", "新建显影预设"),
    ("Cmd+Shift+R", "重置所有设置"),
    ("Cmd+Shift+U", "自动色调"),
    ("V", "切换黑白／彩色"),
    ("Cmd+Shift+E", "导出…"),
    ("Cmd+Alt+Shift+E", "使用上次设置导出"),
    ("double-click", "重置滑块或颜色分级轮"),
    ("Shift+拖动", "颜色分级轮：仅色相或饱和度"),
    ("Cmd+拖动", "颜色分级轮：精细调整"),
    ("上 / 下", "微调悬停滑块（Shift：十步）"),
    ("right-click", "面板标题：单独模式"),
    ("E", "在放大视图中显示照片"),
];

const WIDTH: f32 = 820.;
const COLUMN: f32 = 380.;
const KEYS: f32 = 165.;
const ROW: f32 = 26.;

impl Editor {
    pub(super) fn shortcuts_window(&mut self, ctx: &egui::Context) {
        let palette = theme::palette(ctx);
        if !self.view.shortcuts {
            return;
        }
        // Cmd+/ closes it again; other keys stay with the sheet.
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::Slash)) {
            self.view.shortcuts = false;
            return;
        }
        let views: [Group; 4] = [
            ("照片库网格", &LIBRARY),
            ("放大视图", &LOUPE),
            ("比较", &COMPARE),
            ("筛选视图", &SURVEY),
        ];
        let common: [Group; 3] = [
            ("评分、旗标与标签", &RATING),
            ("快速收藏集", &QUICK),
            ("全局", &GENERAL),
        ];
        let develop: [Group; 1] = [("显影", &DEVELOP)];
        // The current module's shortcuts come first, in the left column.
        let (left, right): (Vec<Group>, Vec<Group>) = if self.module == Module::Library {
            (views.to_vec(), [&common[..], &develop].concat())
        } else {
            ([&develop[..], &common].concat(), views.to_vec())
        };
        let (left, right) = (left.as_slice(), right.as_slice());
        let response = egui::Modal::new(egui::Id::new("keyboard-shortcuts"))
            .backdrop_color(Color32::from_black_alpha(140))
            .frame(modal_frame(&palette).inner_margin(egui::Margin::symmetric(28, 22)))
            .show(ctx, |ui| {
                ui.set_width(WIDTH);
                let mut close = false;
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new("键盘快捷键")
                            .size(17.)
                            .color(palette.gray(235)),
                    );
                    ui.add_space(10.);
                    ui.label(
                        egui::RichText::new(format!("{} 随时显示此帮助", keys_text("Cmd+/")))
                            .size(12.)
                            .color(palette.gray(130)),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        close = icon_button(ui, Icon::Close, "关闭 · Esc").clicked();
                    });
                });
                ui.add_space(14.);
                let height = (ctx.content_rect().height() * 0.8 - 90.).max(200.);
                egui::ScrollArea::vertical()
                    .max_height(height)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        ui.horizontal_top(|ui| {
                            ui.spacing_mut().item_spacing.x = WIDTH - 2. * COLUMN;
                            for column in [left, right] {
                                ui.allocate_ui(Vec2::new(COLUMN, 0.), |ui| {
                                    ui.vertical(|ui| {
                                        for (title, shortcuts) in column {
                                            group(ui, title, shortcuts);
                                        }
                                    });
                                });
                            }
                        });
                    });
                close
            });
        if response.inner || response.should_close() {
            self.view.shortcuts = false;
        }
    }
}

fn group(ui: &mut egui::Ui, title: &str, shortcuts: &[Shortcut]) {
    let palette = theme::palette(ui.ctx());
    ui.label(
        egui::RichText::new(title.to_uppercase())
            .size(10.5)
            .color(palette.gray(125)),
    );
    ui.add_space(6.);
    for (keys, action) in shortcuts {
        let (rect, _) = ui.allocate_exact_size(Vec2::new(COLUMN, ROW), Sense::hover());
        // Key caps right-aligned in their column, the action after them.
        let mut x = rect.left() + KEYS;
        for (i, alternative) in keys
            .split(" / ")
            .enumerate()
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
        {
            for cap in alternative.split('+').collect::<Vec<_>>().into_iter().rev() {
                x = key_cap(ui, cap, Pos2::new(x, rect.center().y)) - 3.;
            }
            if i > 0 {
                x -= 2.;
                ui.painter().text(
                    Pos2::new(x, rect.center().y),
                    Align2::RIGHT_CENTER,
                    "/",
                    FontId::proportional(11.),
                    palette.gray(100),
                );
                x -= 10.;
            }
        }
        ui.painter().text(
            Pos2::new(rect.left() + KEYS + 14., rect.center().y),
            Align2::LEFT_CENTER,
            *action,
            FontId::proportional(12.5),
            palette.gray(205),
        );
    }
    ui.add_space(16.);
}

/// Draws one key cap ending at `right`; returns its left edge. A word in
/// lower case is an action, drawn as plain text.
fn key_cap(ui: &egui::Ui, cap: &str, right: Pos2) -> f32 {
    let palette = theme::palette(ui.ctx());
    let label = cap_label(cap);
    let action = cap.chars().all(|c| c.is_lowercase() || c == '-') && cap.len() > 1;
    let galley = ui.painter().layout_no_wrap(
        label,
        FontId::proportional(if action { 11.5 } else { 11. }),
        palette.gray(if action { 150 } else { 230 }),
    );
    if action {
        let left = right.x - galley.size().x;
        ui.painter().galley(
            Pos2::new(left, right.y - galley.size().y / 2.),
            galley,
            Color32::WHITE,
        );
        return left;
    }
    let width = (galley.size().x + 10.).max(20.);
    let cap_rect = Rect::from_min_max(
        Pos2::new(right.x - width, right.y - 10.),
        Pos2::new(right.x, right.y + 10.),
    );
    ui.painter().rect(
        cap_rect,
        4.,
        palette.gray(46),
        Stroke::new(1., palette.gray(62)),
        egui::StrokeKind::Inside,
    );
    ui.painter().galley(
        cap_rect.center() - galley.size() / 2.,
        galley,
        Color32::WHITE,
    );
    cap_rect.left()
}

/// A small borderless icon button, as in the top bar.
pub(super) fn icon_button(ui: &mut egui::Ui, icon: Icon, hover: &str) -> egui::Response {
    let palette = theme::palette(ui.ctx());
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(26.), Sense::click());
    if response.hovered() {
        ui.painter().rect_filled(rect, 4., palette.gray(45));
    }
    let color = palette.gray(if response.hovered() { 235 } else { 160 });
    icons::paint_at(ui.painter(), icon, rect.center(), 15., color);
    response
        .on_hover_text(hover)
        .on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A key as the platform names it: ⌘ ⌥ ⇧ on macOS.
fn cap_label(cap: &str) -> String {
    let mac = cfg!(target_os = "macos");
    match cap {
        "Cmd" if mac => "⌘".into(),
        "Cmd" => "Ctrl".into(),
        "Alt" if mac => "⌥".into(),
        "Shift" if mac => "⇧".into(),
        "上" => "↑".into(),
        "下" => "↓".into(),
        "左" => "←".into(),
        "右" => "→".into(),
        "方向键" => "← ↑ → ↓".into(),
        "回车" if mac => "↩".into(),
        other => other.into(),
    }
}

/// Keys as text, e.g. for a hover: "⌘/".
pub(super) fn keys_text(keys: &str) -> String {
    keys.split('+').map(cap_label).collect()
}
