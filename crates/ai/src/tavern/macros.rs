//! 宏：照 SillyTavern 1.18 起默认的宏引擎（`macros/engine`）。
//! 从左往右一个个求值，参数里可以套宏（先求参数）；参数两边的空白去掉；参数个数不对的、不认识的原样留着
//! （里面套的宏照样求值），名字记进 `unknown` 给诊断用。名字不分大小写。
//! 收尾两步和它一样：`{{trim}}` 连同两边的换行一起去掉，`\{`、`\}` 还原成花括号。
//! 变量只在一次拼装里有效：同一个 `Macros` 从头用到尾，前面的提示词设的、后面的取得到。
use std::collections::{BTreeSet, HashMap};
use std::sync::OnceLock;

use fancy_regex::Regex;

/// 宏能读到的东西。时间由调用方给（本地时间的年、月、日、时、分），这样测试是确定的。
#[derive(Clone, Debug, Default)]
pub struct Env {
    pub user: String,
    pub char: String,
    pub description: String,
    pub personality: String,
    pub scenario: String,
    pub persona: String,
    pub model: String,
    /// 输入框里的文字（`{{input}}`）。SillyTavern 拼装时输入框已经清空，所以拼装时给空的。
    pub input: String,
    pub last_message: String,
    pub last_user_message: String,
    pub last_char_message: String,
    pub max_context: u64,
    pub max_response: u64,
    pub now: Option<(i64, u32, u32, u32, u32)>,
    /// `{{random}}` 的种子；`{{pick}}` 再混上这段文字和位置，同样的输入挑同样的项。
    pub seed: u64,
}

/// JS 把数字转成字符串的样子：整数不带小数点。
fn js_number(n: f64) -> String { if n.fract() == 0. && n.abs() < 1e15 { format!("{}", n as i64) } else { format!("{n}") } }

/// `{{random}}`、`{{pick}}` 的列表：多个参数就是各项；一个参数按逗号分（`\,` 是逗号本身），去掉两边空白。
fn choices(args: &[String]) -> Vec<String> {
    if args.len() != 1 { return args.to_vec(); }
    args[0].replace("\\,", "\u{0}").split(',').map(|s| s.trim().replace('\u{0}', ",")).collect()
}

fn mix(mut x: u64) -> u64 {
    x ^= x >> 33; x = x.wrapping_mul(0xff51_afd7_ed55_8ccd); x ^= x >> 33; x = x.wrapping_mul(0xc4ce_b9fe_1a85_ec53); x ^ (x >> 33)
}

/// 从 `at`（指着 `{{`）往后找配对的 `}}`，中间套的 `{{…}}` 跳过。返回 `}}` 的位置。
fn closing(text: &str, at: usize) -> Option<usize> {
    let b = text.as_bytes();
    let (mut i, mut depth) = (at + 2, 0usize);
    while i + 1 < b.len() {
        if b[i] == b'{' && b[i + 1] == b'{' { depth += 1; i += 2; } else if b[i] == b'}' && b[i + 1] == b'}' {
            if depth == 0 { return Some(i); }
            depth -= 1;
            i += 2;
        } else { i += 1; }
    }
    None
}

/// 按最外层的 `::` 切开（套着的宏里的 `::` 不算）。
fn split_args(inner: &str) -> Vec<&str> {
    let b = inner.as_bytes();
    let (mut parts, mut start, mut depth, mut i) = (vec![], 0, 0usize, 0);
    while i < b.len() {
        let pair = |c: u8| b[i] == c && b.get(i + 1) == Some(&c);
        if pair(b'{') { depth += 1; i += 2; } else if pair(b'}') { depth = depth.saturating_sub(1); i += 2; } else if depth == 0 && pair(b':') {
            parts.push(&inner[start..i]);
            i += 2;
            start = i;
        } else { i += 1; }
    }
    parts.push(&inner[start..]);
    parts
}

fn identifier(s: &str) -> bool {
    let mut c = s.chars();
    c.next().is_some_and(|f| f.is_ascii_alphabetic() || f == '_') && c.all(|x| x.is_ascii_alphanumeric() || x == '_' || x == '-')
}

/// 一个 `{{…}}` 求出来的结果。
enum Out { Value(String), Keep(String), NotMacro }

#[derive(Clone)]
pub struct Macros {
    pub env: Env,
    vars: HashMap<String, String>,
    globals: HashMap<String, String>,
    rng: u64,
    /// 遇到了、但不认识（或参数个数不对）的宏名。
    pub unknown: BTreeSet<String>,
}

impl Macros {
    pub fn new(env: Env) -> Macros {
        let rng = mix(env.seed ^ 0x9e37_79b9_7f4a_7c15);
        Macros { env, vars: HashMap::new(), globals: HashMap::new(), rng, unknown: BTreeSet::new() }
    }

    pub fn var(&self, name: &str) -> Option<&str> { self.vars.get(name).map(String::as_str) }

    pub fn eval(&mut self, text: &str) -> String { self.eval_with(text, &|s| s.to_owned()) }

    /// 每个宏求出来的值再经过 `post`（正则查找串里替换宏时用它转义）。
    pub fn eval_with(&mut self, text: &str, post: &dyn Fn(&str) -> String) -> String {
        static MARKERS: OnceLock<[(Regex, &str); 5]> = OnceLock::new();
        static TRIM: OnceLock<Regex> = OnceLock::new();
        static BRACE: OnceLock<Regex> = OnceLock::new();
        // 老写法 `<USER>` 这些先改成宏。
        let markers = MARKERS.get_or_init(|| [("<USER>", "{{user}}"), ("<BOT>", "{{char}}"), ("<CHAR>", "{{char}}"), ("<GROUP>", "{{group}}"), ("<CHARIFNOTGROUP>", "{{charIfNotGroup}}")]
            .map(|(p, r)| (Regex::new(&format!("(?i){p}")).unwrap(), r)));
        let mut s = text.to_owned();
        for (re, to) in markers { if s.contains('<') { s = re.replace_all(&s, *to).into_owned(); } }
        if !s.contains("{{") && !s.contains('\\') { return s; }
        let s = self.walk(&s, Some(post), 0);
        let s = TRIM.get_or_init(|| Regex::new(r"(?i)(?:\r?\n)*\{\{trim\}\}(?:\r?\n)*").unwrap()).replace_all(&s, "");
        BRACE.get_or_init(|| Regex::new(r"\\([{}])").unwrap()).replace_all(&s, "$1").into_owned()
    }

    fn walk(&mut self, text: &str, post: Option<&dyn Fn(&str) -> String>, depth: usize) -> String {
        let mut out = String::with_capacity(text.len());
        let (mut i, mut lit) = (0, 0);
        while let Some(off) = text[i..].find("{{") {
            let at = i + off;
            let Some(end) = closing(text, at) else { break };
            match self.call(&text[at + 2..end], at, depth) {
                Out::NotMacro => { i = at + 1; continue }
                Out::Value(v) => { out.push_str(&text[lit..at]); out.push_str(&post.map_or_else(|| v.clone(), |p| p(&v))); }
                Out::Keep(k) => { out.push_str(&text[lit..at]); out.push_str(&k); }
            }
            i = end + 2;
            lit = i;
        }
        out.push_str(&text[lit..]);
        out
    }

    fn call(&mut self, inner: &str, at: usize, depth: usize) -> Out {
        // 注释：里面套的宏照样求值（setvar 生效），整段换成空的。
        if inner.trim_start().starts_with("//") { self.walk(inner, None, depth + 1); return Out::Value(String::new()); }
        let parts = split_args(inner);
        let head = parts[0].trim();
        // 老写法：`{{reverse:abc}}`、`{{random:a,b}}`（单冒号），`{{banned "x"}}`（空格）。
        let (name, first) = match head.find([':', ' ']) { Some(k) if parts.len() == 1 => (&head[..k], Some(&head[k + 1..])), _ => (head, None) };
        if !identifier(name) { return Out::NotMacro; }
        let mut raw: Vec<&str> = first.into_iter().collect();
        raw.extend(&parts[1..]);
        let args: Vec<String> = raw.iter().map(|a| self.walk(a, None, depth + 1)).collect();
        let lower = name.to_ascii_lowercase();
        match self.run(&lower, &args, at, depth) {
            Some(v) => Out::Value(v),
            None => {
                self.unknown.insert(name.to_owned());
                let sep = if first.is_some() { &head[name.len()..name.len() + 1] } else { "::" };
                Out::Keep(if args.is_empty() { format!("{{{{{inner}}}}}") } else { format!("{{{{{name}{sep}{}}}}}", args.join("::")) })
            }
        }
    }

    /// 认识的宏；参数个数不对时返回 None（原样留着）。
    fn run(&mut self, name: &str, args: &[String], at: usize, depth: usize) -> Option<String> {
        let a = |i: usize| args[i].trim();
        let n = args.len();
        let text = |m: &mut Macros, s: String| if depth < 4 && s.contains("{{") { m.walk(&s, None, depth + 1) } else { s };
        let v = match (name, n) {
            ("user" | "notchar", 0) => self.env.user.clone(),
            ("char" | "group" | "groupnotmuted" | "charifnotgroup", 0) => self.env.char.clone(),
            ("model", 0) => self.env.model.clone(),
            ("description", 0) => { let t = self.env.description.clone(); text(self, t) },
            ("personality", 0) => { let t = self.env.personality.clone(); text(self, t) },
            ("scenario", 0) => { let t = self.env.scenario.clone(); text(self, t) },
            ("persona", 0) => { let t = self.env.persona.clone(); text(self, t) },
            ("mesexamples" | "mesexamplesraw" | "charprompt" | "charinstruction" | "charjailbreak" | "charversion" | "char_version" | "creatornotes" | "original" | "noop", 0) => String::new(),
            ("input", 0) => self.env.input.clone(),
            ("lastmessage", 0) => self.env.last_message.clone(),
            ("lastusermessage", 0) => self.env.last_user_message.clone(),
            ("lastcharmessage", 0) => self.env.last_char_message.clone(),
            ("newline", 0) => "\n".into(),
            // 收尾时连同两边的换行一起去掉。
            ("trim", 0) => "{{trim}}".into(),
            ("maxprompt" | "maxprompttokens", 0) => self.env.max_context.saturating_sub(self.env.max_response).to_string(),
            ("maxcontext" | "maxcontexttokens", 0) => self.env.max_context.to_string(),
            ("maxresponse" | "maxresponsetokens", 0) => self.env.max_response.to_string(),
            ("time" | "isotime", 0) => self.env.now.map_or_else(String::new, |(_, _, _, h, mi)| format!("{h:02}:{mi:02}")),
            ("date" | "isodate", 0) => self.env.now.map_or_else(String::new, |(y, m, d, _, _)| format!("{y}-{m:02}-{d:02}")),
            ("weekday", 0) => self.env.now.map_or_else(String::new, |(y, m, d, _, _)| ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"][fdl_core::time::weekday(y, m, d) as usize].into()),
            ("reverse", 1) => a(0).chars().rev().collect(),
            // 没有世界书，outlet 总是空的；banned 在 SillyTavern 里只是登记禁用词，正文里去掉。
            ("outlet" | "banned", 1) => String::new(),
            ("setvar" | "setglobalvar", 2) => { self.map(name).insert(a(0).to_owned(), a(1).to_owned()); String::new() }
            ("addvar" | "addglobalvar", 2) => {
                let (k, add) = (a(0).to_owned(), a(1).to_owned());
                let map = self.map(name);
                let cur = map.get(&k).cloned().unwrap_or_default();
                let next = match (cur.trim().parse::<f64>(), add.parse::<f64>()) {
                    (Ok(x), Ok(y)) => js_number(x + y),
                    _ if cur.is_empty() => add.parse::<f64>().map_or(add.clone(), js_number),
                    _ => format!("{cur}{add}"),
                };
                map.insert(k, next);
                String::new()
            }
            ("incvar" | "decvar" | "incglobalvar" | "decglobalvar", 1) => {
                let by = if name.starts_with("dec") { -1. } else { 1. };
                let k = a(0).to_owned();
                let map = self.map(name);
                let next = js_number(map.get(&k).and_then(|v| v.trim().parse::<f64>().ok()).unwrap_or(0.) + by);
                map.insert(k, next.clone());
                next
            }
            ("getvar" | "getglobalvar", 1) => { let k = a(0).to_owned(); self.map(name).get(&k).cloned().unwrap_or_default() }
            ("random", 1..) => {
                let list = choices(args);
                self.rng = mix(self.rng);
                list.get((self.rng % list.len() as u64) as usize).cloned().unwrap_or_default()
            }
            ("pick", 1..) => {
                let list = choices(args);
                let h = args.iter().flat_map(|s| s.bytes()).fold(self.env.seed ^ at as u64, |h, b| mix(h ^ b as u64));
                list.get((h % list.len() as u64) as usize).cloned().unwrap_or_default()
            }
            _ => return None,
        };
        Some(v)
    }

    fn map(&mut self, name: &str) -> &mut HashMap<String, String> { if name.contains("global") { &mut self.globals } else { &mut self.vars } }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env() -> Env { Env { user: "我".into(), char: "写作助手".into(), description: "帮 {{user}} 写".into(), now: Some((2026, 11, 19, 8, 5)), ..Default::default() } }

    /// 期望值都是在 SillyTavern 1.18 里用 `substituteParams` 实测的。
    #[test]
    fn matches_sillytavern_1_18() {
        let mut m = Macros::new(env());
        let cases = [
            ("{{getvar::a}}|{{setvar::a:: x }}|{{getvar:: a }}", "||x"),
            ("{{setvar::v::  - a}}[{{getvar::v}}]", "[- a]"),
            ("{{setvar::v::\nX\n}}[{{getvar::v}}]", "[X]"),
            ("a\n\n{{trim}}\nb  {{trim}}  c", "ab    c"),
            ("{{// {{setvar::c::1}} }}[{{getvar::c}}]", "[1]"),
            ("{{incvar::n}}{{incvar::n}}{{addvar::n::3}}{{getvar::n}}", "125"),
            ("{{user}} {{USER}} <USER> {{ user }} {{char}}", "我 我 我 我 写作助手"),
            ("{{unknownmacro}} {{unknown::x}}", "{{unknownmacro}} {{unknown::x}}"),
            ("{{setvar::x::a::b}}[{{getvar::x}}]", "{{setvar::x::a::b}}[]"),
            ("{{getvar::missing}}.", "."),
            ("{{setvar::y::{{user}}!}}[{{getvar::y}}]", "[我!]"),
            ("[{{newline}}][{{noop}}]", "[\n][]"),
            ("{{//comment}}x{{// multi\nline }}y", "xy"),
            ("{{setvar::e::}}[{{getvar::e}}]", "[]"),
            ("{{addvar::s::ab}}{{addvar::s:: cd}}[{{getvar::s}}]", "[abcd]"),
            ("{{ setvar :: sp :: v }}[{{getvar::sp}}]", "[v]"),
            ("{{description}}|{{persona}}|{{scenario}}", "帮 我 写||"),
            ("{{unknown::{{user}}}}", "{{unknown::我}}"),
            ("{{user", "{{user"),
            ("{{{user}}}", "{我}"),
            ("{{getvar::{{user}}x}}|{{setvar::{{user}}x::7}}{{getvar::我x}}", "|7"),
            ("{{setvar::b::p}q}}[{{getvar::b}}]", "[p}q]"),
            ("\\{\\{user\\}\\}", "{{user}}"),
            ("{{setvar::t::a\n{{trim}}\nb}}[{{getvar::t}}]", "[ab]"),
            ("{{ // c }}z", "z"),
            ("{{random::a}}|{{pick::a}}", "a|a"),
            ("{{user::x}}", "{{user::x}}"),
            ("{{setvar::n2::5}}{{incvar::n2}}|{{decvar::n2}}|{{addvar::n2::x}}[{{getvar::n2}}]", "6|5|[5x]"),
            ("{{setvar::n3::1.5}}{{addvar::n3::2}}[{{getvar::n3}}]", "[3.5]"),
            ("{{getvar}}|{{setvar::only}}", "{{getvar}}|{{setvar::only}}"),
            ("{{setvar::m::a}}}}x", "}}x"),
            ("{{setvar::sp::a  b}}[{{getvar::sp}}]", "[a  b]"),
            ("{{setvar::nl::\n\n a \n\n}}[{{getvar::nl}}]", "[a]"),
            ("{{setvar::u::{{getvar::b}}{{getvar::b}}}}[{{getvar::u}}]", "[p}qp}q]"),
        ];
        for (input, want) in cases { assert_eq!(m.eval(input), want, "{input:?}"); }
        // 不认识的和参数个数不对的。
        assert_eq!(m.unknown.iter().map(String::as_str).collect::<Vec<_>>(), ["getvar", "setvar", "unknown", "unknownmacro", "user"]);
    }

    #[test]
    fn time_and_legacy_forms() {
        let mut m = Macros::new(env());
        assert_eq!(m.eval("{{isodate}} {{time}} {{weekday}}"), "2026-11-19 08:05 Thursday");
        assert_eq!(m.eval("{{reverse:abc}}|{{banned \"x\"}}|{{outlet::w}}"), "cba||");
        let r = m.eval("{{random:甲, 乙\\,丙}}");
        assert!(r == "甲" || r == "乙,丙");
        assert_eq!(m.eval("{{pick::x::y::z}}"), Macros::new(env()).eval("{{pick::x::y::z}}"));
    }

    #[test]
    fn post_escapes_each_value() {
        let mut m = Macros::new(Env { user: "a.b".into(), ..Default::default() });
        assert_eq!(m.eval_with("x{{user}}y", &|s| s.replace('.', "\\.")), "xa\\.by");
    }
}
