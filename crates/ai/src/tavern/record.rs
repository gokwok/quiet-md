//! 一次生成的附属记录：文档库的 `assets/gen/<标识>.json`，文档里对应的文字带 `<span gen="标识">`。
//! 记下当时的要求和原文，下次拼装还原历史时用（见 `history`）；读的时候宽松，缺字段按空的算。
use std::path::{Path, PathBuf};

use serde_json::{Map, Value, json};

use super::preset::{num, text};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Record {
    pub id: String,
    /// 这次的要求，已经跑过「存下来时」的正则和宏（和 SillyTavern 存进聊天的一样）。
    pub request: String,
    /// 模型返回的原文（思考、标签都还在）。已经跑过「存下来时」的正则。
    pub raw: String,
    /// 写进文档时这次生成各块的全文（`Document::gen_text`）的哈希；对不上就是被改过。
    pub hash: String,
    pub preset: String,
    pub char: String,
    pub persona: String,
    pub model: String,
    /// 发出去的参数（`temperature` 这些），原样记下。
    pub params: Map<String, Value>,
    /// Unix 秒。
    pub created: i64,
}

/// 记录所在的目录：文档库的 `assets/gen`。
pub fn dir() -> PathBuf { fdl_core::store::Store::assets_dir().join("gen") }

/// 标识来自文档，只认字母数字和连字符，免得拼出别处的路径。
pub fn valid_id(id: &str) -> bool { !id.is_empty() && id.len() <= 64 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') }

/// 文字的哈希（FNV-1a 64 位，十六进制）。只用来判断改没改过，不防篡改。
pub fn hash(s: &str) -> String {
    let h = s.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ b as u64).wrapping_mul(0x0000_0100_0000_01b3));
    format!("{h:016x}")
}

impl Record {
    pub fn read(id: &str, v: &Value) -> Record {
        let t = |k: &str| text(v, k).unwrap_or_default();
        Record {
            id: id.to_owned(), request: t("request"), raw: t("raw"), hash: t("hash"), preset: t("preset"), char: t("char"), persona: t("persona"), model: t("model"),
            params: v.get("params").and_then(Value::as_object).cloned().unwrap_or_default(),
            created: num(v, "created").map_or(0, |n| n as i64),
        }
    }

    pub fn to_json(&self) -> Value {
        json!({
            "request": self.request, "raw": self.raw, "hash": self.hash, "preset": self.preset, "char": self.char, "persona": self.persona,
            "model": self.model, "params": self.params, "created": self.created,
        })
    }

    /// 从 `dir` 里读；没有、读不了、不是 JSON 对象都算没有。
    pub fn load(dir: &Path, id: &str) -> Option<Record> {
        if !valid_id(id) { return None; }
        let v: Value = serde_json::from_slice(&std::fs::read(dir.join(format!("{id}.json"))).ok()?).ok()?;
        v.is_object().then(|| Record::read(id, &v))
    }

    /// 写进 `dir`：先写临时文件再改名，iCloud 和中途退出都不会留下半个文件。
    pub fn save(&self, dir: &Path) -> std::io::Result<()> {
        if !valid_id(&self.id) { return Err(std::io::Error::other("invalid generation id")); }
        std::fs::create_dir_all(dir)?;
        let tmp = dir.join(format!(".{}.json.tmp", self.id));
        std::fs::write(&tmp, serde_json::to_vec_pretty(&self.to_json()).map_err(std::io::Error::other)?)?;
        std::fs::rename(&tmp, dir.join(format!("{}.json", self.id)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saves_loads_and_reads_leniently() {
        let dir = std::env::temp_dir().join(format!("fdl-ai-record-{}", std::process::id()));
        let mut params = Map::new();
        params.insert("temperature".into(), json!(1.0));
        let r = Record { id: "ab12-cd".into(), request: "接着写".into(), raw: "<think>x</think>正文".into(), hash: hash("正文"), preset: "p".into(), model: "m".into(), params, created: 1_800_000_000, ..Default::default() };
        r.save(&dir).unwrap();
        assert_eq!(Record::load(&dir, "ab12-cd"), Some(r));
        assert_eq!(Record::load(&dir, "../x"), None);
        assert!(Record { id: "a/b".into(), ..Default::default() }.save(&dir).is_err());
        std::fs::write(dir.join("old.json"), r#"{"raw": "只有原文", "created": "12"}"#).unwrap();
        let old = Record::load(&dir, "old").unwrap();
        assert_eq!((old.raw.as_str(), old.request.as_str(), old.created), ("只有原文", "", 12));
        std::fs::remove_dir_all(&dir).unwrap();
        assert_ne!(hash("a"), hash("b"));
        assert_eq!(hash(""), "cbf29ce484222325");
    }
}
