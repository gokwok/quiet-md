//! 背景设置的在线图库。搜索请求只发出使用者输入的关键词，选中的照片随文档库保存。
use std::cell::RefCell;
use std::hash::{Hash, Hasher};
use std::path::Path;
use std::time::Duration;

use gpui::*;
use reqwest::{Client, Url};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{Page, PAGE, TAB};
use crate::app::Workspace;
use crate::ios::panel::{Node, PhotoTile};
use crate::settings::tr;
use crate::store::Store;

const DEFAULT_QUERY: &str = "forest sunrise";
const API: &str = "https://commons.wikimedia.org/w/api.php";
const USER_AGENT: &str = "Margin/0.1 (https://quiet.md/)";

#[derive(Clone, Deserialize, Serialize)]
pub(super) struct Credit { pub(super) title: String, pub(super) author: String, pub(super) license: String, pub(super) license_url: String, pub(super) source: String }

impl Credit {
    fn caption(&self) -> String { format!("{} · {}", self.author, self.license) }
}

#[derive(Clone)]
struct Photo { id: u64, file: String, thumb: String, preview: String, credit: Credit }

enum Results { Loading, Ready(Vec<Photo>), Failed }

struct Gallery { query: String, results: Results, revision: u64, importing: bool }
thread_local! {
    static GALLERY: RefCell<Gallery> = const { RefCell::new(Gallery {
        query: String::new(), results: Results::Loading, revision: 0, importing: false,
    }) };
}

fn in_gallery() -> bool { PAGE.with(|p| p.get()) == Some(Page::PhotoLibrary) }

pub(super) fn credit(src: &str) -> Option<String> {
    let file = src.strip_prefix("file:")?;
    let path = crate::ios::style::photos_dir().join(file).with_extension("source.json");
    let credit: Credit = serde_json::from_slice(&std::fs::read(path).ok()?).ok()?;
    Some(credit.caption())
}

pub(super) fn nodes() -> Vec<Node> {
    GALLERY.with(|state| {
        let state = state.borrow();
        let mut v = vec![
            Node::Header { title: tr("在线照片", "Online photos").into(), warn: false, reset: None, close: "page:background".into() },
            Node::SearchPhotos { prompt: tr("搜索照片", "Search photos").into(), query: state.query.clone() },
        ];
        let content = match &state.results {
            Results::Loading => vec![Node::Empty("photo", tr("正在加载照片…", "Loading photos…").into())],
            Results::Failed => vec![
                Node::Empty("wifi.exclamationmark", tr("无法连接照片图库", "Couldn’t reach the photo library").into()),
                Node::Actions(vec![("arrow.clockwise", tr("重试", "Retry").into(), "bg-retry".into())]),
            ],
            Results::Ready(photos) if photos.is_empty() => vec![Node::Empty("magnifyingglass", tr("没有找到照片", "No photos found").into())],
            Results::Ready(photos) => {
                let items = photos.iter().map(|p| PhotoTile {
                    path: p.preview.clone(), title: p.credit.title.clone(), credit: Some(p.credit.caption()), picked: false,
                    act: format!("bg-use-photo:{}", p.id),
                }).collect();
                let label = if state.importing { tr("正在保存照片…", "Saving photo…") } else { "Wikimedia Commons" };
                vec![Node::Caption(label.into()), Node::Photos { items, library: true }]
            }
        };
        let (_, safe, viewport) = crate::ios::layout();
        let width = f32::from(viewport.width - px(26.));
        let available = f32::from(viewport.height - safe.top - safe.bottom.max(px(34.))) - 12.;
        let max = available.min(f32::from(viewport.height) * 0.83) - crate::ios::panel::height(&v, width);
        v.push(Node::FixedScroll { nodes: content, height: f64::from(max.max(1.)) });
        v
    })
}

pub(super) fn load(query: &str, w: &Workspace, window: &mut Window, cx: &mut Context<Workspace>) {
    let query: String = query.trim().chars().take(100).collect();
    let request = if query.is_empty() { DEFAULT_QUERY.to_string() } else { query.clone() };
    let revision = GALLERY.with(|g| {
        let mut g = g.borrow_mut();
        g.query = query;
        g.revision = g.revision.wrapping_add(1);
        g.results = Results::Loading;
        g.importing = false;
        g.revision
    });
    let document = w.current.clone();
    let root = Store::root_dir();
    cx.notify();
    let work = crate::ios::assistant::llm::runtime().spawn(search(request));
    cx.spawn_in(window, async move |this, cx| {
        let found = work.await.ok().and_then(Result::ok);
        let _ = this.update_in(cx, |w, _, cx| {
            if !in_gallery() || w.current != document || Store::root_dir() != root { return; }
            GALLERY.with(|g| {
                let mut g = g.borrow_mut();
                if g.revision == revision { g.results = found.map_or(Results::Failed, Results::Ready); }
            });
            cx.notify();
        });
    }).detach();
}

pub(super) fn retry(w: &Workspace, window: &mut Window, cx: &mut Context<Workspace>) {
    let query = GALLERY.with(|g| g.borrow().query.clone());
    load(&query, w, window, cx);
}

pub(super) fn apply(id: u64, w: &Workspace, window: &mut Window, cx: &mut Context<Workspace>) {
    let job = GALLERY.with(|g| {
        let mut g = g.borrow_mut();
        if g.importing { return None; }
        let Results::Ready(photos) = &g.results else { return None; };
        let photo = photos.iter().find(|p| p.id == id)?.clone();
        g.importing = true;
        Some((g.revision, photo))
    });
    let Some((revision, photo)) = job else { return; };
    let document = w.current.clone();
    let root = Store::root_dir();
    let title = photo.file.clone();
    let work = crate::ios::assistant::llm::runtime().spawn(full_photo(title));
    cx.notify();
    cx.spawn_in(window, async move |this, cx| {
        let bytes = work.await.ok().and_then(Result::ok);
        let _ = this.update_in(cx, |w, _, cx| {
            if !in_gallery() || w.current != document || Store::root_dir() != root { return; }
            let valid = GALLERY.with(|g| {
                let mut g = g.borrow_mut();
                if g.revision != revision { return false; }
                g.importing = false;
                true
            });
            if !valid { return; }
            cx.notify();
            let Some(src) = bytes.and_then(|b| store_photo(&b, &photo.credit).ok()) else {
                return w.show_toast_kind(crate::ui::ToastKind::Error, tr("没能存下这张图片", "Couldn’t save the image"), cx);
            };
            if super::set_photo(src.clone(), w, cx) {
                PAGE.with(|p| p.set(Some(Page::Background)));
                TAB.with(|t| t.set(Some(3)));
                cx.notify();
            } else if let Some(file) = src.strip_prefix("file:") {
                let path = crate::ios::style::photos_dir().join(file);
                let _ = std::fs::remove_file(path.with_extension("source.json"));
                let _ = std::fs::remove_file(path);
            }
        });
    }).detach();
}

pub(super) fn prompt() {
    use crate::ios::assistant::{Field, prompt};
    let current = GALLERY.with(|g| g.borrow().query.clone());
    prompt(tr("搜索照片", "Search photos"), Some("Wikimedia Commons"), tr("照片关键词", "Photo keywords"), &current, Field::Search, None, |query| {
        if let Some(query) = query {
            crate::ios::menu::dispatch(move |w, window, cx| if in_gallery() { load(&query, w, window, cx); });
        }
    });
}

fn client() -> anyhow::Result<Client> {
    Ok(Client::builder().https_only(true).user_agent(USER_AGENT).timeout(Duration::from_secs(12))
        .redirect(reqwest::redirect::Policy::none()).build()?)
}

async fn download(http: &Client, url: Url, limit: usize) -> anyhow::Result<Vec<u8>> {
    let mut response = http.get(url).send().await?.error_for_status()?;
    anyhow::ensure!(response.content_length().is_none_or(|n| n <= limit as u64), "response is too large");
    let mut data = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        anyhow::ensure!(data.len().saturating_add(chunk.len()) <= limit, "response is too large");
        data.extend_from_slice(&chunk);
    }
    Ok(data)
}

async fn query(http: &Client, args: &[(&str, &str)]) -> anyhow::Result<Value> {
    let mut url = Url::parse(API)?;
    url.query_pairs_mut().extend_pairs([("action", "query"), ("format", "json"), ("formatversion", "2"), ("prop", "imageinfo")]).extend_pairs(args.iter().copied());
    let result: Value = serde_json::from_slice(&download(http, url, 2_000_000).await?)?;
    anyhow::ensure!(result.get("error").is_none(), "Commons returned a search error");
    Ok(result)
}

fn image_url(source: &str) -> anyhow::Result<Url> {
    let mut url = Url::parse(source)?;
    anyhow::ensure!(url.scheme() == "https" && matches!(url.host_str(), Some("upload.wikimedia.org" | "thumb.wikimedia.org")), "unexpected image host");
    url.set_query(None);
    Ok(url)
}

fn validate_jpeg(data: &[u8]) -> anyhow::Result<()> {
    use image::{ImageFormat, ImageReader};
    let (w, h) = ImageReader::with_format(std::io::Cursor::new(data), ImageFormat::Jpeg).into_dimensions()?;
    anyhow::ensure!(u64::from(w) * u64::from(h) <= 24_000_000, "image is too large");
    Ok(())
}

async fn search(term: String) -> anyhow::Result<Vec<Photo>> {
    use futures::{StreamExt, stream};
    let http = client()?;
    let phrase = format!("{term} filemime:image/jpeg");
    let result = query(&http, &[("generator", "search"), ("gsrsearch", &phrase), ("gsrnamespace", "6"),
        ("gsrlimit", "14"), ("iiprop", "url|extmetadata"), ("iiurlwidth", "320"),
        ("iiextmetadatafilter", "Artist|LicenseShortName|LicenseUrl")]).await?;
    let mut found: Vec<_> = result["query"]["pages"].as_array().into_iter().flatten().filter_map(photo).collect();
    found.sort_by_key(|(rank, _)| *rank);
    let has_results = !found.is_empty();
    let found: Vec<_> = stream::iter(found.into_iter().map(|(_, mut photo)| {
        let http = http.clone();
        async move {
            photo.preview = cache_thumbnail(&http, &photo.thumb).await.ok()?;
            Some(photo)
        }
    })).buffered(4).filter_map(futures::future::ready).collect().await;
    anyhow::ensure!(!has_results || !found.is_empty(), "thumbnails unavailable");
    Ok(found)
}

fn photo(page: &Value) -> Option<(u64, Photo)> {
    let info = &page["imageinfo"][0];
    let meta = &info["extmetadata"];
    let license = meta["LicenseShortName"]["value"].as_str()?;
    if !(license == "Public domain" || license.starts_with("CC0") || license.starts_with("CC BY ") || license.starts_with("CC BY-SA ")) { return None; }
    let author_html = meta["Artist"]["value"].as_str().unwrap_or_default();
    let author = regex::Regex::new("<[^>]*>").ok()?.replace_all(author_html, "");
    let author = author.split_whitespace().collect::<Vec<_>>().join(" ");
    if author.is_empty() && license.starts_with("CC BY") { return None; }
    let file = page["title"].as_str()?.to_string();
    let title = file.strip_prefix("File:").unwrap_or(&file);
    let title = title.trim_end_matches(".jpg").trim_end_matches(".JPG").trim_end_matches(".jpeg").to_string();
    Some((page["index"].as_u64().unwrap_or(u64::MAX), Photo {
        id: page["pageid"].as_u64()?, file, thumb: info["thumburl"].as_str()?.into(), preview: String::new(),
        credit: Credit { title, author: if author.is_empty() { "Wikimedia Commons".into() } else { author },
            license: license.into(), license_url: meta["LicenseUrl"]["value"].as_str().unwrap_or_default().into(),
            source: info["descriptionurl"].as_str()?.into() },
    }))
}

async fn cache_thumbnail(http: &Client, source: &str) -> anyhow::Result<String> {
    let url = image_url(source)?;
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    url.hash(&mut hasher);
    let path = std::env::temp_dir().join("quiet-commons-thumbnails").join(format!("{:016x}.jpg", hasher.finish()));
    if !path.is_file() {
        let data = download(http, url, 2_000_000).await?;
        validate_jpeg(&data)?;
        write_atomic(&path, &data)?;
    }
    Ok(path.to_string_lossy().into_owned())
}

async fn full_photo(title: String) -> anyhow::Result<Vec<u8>> {
    let http = client()?;
    let result = query(&http, &[("titles", &title), ("iiprop", "url"), ("iiurlwidth", "1280")]).await?;
    let info = &result["query"]["pages"][0]["imageinfo"][0];
    let source = info["thumburl"].as_str().or_else(|| info["url"].as_str()).ok_or_else(|| anyhow::anyhow!("photo no longer available"))?;
    let data = download(&http, image_url(source)?, 8_000_000).await?;
    validate_jpeg(&data)?;
    Ok(data)
}

fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() { std::fs::create_dir_all(dir)?; }
    let tmp = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
    let result = std::fs::write(&tmp, bytes).and_then(|()| std::fs::rename(&tmp, path));
    if result.is_err() { let _ = std::fs::remove_file(tmp); }
    result
}

pub(super) fn store_photo(bytes: &[u8], credit: &Credit) -> anyhow::Result<String> {
    let file = format!("{}.jpg", uuid::Uuid::new_v4());
    let path = crate::ios::style::photos_dir().join(&file);
    let credit_path = path.with_extension("source.json");
    write_atomic(&credit_path, &serde_json::to_vec_pretty(credit)?)?;
    if let Err(err) = write_atomic(&path, bytes) {
        let _ = std::fs::remove_file(credit_path);
        return Err(err.into());
    }
    Ok(format!("file:{file}"))
}
