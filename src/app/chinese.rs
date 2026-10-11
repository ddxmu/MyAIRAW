//! Chinese names for display-only facts returned by the unmodified photo model.
use std::collections::HashMap;
use std::sync::OnceLock;

pub(super) fn label(text: &str) -> &str {
    static LABELS: OnceLock<HashMap<String, String>> = OnceLock::new();
    LABELS
        .get_or_init(|| {
            serde_json::from_str(include_str!("../../assets/zh-CN.json"))
                .expect("reviewed Chinese labels")
        })
        .get(text)
        .map(String::as_str)
        .unwrap_or(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_labels_translate_without_changing_camera_or_file_names() {
        assert_eq!(label("Exposure"), "曝光");
        assert_eq!(label("White Balance"), "白平衡");
        assert_eq!(label("SONY ILCE-7M5"), "SONY ILCE-7M5");
        assert_eq!(label("旅行.rawmakase"), "旅行.rawmakase");
    }
}
