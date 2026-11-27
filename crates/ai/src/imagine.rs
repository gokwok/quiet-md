//! 生图的提示词：画风和角色两个库，加上每张图的那句描述，拼成 NovelAI V4 起的提示词。
//!
//! - 画风：一组正向标签（画师、媒介、风格）和一组负面标签。质量词各模型不一样，由调用方按模型加，不放进画风，全局选一个；内置几种（`builtin`），只能看，可以复制一份再改。
//! - 人物：名字、别名、外貌标签（含 girl / boy）、排除标签、头像。别名少于两个字的不认（「夕」会撞上「夕阳」）。描述里写到名字或别名的角色，外貌标签放进分角色提示词（`char_captions`），
//!   排除标签放进分角色的负面词（`v4_negative_prompt` 里同一位置），几个人的特征不混在一起。
//! - 质量词：一组正向（画质、年代）和一组负向（画坏的手、文字、打码），每张图都带，和画风是两回事：画风管「画成什么样」，质量词管「画得好不好」。
//!   有一份默认的，可以改；改回和默认一样就不存，以后默认改了也跟着走。
//! - 描述默认原样放进主提示词。打开「优化描述」时先交给文本模型（`plan_messages`）：按 NovelAI 的写法和配图的分镜规矩，
//!   把描述变成这一幕的画面标签、每个出场人物这一幕的表情动作，和要额外排除的东西（`Plan`），人确认、改过后再拼（`assemble` 的 `plan`）。
//!   人物外貌只用人物里写好的，模型不重复也不改；人数、画风、质量词照旧由这里拼，模型不碰。
//!
//! 存在全局库（`global`）的 `imagine.json`，和写作的预设一起，各空间共用。读的时候宽松，缺字段按空的算。
use std::path::Path;

use fdl_core::file_io;
use serde_json::{Value, json};

use crate::tavern::preset::text;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Style { pub id: String, pub name: String, pub positive: String, pub negative: String }

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Character {
    pub id: String, pub name: String, pub aliases: Vec<String>, pub look: String,
    /// 只压这个人的标签（比如不要长发），原样发，不拆权重。
    pub negative: String,
    /// 头像：界面那头认的标识（自带的 `builtin-NN` 或照片），空是用名字的第一个字。
    pub avatar: Option<String>,
}

/// 默认的质量词。带权重的写法（`2.0::text, caption::`）原样留着，拼的时候不拆。
pub const QUALITY: &str = "HDR, masterpiece, very_aesthetic, high_quality, best_quality, latest, newest, fine_fabric_emphasis, ultra-detailed, year_2026, year_2025";
pub const QUALITY_NEGATIVE: &str = "snow, snowy, winter, 2.0::text, caption, subtitle, letters, words, typography::, deformed hands, extra fingers, 1.5::censored, mosaic, bars, glowing censor::";

/// 内置画风的标识都以它开头。
pub const BUILTIN: &str = "builtin-";

/// 内置的画风：(标识, 中文名, 英文名, 正向, 负面)。
#[allow(clippy::type_complexity)]
const BUILTINS: &[(&str, &str, &str, &str, &str)] = &[
    ("builtin-anime", "动画", "Anime", "anime screenshot, anime coloring, cel shading, clean lineart", "3d, realistic"),
    ("builtin-painterly", "厚涂", "Painterly", "painterly, impasto, oil painting (medium), dramatic lighting, detailed shading", "flat color, lineart"),
    ("builtin-watercolor", "水彩", "Watercolor", "watercolor (medium), traditional media, muted colors, soft edges, white background", "3d, realistic, high contrast"),
    ("builtin-lineart", "黑白线稿", "Line Art", "monochrome, greyscale, lineart, screentone, manga", "color"),
    ("builtin-picturebook", "绘本", "Picture Book", "storybook illustration, colored pencil (medium), pastel colors, simple background", "3d, realistic"),
];

/// 内置画风；`english` 时名字用英文。
pub fn builtin(english: bool) -> Vec<Style> {
    BUILTINS.iter().map(|&(id, zh, en, positive, negative)| Style { id: id.into(), name: (if english { en } else { zh }).into(), positive: positive.into(), negative: negative.into() }).collect()
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Library {
    /// 自己加的画风（内置的不在这里）。
    pub styles: Vec<Style>,
    /// 选中的画风；空是不用画风。
    pub style: String,
    pub characters: Vec<Character>,
    /// 改过的质量词（正向、负向）；`None` 是用默认的。
    pub quality: Option<String>,
    pub quality_negative: Option<String>,
    /// 配图卡片上「优化描述」开着（记住上一次）。
    pub optimize: bool,
}

fn file(dir: &Path) -> std::path::PathBuf { dir.join("imagine.json") }
fn words(v: &Value, k: &str) -> Vec<String> {
    v.get(k).and_then(Value::as_array).map_or_else(Vec::new, |a| a.iter().filter_map(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned).collect())
}

impl Library {
    pub fn load(dir: &Path) -> Library {
        let Some(v) = file_io::read(file(dir)).ok().and_then(|b| serde_json::from_slice::<Value>(&b).ok()) else { return Library::default() };
        let list = |k: &str| v.get(k).and_then(Value::as_array).cloned().unwrap_or_default();
        let t = |v: &Value, k: &str| text(v, k).unwrap_or_default();
        let styles = list("styles").iter().filter_map(|s| Some(Style { id: text(s, "id").filter(|i| !i.is_empty() && !i.starts_with(BUILTIN))?, name: t(s, "name"), positive: t(s, "positive"), negative: t(s, "negative") })).collect();
        let characters = list("characters").iter().filter_map(|c| Some(Character { id: text(c, "id").filter(|i| !i.is_empty())?, name: t(c, "name"), aliases: words(c, "aliases"), look: t(c, "look"), negative: t(c, "negative"), avatar: text(c, "avatar").filter(|a| !a.is_empty()) })).collect();
        Library { styles, style: t(&v, "style"), characters, quality: text(&v, "quality"), quality_negative: text(&v, "quality_negative"), optimize: v.get("optimize").and_then(Value::as_bool).unwrap_or(false) }
    }

    pub fn save(&self, dir: &Path) -> std::io::Result<()> {
        let v = json!({
            "style": self.style,
            "styles": self.styles.iter().map(|s| json!({ "id": s.id, "name": s.name, "positive": s.positive, "negative": s.negative })).collect::<Vec<_>>(),
            "characters": self.characters.iter().map(|c| {
                let mut v = json!({ "id": c.id, "name": c.name, "aliases": c.aliases, "look": c.look, "negative": c.negative });
                if let Some(a) = &c.avatar { v["avatar"] = a.as_str().into(); }
                v
            }).collect::<Vec<_>>(),
        });
        let mut v = v;
        if let Some(q) = &self.quality { v["quality"] = q.as_str().into(); }
        if let Some(q) = &self.quality_negative { v["quality_negative"] = q.as_str().into(); }
        if self.optimize { v["optimize"] = true.into(); }
        let path = file(dir);
        file_io::create_dir_all(dir)?;
        let bytes = serde_json::to_vec_pretty(&v).map_err(std::io::Error::other)?;
        file_io::with_write(&path, |p| {
            let tmp = p.with_extension("json.tmp");
            std::fs::write(&tmp, &bytes)?;
            Ok(std::fs::rename(&tmp, p)?)
        }).map_err(std::io::Error::other)
    }

    /// 标识对应的画风（内置的也算）；找不到是 `None`。
    pub fn find_style(&self, id: &str, english: bool) -> Option<Style> {
        if id.is_empty() { return None; }
        builtin(english).into_iter().chain(self.styles.iter().cloned()).find(|s| s.id == id)
    }
    pub fn quality(&self) -> &str { self.quality.as_deref().unwrap_or(QUALITY) }
    pub fn quality_negative(&self) -> &str { self.quality_negative.as_deref().unwrap_or(QUALITY_NEGATIVE) }
    /// 改质量词：和默认一样就回到默认（不存）；空着就是不要质量词。`negative` 是负向那组。
    pub fn set_quality(&mut self, negative: bool, value: &str) {
        let (slot, default) = if negative { (&mut self.quality_negative, QUALITY_NEGATIVE) } else { (&mut self.quality, QUALITY) };
        let value = value.trim();
        *slot = (value != default).then(|| value.to_owned());
    }

    /// 选中的画风；选的那个删掉了就是不用画风。
    pub fn current_style(&self, english: bool) -> Option<Style> { self.find_style(&self.style, english) }
}

/// 描述里写到的角色，按第一次出现的先后排。同一处能对上好几个名字时取最长的（「林夕」不会被别名「夕」抢走）；
/// 英文名不分大小写。超过三个 NovelAI 容易把人画混，多的不要。
pub fn mentioned<'a>(characters: &'a [Character], input: &str) -> Vec<&'a Character> {
    let hay = input.to_lowercase();
    let mut names: Vec<(String, usize)> = characters.iter().enumerate()
        .flat_map(|(i, c)| std::iter::once((&c.name, true)).chain(c.aliases.iter().map(|a| (a, false))).map(move |(n, own)| (n.trim().to_lowercase(), own, i)))
        .filter(|(n, own, _)| if *own { !n.is_empty() } else { n.chars().count() >= 2 }).map(|(n, _, i)| (n, i)).collect();
    names.sort_by_key(|a| std::cmp::Reverse(a.0.len()));
    let mut taken = vec![false; hay.len()];
    let mut first: Vec<Option<usize>> = vec![None; characters.len()];
    for (name, i) in &names {
        let mut from = 0;
        while let Some(at) = hay[from..].find(name.as_str()).map(|x| x + from) {
            let end = at + name.len();
            if !taken[at..end].iter().any(|t| *t) {
                taken[at..end].iter_mut().for_each(|t| *t = true);
                first[*i] = Some(first[*i].map_or(at, |f| f.min(at)));
            }
            from = end;
        }
    }
    let mut hit: Vec<(usize, &Character)> = first.iter().enumerate().filter_map(|(i, f)| f.map(|f| (f, &characters[i]))).collect();
    hit.sort_by_key(|(f, _)| *f);
    hit.into_iter().map(|(_, c)| c).take(3).collect()
}

/// 拼好的提示词：主提示词、负面词，和每个角色的分角色提示词。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Prompt { pub base: String, pub negative: String, pub characters: Vec<Cast> }

/// 出场的一个人：名字、外貌标签、排除标签。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Cast { pub name: String, pub tags: String, pub negative: String }

/// 逗号分开的标签：去掉空的和重复的（不分大小写），留原来的顺序。
pub fn join(parts: &[&str]) -> String {
    let mut seen = std::collections::HashSet::new();
    parts.iter().flat_map(|p| p.split([',', '，', '\n'])).map(str::trim).filter(|t| !t.is_empty() && seen.insert(t.to_lowercase())).collect::<Vec<_>>().join(", ")
}

/// 外貌标签里的性别标签（girl / boy / other，可以带 `1`，可以在权重组里）：取第一个。
fn gender(look: &str) -> Option<&'static str> {
    look.split([',', '，']).find_map(|t| {
        let t = t.trim().trim_end_matches("::").rsplit("::").next()?.trim().to_lowercase();
        match t.trim_start_matches('1') { "girl" => Some("girl"), "boy" => Some("boy"), "other" => Some("other"), _ => None }
    })
}

/// 人数标签（`1girl`、`2girls, 1boy`）：按出场角色提示词（`looks`）里的性别数，再加上 `more`（画面里没名字的人，各性别几个）。没写性别的不算。
fn counts(looks: &[&str], more: [usize; 3]) -> String {
    ["girl", "boy", "other"].iter().zip(more).filter_map(|(g, more)| {
        let n = looks.iter().filter(|l| gender(l) == Some(g)).count() + more;
        (n > 0).then(|| if n == 1 { format!("1{g}") } else { format!("{n}{g}s") })
    }).collect::<Vec<_>>().join(", ")
}

/// 逗号分开的一段拆成一个个标签，带权重的组（`1.5::a, b::`）整组算一个，不从中间拆开。
pub fn pieces(s: &str) -> Vec<String> {
    let mut out: Vec<String> = vec![];
    let mut cur = String::new();
    for part in s.split([',', '，', '\n']) {
        if !cur.is_empty() { cur.push(','); }
        cur.push_str(part);
        if cur.matches("::").count().is_multiple_of(2) { let t = cur.trim(); if !t.is_empty() { out.push(t.to_owned()); } cur.clear(); }
    }
    let t = cur.trim();
    if !t.is_empty() { out.push(t.to_owned()); }
    out
}

/// 画面标签里模型写的人数标签（没名字的路人：`1boy`、`2girls`）拿出来：（剩下的标签, 各性别几个）。
fn take_counts(scene: &str) -> (String, [usize; 3]) {
    let mut more = [0; 3];
    let kept: Vec<String> = pieces(scene).into_iter().filter(|t| {
        let t = t.to_lowercase();
        let digits = t.chars().take_while(char::is_ascii_digit).count();
        let (Ok(n), rest) = (t[..digits].parse::<usize>(), &t[digits..]) else { return true };
        let Some(i) = ["girl", "boy", "other"].iter().position(|g| rest == *g || rest.strip_suffix('s') == Some(g)) else { return true };
        more[i] += n;
        false
    }).collect();
    (kept.join(", "), more)
}

/// 模型整理出的这一幕：画面标签（构图、场景、光线，可以带没名字的人的人数标签）、每个出场人物这一幕完整的提示词和排除（从预设改出来的）、画面额外排除的。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Plan { pub scene: String, pub characters: Vec<Cast>, pub negative: String }

/// 权重组（`1.4::a, b::`、`-1::c::`）拆成（权重, 组里的标签）；不是权重组是 `None`。
fn group(piece: &str) -> Option<(&str, Vec<String>)> {
    let (weight, inner) = piece.strip_suffix("::")?.split_once("::")?;
    weight.trim().parse::<f32>().ok()?;
    Some((weight.trim(), inner.split([',', '，']).map(str::trim).filter(|t| !t.is_empty()).map(str::to_owned).collect()))
}

/// 一个个标签（权重组拆开，组里的各算一个），按原来的顺序。
pub fn tags(s: &str) -> Vec<String> {
    pieces(s).into_iter().flat_map(|p| match group(&p) { Some((_, inner)) => inner, None => vec![p] }).collect()
}

/// 人物预设和整理后的比一比：（去掉的, 新加的），一个个标签比（不分大小写，权重组拆开比），各按原来的顺序。
pub fn diff(preset: &str, now: &str) -> (Vec<String>, Vec<String>) {
    let (a, b) = (tags(preset), tags(now));
    let has = |v: &[String], t: &str| v.iter().any(|x| x.eq_ignore_ascii_case(t));
    (a.iter().filter(|t| !has(&b, t)).cloned().collect(), b.iter().filter(|t| !has(&a, t)).cloned().collect())
}

/// 在预设上改：`remove` 里的标签（不分大小写）从预设里拿掉，在权重组里的只拿掉那一个、组还在（拿空了整组不要）；
/// `add` 里预设没有的接在后面。没点名去掉的一律原样留着（顺序、权重组都不动）。
pub fn apply(preset: &str, remove: &[String], add: &[String]) -> String {
    let gone = |t: &str| remove.iter().any(|r| r.trim().eq_ignore_ascii_case(t.trim()));
    let mut out: Vec<String> = pieces(preset).into_iter().filter_map(|p| match group(&p) {
        Some((weight, inner)) => {
            let kept: Vec<String> = inner.into_iter().filter(|t| !gone(t)).collect();
            (!kept.is_empty()).then(|| format!("{weight}::{}::", kept.join(", ")))
        }
        None => (!gone(&p)).then_some(p),
    }).collect();
    let mut have = tags(preset);
    for t in add.iter().flat_map(|a| pieces(a)) {
        if have.iter().any(|h| h.eq_ignore_ascii_case(&t)) && !gone(&t) { continue; }
        have.push(t.clone());
        out.push(t);
    }
    out.join(", ")
}

/// 整理描述用的系统提示词：开头一句说手上是什么（一句描述 / 文档里的一段），后面是共用的规矩（NovelAI V4 起的写法加上配图的分镜规矩；只要 JSON）。
const PLAN_HEAD: &str = "You turn a short scene description (usually Chinese) into a NovelAI (V4.5 / V5 anime model) prompt plan for ONE illustration.";
const PASSAGE_HEAD: &str = "You turn a passage from a longer document (usually Chinese) into a NovelAI (V4.5 / V5 anime model) prompt plan for ONE illustration of it.";
/// 给一段配图时多的规矩：前后文只用来认人、认地方，画的只是这一段；人物只要画面里有的。
const PASSAGE_RULES: &str = r#"

ILLUSTRATING A PASSAGE
- <passage> is what to illustrate; <before>/<after> are ONLY for resolving who "he/she/they" are, where and when it happens, clothing and weather already established. Never illustrate events that occur only in <before>/<after>.
- Pick ONE frame from <passage>. Dialogue: the speaker and listener are in frame; show the emotion of the line, never render speech, text or speech bubbles. Headings and list items are structure, not things to draw. [image]/[table]/[code] markers are not picture content.
- "characters": include ONLY candidates visible in the chosen frame (max 3). "In passage" candidates are likely present; "nearby" candidates only if the passage clearly refers to them. Omit everyone else. Every candidate in the frame needs its key, even with nothing to change ({"remove": [], "add": []}): a missing key means not in the picture.
- <direction>, if present, is the user's instruction and overrides your choice of moment."#;
const RULES: &str = r#"Reply with JSON only, no code fence, no comments:
{"scene": "...", "characters": {"<name exactly as given>": {"remove": ["..."], "add": ["..."], "remove_negative": ["..."], "add_negative": ["..."]}}, "negative": "..."}

HOW NOVELAI READS PROMPTS
- It was trained on Danbooru tags plus short English captions. Write English Danbooru tags: lowercase, words separated by spaces (no underscores), tags separated by ", ". Use a very short English phrase only for something no tag can express. Never output Chinese.
- Earlier tags weigh more. Order "scene": shot size and camera angle first, then place and background, then time, weather and lighting, then mood and effects.
- Several people: "scene" describes only the shared picture (composition, place, light). Everything about one person goes into that person's entry, otherwise traits bleed between people.
- Interaction between two named characters: prefix the action tag in each entry: "source#" on the one doing it, "target#" on the one receiving it, "mutual#" when both do it (e.g. "source#hug" / "target#hug", "mutual#holding hands").
- Weight syntax "1.5::tag::" (stronger) and "-1::tag::" (remove) exists. Do not invent weights; but if the description already contains English tags or weight groups, keep them exactly as written.

NAMED CHARACTERS: ONLY LIST CHANGES TO THEIR PROFILE
- Each named character comes with profile tags (their saved look) and profile negative tags. Do not repeat the profile. Every profile tag you do not list in "remove" is kept exactly as written, weight groups included.
- "add": what this moment shows on this person: expression, pose, action, gaze, props in hand, clothing changes the scene requires. Often 3-10 tags.
- "remove": profile tags this picture contradicts, copied exactly as they appear in the profile (one tag each, even inside a weight group): a sword they always carry when the scene says they are unarmed, a uniform when they are in pajamas, "hair down" when the scene ties it up. Usually empty.
- NEVER remove identity tags: the character's own name tag, the series / franchise tag, the gender or count tag ("1girl", "1boy", "girl", ...), hair, eyes, skin, body, species features, signature marks. They are what makes the character recognisable and what the image model counts people by. Only remove a look tag when the description itself changes it (e.g. dyed hair).
- "add_negative": what must not appear on this person (e.g. "sword, weapon" when unarmed). "remove_negative": profile negative tags this picture needs (rare). Both usually empty.
- Every change is shown to the user next to the profile, so make only changes the picture needs.

WHAT YOU MUST NOT ADD
- Do not add character names, series names, artist tags, style or medium tags, quality or year tags, or "1girl"-style counts for named characters anywhere (all handled elsewhere). This is about adding: tags already in a character's profile stay, see above.
- No story, thoughts, sounds or anything that cannot be seen. Turn feelings into visible cues ("sad" -> "sad, tears, looking down").
- Unnamed extra people who must appear: put a count tag for them at the start of "scene" (e.g. "1boy", "2girls") plus their position or action; otherwise do not add people.

STORYBOARDING: PICK ONE GOOD FRAME
- Illustrate a single moment: the most telling, most visual instant of the description (the peak of the action or the emotional beat), not a summary of everything.
- Choose the shot by what the moment is about:
  emotion or a face -> "close-up" or "portrait" or "upper body";
  a gesture or pose -> "cowboy shot";
  whole-body action or outfit -> "full body";
  two people's relationship -> both in frame, "upper body" or "cowboy shot", with interaction tags;
  a place or atmosphere -> "wide shot", "scenery", the character small ("from distance").
- Camera angle only when it adds meaning: "from above" (vulnerable, overview), "from below" (powerful, imposing), "from side" or "profile" (quiet, thoughtful), "from behind" (facing the scenery, leaving), "dutch angle" (tension, unease). Eye level needs no tag.
- Gaze carries the story: "looking at viewer" for direct or intimate moments, "looking away", "looking afar", "looking back", "looking at another" for narrative ones.
- Fit the aspect ratio: portrait favours one character and vertical subjects (cowboy shot, full body); landscape favours scenery, width, or two people side by side (wide shot, upper body); square favours portrait or upper body, centred.
- Depth and framing when the setting allows: "depth of field", "blurry foreground", framing through a window or doorway.
- Light sets the mood: "backlighting" or "rim light" (dramatic, nostalgic), "sunset" / "golden hour" (warm), "night" with "moonlight", "dappled sunlight", "light rays"; rain brings "wet", "reflection".
- Keep it simple: one focal point, few props. Do not pile up conflicting tags (one shot size, one time of day).

LENGTH
- "scene": about 8-20 tags. Each character "add": about 3-10 tags for this moment. "negative": only things this picture specifically risks (e.g. "umbrella" if it must be absent), often empty.
- Every tag costs part of a shared token budget and dilutes the others; prefer fewer, precise tags."#;

fn shape((width, height): (u32, u32)) -> String {
    let shape = if width == height { "square" } else if height > width { "portrait (tall)" } else { "landscape (wide)" };
    format!("Aspect ratio: {shape}, {width}x{height}.\n")
}
/// 一组人物的预设：名字、外貌、排除。
fn profiles(user: &mut String, cast: &[&Character]) {
    for c in cast {
        let or = |s: &str| if s.trim().is_empty() { "(none)".to_owned() } else { s.trim().to_owned() };
        user.push_str(&format!("- {}\n  profile: {}\n  profile negative: {}\n", c.name, or(&c.look), or(&c.negative)));
    }
}

/// 交给文本模型的（系统提示词, 用户消息）。`cast` 是描述里写到的人物（名字、外貌），`(width, height)` 是要的尺寸。
pub fn plan_messages(cast: &[&Character], input: &str, size: (u32, u32)) -> (String, String) {
    let mut user = format!("Description:\n{}\n\n{}", input.trim(), shape(size));
    if cast.is_empty() { user.push_str("\nNo saved characters are named in this description.\n"); }
    else {
        user.push_str("\nNamed characters in this picture, with their profile tags and profile negative tags. List only what to remove from and add to these:\n");
        profiles(&mut user, cast);
        user.push_str("Use these exact names as the keys of \"characters\".\n");
    }
    (format!("{PLAN_HEAD} {RULES}"), user)
}

/// 文档里要配图的一段：文档标题、所在小节的标题、前文、这一段、后文、补充要求（都可以是空的）。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Passage { pub title: String, pub section: String, pub before: String, pub focus: String, pub after: String, pub direction: String }

/// 给一段配图时交给文本模型的（系统提示词, 用户消息）。`focus` 是这一段（和补充要求）里写到的人物，`nearby` 是只在前后文里出现的；
/// 都是候选，模型只挑画面里有的（`parse_plan` 的 `strict`）。
pub fn passage_messages(focus: &[&Character], nearby: &[&Character], p: &Passage, size: (u32, u32)) -> (String, String) {
    let mut user = String::new();
    if !p.title.trim().is_empty() { user.push_str(&format!("Document: {}\n", p.title.trim())); }
    if !p.section.trim().is_empty() { user.push_str(&format!("Section: {}\n", p.section.trim())); }
    for (tag, text) in [("before", &p.before), ("passage", &p.focus), ("after", &p.after), ("direction", &p.direction)] {
        if !text.trim().is_empty() { user.push_str(&format!("\n<{tag}>\n{}\n</{tag}>\n", text.trim())); }
    }
    user.push('\n');
    user.push_str(&shape(size));
    if focus.is_empty() && nearby.is_empty() { user.push_str("\nNo saved characters are named in or near this passage.\n"); }
    else {
        user.push_str("\nCandidate characters, with their profile tags and profile negative tags. Include only those visible in the frame; for them list only what to remove from and add to these.\n");
        if !focus.is_empty() { user.push_str("In passage:\n"); profiles(&mut user, focus); }
        if !nearby.is_empty() { user.push_str("Nearby:\n"); profiles(&mut user, nearby); }
        user.push_str("Use these exact names as the keys of \"characters\".\n");
    }
    (format!("{PASSAGE_HEAD} {RULES}{PASSAGE_RULES}"), user)
}

/// 读模型回的 JSON（前后有别的字、包在代码块里也认）；人物按名字对上 `cast`（不分大小写），对不上的不要。读不出来是 `None`。
/// 人物那一项是 `{"remove", "add", "remove_negative", "add_negative"}`，在预设上改（`apply`），预设里没点名去掉的都留着；
/// 漏了的照预设；`strict`（给一段配图，人物是候选）时漏了的就是不在画面里，不要，最多三个。旧的写法 `{"prompt", "negative"}`（整段重写）也还认。
pub fn parse_plan(reply: &str, cast: &[&Character], strict: bool) -> Option<Plan> {
    let (a, z) = (reply.find('{')?, reply.rfind('}')?);
    let v: Value = serde_json::from_str(reply.get(a..=z)?).ok()?;
    let tags = |v: Option<&Value>| match v {
        Some(Value::String(s)) => concat(&[s]),
        Some(Value::Array(a)) => a.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", "),
        _ => String::new(),
    };
    let given = v.get("characters").and_then(Value::as_object);
    let list = |o: &Value, k: &str| -> Vec<String> { match o.get(k) {
        Some(Value::Array(a)) => a.iter().filter_map(Value::as_str).map(str::to_owned).collect(),
        Some(Value::String(s)) => pieces(s),
        _ => vec![],
    } };
    let find = |c: &Character| given.and_then(|m| m.iter().find(|(k, _)| k.trim().to_lowercase() == c.name.trim().to_lowercase()).map(|(_, v)| v));
    let characters = cast.iter().filter(|c| !strict || find(c).is_some()).take(if strict { 3 } else { usize::MAX }).map(|c| {
        let found = find(c);
        let (prompt, negative) = match found {
            Some(o @ Value::Object(_)) if o.get("prompt").is_some() => (tags(o.get("prompt")), o.get("negative").map(|n| tags(Some(n)))),
            Some(o @ Value::Object(_)) => (apply(&c.look, &list(o, "remove"), &list(o, "add")), Some(apply(&c.negative, &list(o, "remove_negative"), &list(o, "add_negative")))),
            _ => (String::new(), None),
        };
        let prompt = if prompt.is_empty() { concat(&[&c.look]) } else { prompt };
        Cast { name: c.name.clone(), tags: prompt, negative: negative.unwrap_or_else(|| concat(&[&c.negative])) }
    }).collect();
    Some(Plan { scene: tags(v.get("scene")), characters, negative: tags(v.get("negative")) })
}

/// 几段逗号分开的字接起来：空的跳过，别的原样（带权重的 `2.0::…::` 不能拆开去重）。
fn concat(parts: &[&str]) -> String { parts.iter().map(|s| s.trim().trim_matches(',').trim()).filter(|s| !s.is_empty()).collect::<Vec<_>>().join(", ") }

/// 拼起来：主提示词 = 人数 + 画风正向 + 描述 + 质量词；负面 = 画风负面 + 质量负向。`quality` 是（正向, 负向）。
/// 写到的人物各占一条分角色提示词，内容是外貌标签；排除标签是同一位置的分角色负面词。外貌空着的不占（只有排除也不占）。
/// 有 `plan`（整理过的）时描述换成它的画面标签（里面没名字的人的人数并进人数），每个人的提示词和排除换成它整理后的（人数也按整理后的数），
/// 画面的排除接在画风负面后面。整理结果里没有的人照预设。`cast` 是出场的人（一般是 `mentioned` 出来的；给一段配图时是模型挑的）。
pub fn assemble(style: Option<&Style>, cast: &[&Character], input: &str, plan: Option<&Plan>, (quality, quality_negative): (&str, &str)) -> Prompt {
    let (scene, more) = plan.map_or_else(|| (input.to_owned(), [0; 3]), |p| take_counts(&p.scene));
    let cast: Vec<Cast> = cast.iter().map(|c| {
        plan.and_then(|p| p.characters.iter().find(|x| x.name == c.name)).cloned()
            .unwrap_or_else(|| Cast { name: c.name.clone(), tags: join(&[&c.look]), negative: concat(&[&c.negative]) })
    }).filter(|c| !c.tags.trim().is_empty()).collect();
    let count = counts(&cast.iter().map(|c| c.tags.as_str()).collect::<Vec<_>>(), more);
    Prompt {
        base: concat(&[&count, style.map_or("", |s| s.positive.as_str()), &scene, quality]),
        negative: concat(&[style.map_or("", |s| s.negative.as_str()), plan.map_or("", |p| p.negative.as_str()), quality_negative]),
        characters: cast,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ch(id: &str, name: &str, aliases: &[&str], look: &str) -> Character {
        Character { id: id.into(), name: name.into(), aliases: aliases.iter().map(|s| (*s).into()).collect(), look: look.into(), ..Default::default() }
    }

    #[test]
    fn mentions_prefer_longest_and_keep_order() {
        let cast = [ch("a", "林夕", &["夕", "小夕"], "girl, black hair"), ch("b", "夕", &[], "boy"), ch("c", "Anna", &["Ann"], "girl, blonde hair"), ch("d", "沈舟", &[], "")];
        let names = |s: &str| mentioned(&cast, s).iter().map(|c| c.id.clone()).collect::<Vec<_>>();
        assert_eq!(names("林夕坐在窗边看书"), ["a"]);
        assert_eq!(names("anna 和林夕，还有夕"), ["c", "a", "b"]);
        assert_eq!(names("窗外下雨"), Vec::<String>::new());
        assert_eq!(names("沈舟"), ["d"]);
    }

    #[test]
    fn assemble_puts_looks_in_character_prompts() {
        let style = &builtin(false)[2];
        let mut cast = [ch("a", "林夕", &[], "girl, black hair, school uniform"), ch("b", "小雨", &[], "1.4::lyna, 1girl, short hair::"), ch("c", "沈舟", &[], "boy, glasses"), ch("d", "猫", &[], "")];
        cast[1].negative = " long hair, 1.5::ponytail:: ".into();
        let p = assemble(Some(style), &mentioned(&cast, "林夕和小雨、沈舟在窗边，猫在睡觉"), " 林夕和小雨、沈舟在窗边，猫在睡觉 ", None, ("best_quality", "lowres, 2.0::text, words::"));
        assert!(p.base.starts_with("2girls, 1boy, watercolor (medium)"), "{}", p.base);
        assert!(p.base.ends_with("林夕和小雨、沈舟在窗边，猫在睡觉, best_quality"), "{}", p.base);
        assert_eq!(p.negative, "3d, realistic, high contrast, lowres, 2.0::text, words::");
        let cast = |n: &str, t: &str, x: &str| Cast { name: n.into(), tags: t.into(), negative: x.into() };
        assert_eq!(p.characters, [cast("林夕", "girl, black hair, school uniform", ""), cast("小雨", "1.4::lyna, 1girl, short hair::", "long hair, 1.5::ponytail::"), cast("沈舟", "boy, glasses", "")]);
        let plain = assemble(None, &[], "a quiet room", None, ("", "lowres"));
        assert_eq!((plain.base.as_str(), plain.negative.as_str(), plain.characters.len()), ("a quiet room", "lowres", 0));
        assert_eq!(join(&["a, b，A", "\nc, b"]), "a, b, c");
    }

    #[test]
    fn plans_are_read_and_assembled() {
        let mut cast = [ch("a", "林夕", &[], "girl, black hair, katana"), ch("b", "Anna", &[], "boy, blonde hair")];
        cast[0].negative = "long hair".into();
        let named: Vec<&Character> = cast.iter().collect();
        let (system, user) = plan_messages(&named, "林夕抱住了 Anna", (1216, 832));
        assert!(system.contains("source#") && user.contains("landscape") && user.contains("- 林夕\n  profile: girl, black hair, katana\n  profile negative: long hair"), "{user}");
        let reply = "好的：\n```json\n{\"scene\": \"1boy, upper body, rain, 1.3::backlighting, rim light::\", \"characters\": {\"林夕\": {\"prompt\": \"girl, black hair, source#hug, crying\", \"negative\": [\"long hair\", \"weapon\"]}, \"anna\": \"\", \"别人\": \"x\"}, \"negative\": \"umbrella\"}\n```";
        let plan = parse_plan(reply, &named, false).unwrap();
        let c = |n: &str, t: &str, x: &str| Cast { name: n.into(), tags: t.into(), negative: x.into() };
        assert_eq!(plan.characters, [c("林夕", "girl, black hair, source#hug, crying", "long hair, weapon"), c("Anna", "boy, blonde hair", "")]);
        assert_eq!(diff(&cast[0].look, &plan.characters[0].tags), (vec!["katana".to_owned()], vec!["source#hug".to_owned(), "crying".to_owned()]));
        let p = assemble(None, &named, "林夕抱住了 Anna", Some(&plan), ("best_quality", "lowres"));
        assert_eq!(p.base, "1girl, 2boys, upper body, rain, 1.3::backlighting, rim light::, best_quality");
        assert_eq!(p.negative, "umbrella, lowres");
        assert_eq!(p.characters, plan.characters);
        assert!(parse_plan("no json here", &named, false).is_none());
        assert_eq!(pieces("a, 1.5::b, c::, d"), ["a", "1.5::b, c::", "d"]);
    }

    #[test]
    fn changes_apply_inside_weight_groups() {
        let look = "1.4::lyna the light charmer, yu-gi-oh!, 1girl, white short hair, slender rapier::, smile";
        let mut lyna = ch("a", "莱娜", &[], look);
        lyna.negative = "long hair".into();
        let named = [&lyna];
        let reply = r#"{"scene": "wide shot, from behind", "characters": {"莱娜": {"remove": ["Slender Rapier", "not in profile"], "add": ["standing", "facing away", "smile"], "add_negative": ["sword"]}}, "negative": ""}"#;
        let plan = parse_plan(reply, &named, false).unwrap();
        let c = &plan.characters[0];
        assert_eq!(c.tags, "1.4::lyna the light charmer, yu-gi-oh!, 1girl, white short hair::, smile, standing, facing away");
        assert_eq!(c.negative, "long hair, sword");
        assert_eq!(diff(look, &c.tags), (vec!["slender rapier".to_owned()], vec!["standing".to_owned(), "facing away".to_owned()]));
        assert_eq!(apply("2::a::, b", &["a".into()], &[]), "b");
        assert_eq!(apply("", &[], &["x, 1.2::y::".into()]), "x, 1.2::y::");
        assert_eq!(tags("a, -1::b, c::, 1girl"), ["a", "b", "c", "1girl"]);
        let p = assemble(None, &named, "莱娜", Some(&plan), ("", ""));
        assert!(p.base.starts_with("1girl, wide shot"), "{}", p.base);
    }

    #[test]
    fn passages_pick_only_visible_candidates() {
        let cast = [ch("a", "林夕", &[], "girl, black hair"), ch("b", "沈舟", &[], "boy, glasses"), ch("c", "Anna", &[], "girl, blonde hair")];
        let passage = Passage { title: "雨夜".into(), section: "第三章".into(), before: "沈舟撑着伞走了。".into(), focus: "她站在窗边，没有回头。".into(), direction: "要背影".into(), ..Default::default() };
        let (system, user) = passage_messages(&[&cast[0]], &[&cast[1]], &passage, (832, 1216));
        assert!(system.starts_with(PASSAGE_HEAD) && system.contains("ILLUSTRATING A PASSAGE") && system.contains("NAMED CHARACTERS") && system.contains("a missing key means not in the picture"), "{system}");
        assert!(user.starts_with("Document: 雨夜\nSection: 第三章\n\n<before>\n沈舟撑着伞走了。\n</before>\n\n<passage>\n她站在窗边，没有回头。\n</passage>\n\n<direction>\n要背影\n</direction>\n\nAspect ratio: portrait"), "{user}");
        assert!(!user.contains("<after>") && user.contains("In passage:\n- 林夕") && user.contains("Nearby:\n- 沈舟"), "{user}");
        let named: Vec<&Character> = cast.iter().collect();
        let reply = r#"{"scene": "from behind, window", "characters": {"林夕": {"add": ["standing"]}}}"#;
        let strict = parse_plan(reply, &named, true).unwrap();
        assert_eq!(strict.characters.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["林夕"]);
        assert_eq!(strict.characters[0].tags, "girl, black hair, standing");
        assert_eq!(parse_plan(reply, &named, false).unwrap().characters.len(), 3);
        let picked: Vec<&Character> = named.iter().copied().filter(|c| strict.characters.iter().any(|x| x.name == c.name)).collect();
        let p = assemble(None, &picked, "", Some(&strict), ("", ""));
        assert_eq!((p.base.as_str(), p.characters.len()), ("1girl, from behind, window", 1));
        let (_, none) = passage_messages(&[], &[], &Passage { focus: "窗外下雨".into(), ..Default::default() }, (1024, 1024));
        assert!(none.starts_with("\n<passage>") && none.contains("No saved characters are named in or near"), "{none}");
    }

    #[test]
    fn library_round_trips() {
        let dir = std::env::temp_dir().join(format!("fdl-ai-imagine-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(Library::load(&dir), Library::default());
        let lib = Library { styles: vec![Style { id: "s1".into(), name: "我的".into(), positive: "flat color".into(), negative: "3d".into() }], style: "s1".into(), characters: vec![Character { negative: "long hair".into(), avatar: Some("builtin-03".into()), ..ch("c1", "林夕", &["小夕"], "girl") }, ch("c2", "沈舟", &[], "boy")], quality: Some("masterpiece".into()), quality_negative: None, optimize: true };
        lib.save(&dir).unwrap();
        let back = Library::load(&dir);
        assert_eq!(back, lib);
        assert_eq!(back.current_style(false).unwrap().name, "我的");
        assert_eq!((back.quality(), back.quality_negative()), ("masterpiece", QUALITY_NEGATIVE));
        let mut q = back.clone();
        q.set_quality(false, &format!(" {QUALITY} "));
        q.set_quality(true, "");
        assert_eq!((q.quality.as_deref(), q.quality_negative.as_deref()), (None, Some("")));
        assert_eq!(back.find_style("builtin-lineart", true).unwrap().name, "Line Art");
        assert!(back.find_style("gone", false).is_none() && back.find_style("", false).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
