//! 正则脚本：SillyTavern 的 `regex_scripts`。
//! 查找串是 JS 的 `/pattern/flags`，这里转成 fancy-regex（支持前后断言、反向引用）能读的写法。
//! 替换照 SillyTavern：`{{match}}` 是整段匹配，`$1`、`$<name>` 是分组（先去掉 `trimStrings`），
//! 别的 `$` 写法原样输出，最后整段再过一遍宏。
use std::sync::OnceLock;

use fancy_regex::{Captures, Regex, RegexBuilder};
use serde_json::Value;

use super::macros::Macros;
use super::preset::{flag, num, text};

/// 脚本作用在哪类文字上（`placement`）。
pub const USER_INPUT: i64 = 1;
pub const AI_OUTPUT: i64 = 2;

/// 什么时候跑：存下来时（两个开关都没开）、发送前（`promptOnly`）、显示时（`markdownOnly`）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope { Stored, Prompt, Display }

#[derive(Clone, Debug, Default)]
pub struct Script {
    pub id: String,
    pub name: String,
    pub find: String,
    pub replace: String,
    pub trim: Vec<String>,
    pub placement: Vec<i64>,
    pub disabled: bool,
    pub markdown_only: bool,
    pub prompt_only: bool,
    /// 查找串里替换宏：0 不替换，1 原样，2 转义后。
    pub substitute: i64,
    pub min_depth: Option<i64>,
    pub max_depth: Option<i64>,
}

impl Script {
    pub fn read(v: &Value) -> Script {
        Script {
            id: text(v, "id").unwrap_or_default(),
            name: text(v, "scriptName").unwrap_or_default(),
            find: text(v, "findRegex").unwrap_or_default(),
            replace: text(v, "replaceString").unwrap_or_default(),
            trim: v.get("trimStrings").and_then(Value::as_array).map_or_else(Vec::new, |a| a.iter().filter_map(Value::as_str).map(str::to_owned).collect()),
            placement: v.get("placement").and_then(Value::as_array).map_or_else(Vec::new, |a| a.iter().filter_map(Value::as_i64).collect()),
            disabled: flag(v, "disabled").unwrap_or(false),
            markdown_only: flag(v, "markdownOnly").unwrap_or(false),
            prompt_only: flag(v, "promptOnly").unwrap_or(false),
            substitute: num(v, "substituteRegex").map_or(0, |n| n as i64),
            min_depth: num(v, "minDepth").map(|n| n as i64),
            max_depth: num(v, "maxDepth").map(|n| n as i64),
        }
    }

    /// 这条脚本这一次跑不跑。`depth` 是这条消息离聊天记录末尾有几条（最新的是 0）。
    pub fn applies(&self, placement: i64, scope: Scope, depth: Option<usize>) -> bool {
        if self.disabled || self.find.is_empty() || !self.placement.contains(&placement) { return false; }
        let scoped = match scope {
            Scope::Display => self.markdown_only,
            Scope::Prompt => self.prompt_only,
            Scope::Stored => !self.markdown_only && !self.prompt_only,
        };
        if !scoped { return false; }
        if let Some(d) = depth.map(|d| d as i64) {
            if self.min_depth.is_some_and(|m| m >= -1 && d < m) { return false; }
            if self.max_depth.is_some_and(|m| m >= 0 && d > m) { return false; }
        }
        true
    }

    /// 替换结果是 HTML（显示用的卡片之类）：笔记里画不了，成形时另作处理。
    pub fn emits_html(&self) -> bool {
        static TAG: OnceLock<Regex> = OnceLock::new();
        let re = TAG.get_or_init(|| Regex::new(r"(?i)<\s*/?\s*[a-z][a-z0-9-]*(\s[^>]*)?/?\s*>|```html").unwrap());
        re.is_match(&self.replace).unwrap_or(false)
    }

    /// 编译查找串。宏替换要用 `macros`；编译不了（语法 JS 不认、或这里不支持）时返回原因。
    pub fn compile(&self, macros: &mut Macros) -> Result<Compiled, String> {
        let find = match self.substitute {
            1 => macros.eval(&self.find),
            2 => macros.eval_with(&self.find, &escape_macro),
            _ => self.find.clone(),
        };
        compile(&find)
    }

    /// 跑一遍。编译不了或运行出错（回溯太多）时原样返回。
    pub fn run(&self, text: &str, macros: &mut Macros) -> String {
        self.compile(macros).and_then(|c| self.run_compiled(&c, text, macros)).unwrap_or_else(|_| text.to_owned())
    }

    /// 用编译好的查找串跑：一次拼装里同一条脚本要跑很多段。运行出错（回溯超限）时返回原因，调用方原样保留这段。
    pub fn run_compiled(&self, c: &Compiled, text: &str, macros: &mut Macros) -> Result<String, String> {
        if text.is_empty() || c.hopeless(text) { return Ok(text.to_owned()); }
        let template = match_macro().replace_all(&self.replace, "$$0").into_owned();
        let trim: Vec<String> = self.trim.iter().map(|t| macros.eval(t)).filter(|t| !t.is_empty()).collect();
        // 每处匹配：先填分组，再在这一段上跑宏（可能有 setvar，所以按匹配顺序逐段跑）。
        let result = c.re.try_replacen(text, if c.global { 0 } else { 1 }, |caps: &Captures<'_, str>| {
            let out = group_ref().replace_all(&template, |r: &Captures<'_, str>| {
                let m = match (r.get(1), r.get(2)) {
                    (Some(n), _) => n.as_str().parse::<usize>().ok().and_then(|n| caps.get(n)).map(|m| m.as_str()),
                    (_, Some(name)) => caps.name(name.as_str()).map(|m| m.as_str()),
                    _ => None,
                };
                trim.iter().fold(m.unwrap_or("").to_owned(), |s, t| s.replace(t.as_str(), ""))
            });
            macros.eval(&out)
        });
        result.map(|s| s.into_owned()).map_err(|e| e.to_string())
    }
}

fn match_macro() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"(?i)\{\{match\}\}").unwrap())
}

fn group_ref() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"\$(\d+)|\$<([^>]+)>").unwrap())
}

/// 宏的值放进正则前转义（SillyTavern 的 `sanitizeRegexMacro`）。
fn escape_macro(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\n' => out.push_str("\\n"), '\r' => out.push_str("\\r"), '\t' => out.push_str("\\t"),
            '\u{b}' => out.push_str("\\v"), '\u{c}' => out.push_str("\\f"), '\0' => out.push_str("\\0"),
            '.' | '^' | '$' | '*' | '+' | '?' | '{' | '}' | '[' | ']' | '\\' | '/' | '|' | '(' | ')' => { out.push('\\'); out.push(c); }
            _ => out.push(c),
        }
    }
    out
}

pub struct Compiled {
    pub re: Regex,
    pub global: bool,
    /// 每个顶层分支必须出现的字面串；一个分支都凑不齐时不用跑（长文上省掉没指望的回溯，TauriTavern 的 gatedReplace 也这么做）。空的是不筛。
    needs: Vec<Vec<String>>,
    icase: bool,
}

impl Compiled {
    /// 流式输出时：最后一处开了头（某个分支的第一个字面串出现了）、但同一分支后面的字面串还没全出现的位置。
    /// 成形要等这段写完再动它，免得一段卡片先按正文显示、写完了又收进折叠块。只按 ASCII 忽略大小写，位置不变。
    pub fn unfinished(&self, text: &str) -> Option<usize> {
        let folded;
        let hay = if self.icase { folded = text.to_ascii_lowercase(); folded.as_str() } else { text };
        self.needs.iter().filter(|alt| alt.len() > 1).filter_map(|alt| {
            let at = hay.rfind(alt[0].as_str())?;
            let rest = &hay[at + alt[0].len()..];
            (!alt[1..].iter().all(|lit| rest.contains(lit.as_str()))).then_some(at)
        }).min()
    }

    /// 流式输出时：末尾是某个字面串的开头（`### 正` 之于 `### 正文`）。返回那段开头的位置。
    pub fn partial_tail(&self, text: &str) -> Option<usize> {
        let folded;
        let hay = if self.icase { folded = text.to_ascii_lowercase(); folded.as_str() } else { text };
        self.needs.iter().flatten().filter_map(|lit| {
            lit.char_indices().skip(1).map(|(i, _)| &lit[..i]).filter(|head| hay.ends_with(head)).last().map(|head| hay.len() - head.len())
        }).min()
    }

    fn hopeless(&self, text: &str) -> bool {
        if self.needs.is_empty() { return false; }
        let folded;
        let hay = if self.icase { folded = text.to_lowercase(); folded.as_str() } else { text };
        !self.needs.iter().any(|alt| alt.iter().all(|lit| hay.contains(lit.as_str())))
    }
}

/// 回溯上限。fancy-regex 默认一百万，长篇输出（几万字）上的 `[\s\S]+?…` 不够用；超了算运行出错。
const BACKTRACK_LIMIT: usize = 50_000_000;
/// JS 的行终止符：`.` 不匹配它们，`m` 模式下 `^`、`$` 认它们。
const LINE_END: &str = "\\n\\r\\x{2028}\\x{2029}";

/// 把 JS 的 `/pattern/flags`（或不带斜杠的整串）编译成 fancy-regex。
pub fn compile(find: &str) -> Result<Compiled, String> {
    let (pattern, flags) = split(find);
    let mut prefix = String::new();
    let mut global = false;
    for f in flags.chars() {
        match f {
            'g' => global = true,
            // `m` 不交给 Rust：JS 的行尾还包括 `\r`、U+2028、U+2029，在 translate 里写出来。
            'm' => {}
            'i' | 's' => prefix.push(f),
            'u' => {}
            // JS 不认这些，SillyTavern 里这条脚本直接不跑。
            _ => return Err(format!("不支持的标志 {f}")),
        }
    }
    let translated = translate(pattern, flags.contains('s'), flags.contains('m'))?;
    let full = if prefix.is_empty() { translated } else { format!("(?{prefix}){translated}") };
    let re = RegexBuilder::new(&full).backtrack_limit(BACKTRACK_LIMIT).build().map_err(|e| e.to_string())?;
    let icase = flags.contains('i');
    let mut needs = requirements(pattern);
    if icase { needs = needs.into_iter().map(|a| a.into_iter().map(|l| l.to_lowercase()).collect()).collect(); }
    Ok(Compiled { re, global, needs, icase })
}

/// 照 SillyTavern 的 `regexFromString`：用 `(\/?)(.+)\1([a-z]*)` 拆出正文和标志；
/// 标志不在 `gmixXsuUAJ` 里或有重复，就整串当正则、不带标志。
fn split(find: &str) -> (&str, &str) {
    static R: OnceLock<Regex> = OnceLock::new();
    let re = R.get_or_init(|| Regex::new(r"(?i)(/?)(.+)\1([a-z]*)").unwrap());
    let Ok(Some(m)) = re.captures(find) else { return (find, "") };
    let (body, flags) = (m.get(2).map_or("", |g| g.as_str()), m.get(3).map_or("", |g| g.as_str()));
    let mut seen = String::new();
    let valid = flags.chars().all(|c| "gmixXsuUAJ".contains(c) && !seen.contains(c) && { seen.push(c); true });
    if !flags.is_empty() && !valid { return (find, ""); }
    (body, flags)
}

/// JS 正则语法 → Rust（regex / fancy-regex）语法。只处理两边意思不一样的地方：
/// `\d \w \b` 在 JS 里只认 ASCII；`[^]`、`[]`；JS 里没意义的转义（`\<`、`\/`）是字面字符；
/// 不成量词的 `{` 是字面字符；字符类里的 `[ & ~` 在 Rust 有集合运算的意思，要转义；`\uXXXX`。
fn translate(p: &str, dotall: bool, multiline: bool) -> Result<String, String> {
    let c: Vec<char> = p.chars().collect();
    let mut out = String::with_capacity(p.len() + 8);
    let mut i = 0;
    let mut class = false;
    let hex = |s: &[char]| s.iter().all(|c| c.is_ascii_hexdigit());
    while i < c.len() {
        let ch = c[i];
        if ch == '\\' {
            let Some(&n) = c.get(i + 1) else { return Err("正则以反斜杠结尾".into()) };
            i += 2;
            match n {
                'd' => out.push_str(if class { "0-9" } else { "[0-9]" }),
                'D' if !class => out.push_str("[^0-9]"),
                'w' => out.push_str(if class { "A-Za-z0-9_" } else { "[A-Za-z0-9_]" }),
                'W' if !class => out.push_str("[^A-Za-z0-9_]"),
                'b' if class => out.push_str("\\x08"),
                // JS 的单词边界只看 ASCII 单词字符；fancy-regex 不能局部关掉 Unicode，用前后断言写出来。
                'b' => out.push_str("(?:(?<=[A-Za-z0-9_])(?![A-Za-z0-9_])|(?<![A-Za-z0-9_])(?=[A-Za-z0-9_]))"),
                'B' => out.push_str("(?:(?<=[A-Za-z0-9_])(?=[A-Za-z0-9_])|(?<![A-Za-z0-9_])(?![A-Za-z0-9_]))"),
                'u' if c.get(i) == Some(&'{') => {
                    let end = c[i..].iter().position(|&x| x == '}').map(|e| i + e).ok_or("\\u{ 没有闭合")?;
                    out.push_str(&format!("\\x{{{}}}", c[i + 1..end].iter().collect::<String>()));
                    i = end + 1;
                }
                'u' if c.len() >= i + 4 && hex(&c[i..i + 4]) => { out.push_str(&format!("\\x{{{}}}", c[i..i + 4].iter().collect::<String>())); i += 4; }
                'x' if c.len() >= i + 2 && hex(&c[i..i + 2]) => { out.push_str(&format!("\\x{}", c[i..i + 2].iter().collect::<String>())); i += 2; }
                'c' if c.get(i).is_some_and(|x| x.is_ascii_alphabetic()) => { out.push_str(&format!("\\x{:02x}", (c[i] as u8) % 32)); i += 1; }
                '0' if !c.get(i).is_some_and(|x| x.is_ascii_digit()) => out.push_str("\\x00"),
                'k' if c.get(i) == Some(&'<') => out.push_str("\\k"),
                'D' | 'W' | 's' | 'S' | 'n' | 'r' | 't' | 'f' | 'v' | 'p' | 'P' | '1'..='9' => { out.push('\\'); out.push(n); }
                // 别的转义在 JS 里就是这个字符本身。
                _ if n.is_ascii_alphanumeric() => out.push(n),
                _ => push_literal(&mut out, n, class),
            }
            continue;
        }
        if class {
            match ch {
                ']' => { class = false; out.push(']'); }
                '[' | '&' | '~' => { out.push('\\'); out.push(ch); }
                _ => out.push(ch),
            }
            i += 1;
            continue;
        }
        match ch {
            '[' if c.get(i + 1) == Some(&']') => { out.push_str("(?!)"); i += 2; }
            '[' if c.get(i + 1) == Some(&'^') && c.get(i + 2) == Some(&']') => { out.push_str("[\\s\\S]"); i += 3; }
            '[' => {
                class = true;
                out.push('[');
                i += 1;
                if c.get(i) == Some(&'^') { out.push('^'); i += 1; }
                // JS 里紧跟在开头的 `]` 是关闭；Rust 会把它当字面字符，这里不会走到（上面已处理空类）。
            }
            '{' => {
                let rest: String = c[i..].iter().take_while(|&&x| x != '}').collect();
                let body = &rest[1..];
                let quant = i > 0 && c.get(i + rest.chars().count()) == Some(&'}') && !body.is_empty()
                    && body.split(',').count() <= 2 && body.split(',').next().is_some_and(|a| !a.is_empty() && a.chars().all(|x| x.is_ascii_digit()))
                    && body.split(',').nth(1).is_none_or(|b| b.chars().all(|x| x.is_ascii_digit()));
                if quant { out.push_str(&rest); out.push('}'); i += rest.chars().count() + 1; } else { out.push_str("\\{"); i += 1; }
            }
            '}' => { out.push_str("\\}"); i += 1; }
            '.' if !dotall => { out.push_str(&format!("[^{LINE_END}]")); i += 1; }
            '^' if multiline => { out.push_str(&format!("(?<![^{LINE_END}])")); i += 1; }
            '$' if multiline => { out.push_str(&format!("(?![^{LINE_END}])")); i += 1; }
            _ => { out.push(ch); i += 1; }
        }
    }
    if class { return Err("字符类没有闭合".into()); }
    Ok(out)
}

/// 每个顶层分支里一定要出现的字面串（JS 写法上直接看）。只取最外层、不在括号和字符类里的连续字面字符；
/// 后面跟着 `? * {` 的那个字符不算。宁可少算：少算只是不筛，多算会漏掉匹配。有一个分支什么都要不到就整个不筛。
fn requirements(p: &str) -> Vec<Vec<String>> {
    let c: Vec<char> = p.chars().collect();
    let (mut alts, mut cur, mut run) = (vec![], vec![], String::new());
    let (mut depth, mut class, mut i) = (0usize, false, 0);
    let end = |run: &mut String, cur: &mut Vec<String>| { if run.chars().count() >= 2 { cur.push(std::mem::take(run)); } else { run.clear(); } };
    while i < c.len() {
        let ch = c[i];
        let lit = match ch {
            '\\' => { i += 1; c.get(i).copied().filter(|n| !n.is_ascii_alphanumeric()) }
            _ if class => { if ch == ']' { class = false; } None }
            '[' => { class = true; None }
            '(' => { depth += 1; None }
            ')' => { depth = depth.saturating_sub(1); None }
            '|' if depth == 0 => { end(&mut run, &mut cur); alts.push(std::mem::take(&mut cur)); None }
            // 量词 `{m,n}` 整段跳过（不成量词的 `{…}` 也跳过：少算无妨）。
            '{' => { i = c[i..].iter().position(|&x| x == '}').map_or(c.len(), |e| i + e); None }
            '^' | '$' | '.' | '|' | '+' | '*' | '?' | '}' => None,
            _ => Some(ch),
        };
        i += 1;
        match lit {
            Some(l) if depth == 0 && !class => {
                if matches!(c.get(i), Some('?' | '*' | '{')) { end(&mut run, &mut cur); } else { run.push(l); }
                if c.get(i) == Some(&'+') { end(&mut run, &mut cur); }
            }
            _ => end(&mut run, &mut cur),
        }
    }
    end(&mut run, &mut cur);
    alts.push(cur);
    if alts.iter().any(Vec::is_empty) { vec![] } else { alts }
}

fn push_literal(out: &mut String, ch: char, class: bool) {
    let meta = if class { "\\]-^[&~" } else { "\\.+*?()|[]{}^$#&-~" };
    if meta.contains(ch) { out.push('\\'); }
    out.push(ch);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tavern::macros::Env;

    fn m() -> Macros { Macros::new(Env { user: "我".into(), char: "助手".into(), ..Default::default() }) }

    fn script(find: &str, replace: &str) -> Script { Script { find: find.into(), replace: replace.into(), placement: vec![2], ..Default::default() } }

    #[test]
    fn js_syntax() {
        let t = |p: &str, s: &str| compile(p).unwrap().re.is_match(s).unwrap();
        assert!(t(r"/\d+/", "a12") && !t(r"/^\d$/", "١"));
        assert!(t(r"/[^]+/", "\n") && !t(r"/a[]/", "a"));
        assert!(t(r"/\<tag\>/", "<tag>") && t(r"/a\/b/", "a/b"));
        assert!(t(r"/{{x}}/", "{{x}}") && t(r"/a{2}/", "aa") && t(r"/a{2,}/", "aaa") && !t(r"/^a{2}$/", "a{2}"));
        assert!(t(r"/[a&&b]/", "&") && t(r"/[\d.]/", "."));
        assert!(t(r"/\u4f60/", "你") && t(r"/\x41/", "A"));
        // 中文不算 JS 的单词字符，`\b` 落在它和字母之间。
        assert!(t(r"/\bab/", "你ab"));
        assert!(t(r"/(?<=<p>)x/", "<p>x") && t(r"/(?<=a[\s\S]+?)z/", "a  z"));
        assert!(t("/ABC/i", "abc") && t("/^b/m", "a\nb") && t("/a.b/s", "a\nb"));
        // JS 的行终止符还有 \r、U+2028、U+2029。
        assert!(!t("/a.b/", "a\rb") && t("/a.b/", "a\u{85}b") && t("/^b/m", "a\rb") && t("/a$/m", "a\u{2028}b") && !t("/^b/", "a\nb"));
        assert!(compile("/a/x").is_err());
        assert!(t("/a/y", "/a/y") && !t("/a/y", "a"));
        // 不是 /…/ 的样子：整串当正则。
        assert!(t(r"^[\s\S]*?### 正文", "思考\n### 正文"));
    }

    #[test]
    fn literal_gate() {
        let n = |p: &str| requirements(p);
        assert_eq!(n(r"<meow_FM>(?![\s\S]+<x>)\s*time[:：]"), [vec!["<meow_FM>".to_owned(), "time".into()]]);
        assert_eq!(n(r"<a>.+?</a>|<bb?>"), [vec!["<a>".to_owned(), "</a>".into()], vec!["<b".into()]]);
        assert!(n(r"死死[的地]?|一抹|x").is_empty());
        assert_eq!(n(r"ab+c\/d"), [vec!["ab".to_owned(), "c/d".into()]]);
        assert_eq!(n(r"seri[\w]{0,4}[:：]x{2}yz"), [vec!["seri".to_owned(), "yz".into()]]);
        // 有一个分支要不到字面串（这里的 `-`）就不筛。
        assert!(n(r"(?<=<p style)-|\s<finish>").is_empty());
        let c = compile("/<TAG>x/i").unwrap();
        assert!(c.hopeless("没有") && !c.hopeless("<tag>X"));
    }

    #[test]
    fn replacement_follows_sillytavern() {
        let mut mm = m();
        let s = Script { trim: vec!["**".into()], ..script(r"/(\w+)=(\w+)/g", "$2:$1 {{match}} $& $9 {{char}}") };
        assert_eq!(s.run("a=b c=d", &mut mm), "b:a a=b $&  助手 d:c c=d $&  助手");
        let s = Script { trim: vec!["**".into()], ..script(r"/<(?<t>[^>]+)>/", "[$<t>]") };
        assert_eq!(s.run("<**x**> <y>", &mut mm), "[x] <y>");
        // 宏在替换结果上跑，setvar 生效。
        let s = script("/v=(\\d)/", "{{setvar::v::$1}}");
        assert_eq!(s.run("v=7", &mut mm), "");
        assert_eq!(mm.var("v"), Some("7"));
    }

    #[test]
    fn scope_and_depth() {
        let s = Script { prompt_only: true, min_depth: Some(2), max_depth: Some(4), ..script("/a/", "b") };
        assert!(!s.applies(AI_OUTPUT, Scope::Stored, Some(3)) && !s.applies(USER_INPUT, Scope::Prompt, Some(3)));
        assert!(s.applies(AI_OUTPUT, Scope::Prompt, Some(3)) && s.applies(AI_OUTPUT, Scope::Prompt, None));
        assert!(!s.applies(AI_OUTPUT, Scope::Prompt, Some(1)) && !s.applies(AI_OUTPUT, Scope::Prompt, Some(5)));
        let plain = script("/a/", "b");
        assert!(plain.applies(AI_OUTPUT, Scope::Stored, Some(0)) && !plain.applies(AI_OUTPUT, Scope::Display, None));
        assert!(script("/a/", "<div class=\"x\">$1</div>").emits_html() && !script("/a/", "“$1”").emits_html());
    }

    #[test]
    fn substituted_find() {
        let mut mm = Macros::new(Env { user: "a.b".into(), ..Default::default() });
        let s = Script { substitute: 2, ..script("/{{user}}/g", "X") };
        assert_eq!(s.run("a.b axb", &mut mm), "X axb");
        let s = Script { substitute: 1, ..script("/{{user}}/g", "X") };
        assert_eq!(s.run("a.b axb", &mut mm), "X X");
    }
}
