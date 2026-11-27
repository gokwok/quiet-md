//! `SPACE.md`：写给助手看的这个空间的说明（是什么、怎么写、要注意什么），一个纯文本文件，放在空间根目录、`space.json` 旁边，
//! 不是文档库里的文档。跟着空间的文件夹走（在 iCloud 里就一起同步）。还没有时第一次打开用一段很短的模板建出来。
use std::io;
use std::path::{Path, PathBuf};

use fdl_core::file_io;

pub const FILE: &str = "SPACE.md";

/// 新建时的模板。
pub const TEMPLATE: &str = "# 这个空间\n\n写给助手看：这个空间放的是什么、你希望它怎么帮你。\n\n## 内容\n\n- \n\n## 写作习惯\n\n- \n\n## 注意\n\n- \n";

pub fn path(root: &Path) -> PathBuf { root.join(FILE) }

/// 读出来；没有（或读不了）是 `None`。
pub fn read(root: &Path) -> Option<String> { file_io::read(path(root)).ok().map(|b| String::from_utf8_lossy(&b).into_owned()) }

pub fn write(root: &Path, text: &str) -> io::Result<()> { crate::tavern::library::write(&path(root), text.as_bytes()) }

/// 要改时打开：有就读，没有就用模板建一份再读。
pub fn open(root: &Path) -> io::Result<String> {
    if let Some(text) = read(root) { return Ok(text); }
    write(root, TEMPLATE)?;
    Ok(TEMPLATE.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn template_then_edit() {
        let root = std::env::temp_dir().join(format!("fdl-ai-space-md-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        assert!(read(&root).is_none());
        assert_eq!(open(&root).unwrap(), TEMPLATE);
        write(&root, "# 小说\n\n第三人称。\n").unwrap();
        assert_eq!(open(&root).unwrap(), "# 小说\n\n第三人称。\n");
        assert!(!root.join("SPACE.json.tmp").exists());
        std::fs::remove_dir_all(&root).unwrap();
    }
}
