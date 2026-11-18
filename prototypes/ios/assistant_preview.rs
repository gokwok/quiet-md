//! 试听：放音色自带的样音（ElevenLabs 托管的公开文件，请求不带 API Key，不花额度），用 `AVAudioPlayer`。
//! 同一时间只放一个；换页、移除角色、换 Key 时停下。下载过的样音留在内存里，再听不用重下。
use std::{cell::{Cell, RefCell}, future::Future, ptr::null_mut, time::{Duration, Instant}};

use gpui::*;
use objc2::{class, msg_send, runtime::AnyObject};

use crate::{app::Workspace, settings::tr, theme::Theme, ui};

#[link(name = "AVFoundation", kind = "framework")]
unsafe extern "C" { static AVAudioSessionCategoryPlayback: *const AnyObject; }

#[derive(Clone, Copy, PartialEq)]
pub enum Phase { Idle, Loading, Playing }

struct Now { id: String, playing: bool, started: Instant, length: f32 }

const CACHE_MAX: usize = 24;

thread_local! {
    static NOW: RefCell<Option<Now>> = const { RefCell::new(None) };
    static PLAYER: Cell<*mut AnyObject> = const { Cell::new(null_mut()) };
    static EPOCH: Cell<u64> = const { Cell::new(0) };
    static CACHE: RefCell<Vec<(String, Vec<u8>)>> = const { RefCell::new(Vec::new()) };
}

pub fn phase(id: &str) -> Phase {
    NOW.with(|n| match &*n.borrow() { Some(now) if now.id == id => if now.playing { Phase::Playing } else { Phase::Loading }, _ => Phase::Idle })
}

/// 这一次播放的标识：动画用它当键，换一次播放就从头再来。
pub fn epoch() -> u64 { EPOCH.with(Cell::get) }

/// 正在放的这段：已经放了几秒、一共几秒。
pub fn timing(id: &str) -> Option<(f32, f32)> {
    NOW.with(|n| n.borrow().as_ref().filter(|now| now.id == id && now.playing).map(|now| (now.started.elapsed().as_secs_f32(), now.length)))
}

pub fn stop() {
    EPOCH.with(|e| e.set(e.get() + 1));
    NOW.with(|n| *n.borrow_mut() = None);
    let player = PLAYER.with(|p| p.replace(null_mut()));
    if player.is_null() { return; }
    // 不在主线程上同步停用音频会话（系统提示会卡界面）；会话留着，下次试听直接用。
    unsafe {
        let _: () = msg_send![player, stop];
        let _: () = msg_send![player, release];
    }
}

/// 点一下放，再点一下停。`source` 给出样音的地址（可能要先问 ElevenLabs）。
pub fn toggle(id: String, source: impl Future<Output = Option<String>> + 'static, cx: &mut Context<Workspace>) {
    super::super::glass::haptic();
    let again = phase(&id) != Phase::Idle;
    stop();
    if again { return cx.notify(); }
    let epoch = epoch();
    NOW.with(|n| *n.borrow_mut() = Some(Now { id: id.clone(), playing: false, started: Instant::now(), length: 0. }));
    cx.notify();
    cx.spawn(async move |this, cx| {
        let cached = CACHE.with(|c| c.borrow().iter().find(|(k, _)| *k == id).map(|(_, b)| b.clone()));
        let bytes = match cached {
            Some(bytes) => Ok(bytes),
            None => match source.await.filter(|url| url.starts_with("https://")) {
                None => Err(tr("这个音色没有试听样音", "This voice has no preview sample")),
                Some(url) => super::roles::fetch_bytes(&url, None).await.unwrap().unwrap().map_err(|_| tr("无法载入试听样音", "Could not load the preview sample")),
            },
        };
        if EPOCH.with(Cell::get) != epoch { return; }
        let _ = this.update(cx, |w, cx| {
            let played = bytes.and_then(|bytes| {
                let length = play(&bytes).ok_or(tr("无法播放试听样音", "Could not play the preview sample"))?;
                CACHE.with(|c| { let mut c = c.borrow_mut(); c.retain(|(k, _)| *k != id); if c.len() >= CACHE_MAX { c.remove(0); } c.push((id.clone(), bytes)); });
                Ok(length)
            });
            match played {
                Ok(length) => { NOW.with(|n| *n.borrow_mut() = Some(Now { id, playing: true, started: Instant::now(), length })); watch(epoch, cx); }
                Err(text) => { stop(); w.show_toast_kind(ui::ToastKind::Error, text, cx); }
            }
            cx.notify();
        });
    }).detach();
}

/// 放完了就回到没在放；中途换了一段的话这里什么也不做。
fn watch(epoch: u64, cx: &mut Context<Workspace>) {
    cx.spawn(async move |this, cx| loop {
        cx.background_executor().timer(Duration::from_millis(200)).await;
        if EPOCH.with(Cell::get) != epoch { return; }
        let player = PLAYER.with(Cell::get);
        let playing: bool = !player.is_null() && unsafe { msg_send![player, isPlaying] };
        if !playing { stop(); let _ = this.update(cx, |_, cx| cx.notify()); return; }
    }).detach();
}

/// 开始播放，返回总长（秒）。播放类别：静音开关打开时也有声音，这是用户自己点的试听。
fn play(bytes: &[u8]) -> Option<f32> {
    unsafe {
        let session: *mut AnyObject = msg_send![class!(AVAudioSession), sharedInstance];
        let _: bool = msg_send![session, setCategory: AVAudioSessionCategoryPlayback, error: null_mut::<*mut AnyObject>()];
        let _: bool = msg_send![session, setActive: true, error: null_mut::<*mut AnyObject>()];
        let data: *mut AnyObject = msg_send![class!(NSData), dataWithBytes: bytes.as_ptr().cast::<std::ffi::c_void>(), length: bytes.len()];
        let player: *mut AnyObject = msg_send![class!(AVAudioPlayer), alloc];
        let player: *mut AnyObject = msg_send![player, initWithData: data, error: null_mut::<*mut AnyObject>()];
        if player.is_null() { return None; }
        let _: bool = msg_send![player, prepareToPlay];
        let started: bool = msg_send![player, play];
        if !started { let _: () = msg_send![player, release]; return None; }
        PLAYER.with(|p| p.set(player));
        let length: f64 = msg_send![player, duration];
        Some(length.max(0.1) as f32)
    }
}

/// 正在放时跳动的几根竖条。
pub fn bars(key: &'static str, height: f32, color: Hsla) -> AnyElement {
    const SHAPE: [(f32, f32); 4] = [(0.86, 0.), (0.64, 0.35), (1.04, 0.6), (0.74, 0.2)];
    let w = (height / 6.).max(2.);
    div().h(px(height)).flex().items_center().gap(px(w * 0.7))
        .children(SHAPE.iter().enumerate().map(move |(i, &(period, phase))| {
            div().w(px(w)).h(px(height)).rounded_full().bg(color)
                .with_animation((key, i), Animation::new(Duration::from_secs_f32(period)).repeat(), move |d, t| {
                    let k = 0.5 - 0.5 * ((t + phase) * std::f32::consts::TAU).cos();
                    d.h(px(height * (0.28 + 0.72 * k)))
                })
        })).into_any_element()
}

/// 播放进度：一条细线，长度直接按已放的秒数算。放的时候旁边的竖条一直在动，每帧都会重画，线跟着走。
pub fn progress(id: &str, width: f32) -> Option<AnyElement> {
    let (at, length) = timing(id)?;
    let done = (at / length).clamp(0., 1.);
    Some(div().w(px(width)).h(px(3.)).rounded_full().bg(Theme::text().opacity(0.12)).overflow_hidden()
        .child(div().h_full().rounded_full().bg(Theme::text()).w(px(width * done)))
        .into_any_element())
}
