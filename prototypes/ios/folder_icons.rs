//! 文件夹统一使用现有的线性 SVG。保留旧索引里的标识，只换显示，不迁移或重写文档库。
use gpui::*;
use crate::store::Entry;

pub struct Choice { pub value: &'static str, pub glyph: &'static str, pub zh: &'static str, pub en: &'static str }
pub const CHOICES: [Choice; 18] = [
    Choice { value: "📁", glyph: "folder", zh: "文件夹", en: "Folder" },
    Choice { value: "📂", glyph: "folder-open", zh: "打开的文件夹", en: "Open folder" },
    Choice { value: "🗂️", glyph: "view-grid", zh: "分类", en: "Categories" },
    Choice { value: "📚", glyph: "library", zh: "书籍", en: "Books" },
    Choice { value: "📖", glyph: "file-text", zh: "阅读", en: "Reading" },
    Choice { value: "📝", glyph: "doc", zh: "笔记", en: "Notes" },
    Choice { value: "✏️", glyph: "pen-line", zh: "写作", en: "Writing" },
    Choice { value: "📌", glyph: "pin", zh: "置顶", en: "Pin" },
    Choice { value: "⭐", glyph: "star", zh: "收藏", en: "Star" },
    Choice { value: "🎯", glyph: "list-todo", zh: "任务", en: "Tasks" },
    Choice { value: "🚀", glyph: "code", zh: "代码", en: "Code" },
    Choice { value: "🧪", glyph: "braces", zh: "开发", en: "Development" },
    Choice { value: "🛠️", glyph: "settings-gear", zh: "工具", en: "Tools" },
    Choice { value: "💼", glyph: "storage", zh: "归档", en: "Archive" },
    Choice { value: "🏠", glyph: "home", zh: "生活", en: "Home" },
    Choice { value: "🎓", glyph: "sigma", zh: "学习", en: "Study" },
    Choice { value: "🎨", glyph: "palette", zh: "设计", en: "Design" },
    Choice { value: "🎵", glyph: "quote", zh: "摘录", en: "Quotes" },
];

pub fn name(value: Option<&str>) -> &'static str {
    CHOICES.iter().find(|i| Some(i.value) == value).map_or("folder", |i| i.glyph)
}
pub fn glyph(e: &Entry, size: f32) -> Svg { crate::ui::icon(name(e.meta.folder_icon.as_deref()), size, super::folder_tint(e)) }
