//! 文件夹统一使用现有的线性 SVG。保留旧索引里的标识，只换显示，不迁移或重写文档库。
//! 图标名不带目录时在 `icons/`（18 格、1.5 描边）；带目录的取编辑器那一套（`editor-icons/`，24 格、2 描边，
//! 画到 20 点左右时线宽和前者差不多）——它们已经在项目里，不另占 Nucleo 的名额。
use gpui::*;
use crate::store::Entry;

pub struct Choice { pub value: &'static str, pub glyph: &'static str, pub zh: &'static str, pub en: &'static str }
pub const CHOICES: [Choice; 48] = [
    Choice { value: "📁", glyph: "folder", zh: "文件夹", en: "Folder" },
    Choice { value: "📂", glyph: "folder-open", zh: "打开的文件夹", en: "Open folder" },
    Choice { value: "🗂️", glyph: "view-grid", zh: "分类", en: "Categories" },
    Choice { value: "📚", glyph: "library", zh: "书籍", en: "Books" },
    Choice { value: "📖", glyph: "book-open", zh: "阅读", en: "Reading" },
    Choice { value: "📝", glyph: "doc", zh: "笔记", en: "Notes" },
    Choice { value: "✏️", glyph: "pen-line", zh: "写作", en: "Writing" },
    Choice { value: "📌", glyph: "pin", zh: "置顶", en: "Pin" },
    Choice { value: "⭐", glyph: "star", zh: "收藏", en: "Star" },
    Choice { value: "🎯", glyph: "list-todo", zh: "任务", en: "Tasks" },
    Choice { value: "🚀", glyph: "code", zh: "代码", en: "Code" },
    Choice { value: "🧪", glyph: "flask", zh: "实验", en: "Lab" },
    Choice { value: "🛠️", glyph: "settings-gear", zh: "工具", en: "Tools" },
    Choice { value: "💼", glyph: "storage", zh: "归档", en: "Archive" },
    Choice { value: "🏠", glyph: "home", zh: "生活", en: "Home" },
    Choice { value: "🎓", glyph: "sigma", zh: "学习", en: "Study" },
    Choice { value: "🎨", glyph: "palette", zh: "设计", en: "Design" },
    Choice { value: "🎵", glyph: "quote", zh: "摘录", en: "Quotes" },
    // 生活
    Choice { value: "❤️", glyph: "heart", zh: "喜爱", en: "Loved" },
    Choice { value: "👔", glyph: "briefcase", zh: "工作", en: "Work" },
    Choice { value: "💰", glyph: "wallet", zh: "财务", en: "Finance" },
    Choice { value: "🛒", glyph: "cart", zh: "购物", en: "Shopping" },
    Choice { value: "✈️", glyph: "plane", zh: "旅行", en: "Travel" },
    Choice { value: "🚗", glyph: "editor-icons/emoji-places", zh: "出行", en: "Transport" },
    Choice { value: "🍎", glyph: "editor-icons/emoji-foods", zh: "饮食", en: "Food" },
    Choice { value: "☕", glyph: "coffee", zh: "咖啡", en: "Coffee" },
    Choice { value: "🐶", glyph: "editor-icons/emoji-nature", zh: "宠物", en: "Pets" },
    Choice { value: "🌿", glyph: "leaf", zh: "自然", en: "Nature" },
    Choice { value: "⚽", glyph: "editor-icons/emoji-activity", zh: "运动", en: "Sports" },
    Choice { value: "💓", glyph: "pulse", zh: "健康", en: "Health" },
    // 兴趣与灵感
    Choice { value: "🎶", glyph: "music", zh: "音乐", en: "Music" },
    Choice { value: "🎬", glyph: "editor-icons/circle-play", zh: "视频", en: "Video" },
    Choice { value: "🖼️", glyph: "editor-icons/image", zh: "图片", en: "Images" },
    Choice { value: "💡", glyph: "editor-icons/emoji-objects", zh: "想法", en: "Ideas" },
    Choice { value: "😀", glyph: "editor-icons/smile", zh: "心情", en: "Mood" },
    Choice { value: "🎁", glyph: "gift", zh: "礼物", en: "Gifts" },
    // 安排与资料
    Choice { value: "📅", glyph: "editor-icons/calendar", zh: "日程", en: "Calendar" },
    Choice { value: "⏰", glyph: "editor-icons/alarm-clock", zh: "提醒", en: "Reminders" },
    Choice { value: "🚩", glyph: "flag", zh: "目标", en: "Goals" },
    Choice { value: "📈", glyph: "chart", zh: "数据", en: "Data" },
    Choice { value: "✉️", glyph: "mail", zh: "邮件", en: "Mail" },
    Choice { value: "💬", glyph: "chat", zh: "对话", en: "Chats" },
    Choice { value: "🌐", glyph: "editor-icons/globe", zh: "网络", en: "Web" },
    Choice { value: "🔗", glyph: "editor-icons/link", zh: "链接", en: "Links" },
    Choice { value: "📎", glyph: "editor-icons/paperclip", zh: "附件", en: "Attachments" },
    Choice { value: "🔒", glyph: "lock", zh: "私密", en: "Private" },
    Choice { value: "☁️", glyph: "cloud", zh: "云端", en: "Cloud" },
    Choice { value: "🌙", glyph: "moon", zh: "日记", en: "Journal" },
];

pub fn name(value: Option<&str>) -> &'static str {
    CHOICES.iter().find(|i| Some(i.value) == value).map_or("folder", |i| i.glyph)
}
pub fn glyph(e: &Entry, size: f32) -> Svg { icon(name(e.meta.folder_icon.as_deref()), size, super::folder_tint(e)) }
/// 画一个文件夹图标（`name` 可以带目录，见上）。
pub fn icon(name: &str, size: f32, color: Hsla) -> Svg {
    if name.contains('/') { svg().path(SharedString::from(format!("{name}.svg"))).size(px(size)).flex_none().text_color(color) } else { crate::ui::icon(name, size, color) }
}
