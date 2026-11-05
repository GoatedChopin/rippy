// Release builds on Windows open without a console window behind the app.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::collections::HashSet;
use std::sync::Arc;

use gpui_kit::assets::Assets;
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::clipboard::Clipboard;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Sizable as _, StyledExt as _,
    WindowExt as _, h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use regex::Regex;
use rustypipe::client::RustyPipe;

mod api;
mod http;
mod tools;
mod youtube;
mod yourip;

use tools::Tool;
use youtube::SearchHit;
use yourip::{Executor, Media};

// The default assets only carry the icons gpui-kit's own components use.
gpui_kit::assets::icon_assets!(ExtraIcons, [Download]);

struct AppAssets;

impl AssetSource for AppAssets {
    fn load(&self, path: &str) -> Result<Option<std::borrow::Cow<'static, [u8]>>> {
        if let Some(bytes) = ExtraIcons.load(path)? {
            return Ok(Some(bytes));
        }
        Assets.load(path)
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut paths = Assets.list(path)?;
        paths.extend(ExtraIcons.list(path)?);
        paths.sort();
        paths.dedup();
        Ok(paths)
    }
}

struct Video {
    id: SharedString,
    url: String,
    title: SharedString,
    channel: Option<SharedString>,
    duration: Option<u32>,
    thumbnail: Option<SharedString>,
}

impl From<SearchHit> for Video {
    fn from(hit: SearchHit) -> Self {
        Self {
            id: hit.id.into(),
            url: hit.url,
            title: hit.title.into(),
            channel: hit.channel.map(SharedString::from),
            duration: hit.duration,
            thumbnail: hit.thumbnail.map(SharedString::from),
        }
    }
}

enum Results {
    NotSearched,
    Found(Vec<Video>),
    Failed,
}

struct VideoList {
    results: Results,
    executor: Executor,
    tokio: tokio::runtime::Handle,
    tools: Entity<tools::Tools>,
    downloading: HashSet<(SharedString, Media)>,
    _subscriptions: Vec<Subscription>,
}

impl VideoList {
    fn new(
        executor: Executor,
        tokio: tokio::runtime::Handle,
        tools: Entity<tools::Tools>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        // Download buttons wait for the tools to be installed.
        let subscription = cx.observe(&tools, |_, _, cx| cx.notify());
        Self {
            results: Results::NotSearched,
            executor,
            tokio,
            tools,
            downloading: HashSet::new(),
            _subscriptions: vec![subscription],
        }
    }

    fn show(&mut self, results: Results, cx: &mut Context<Self>) {
        self.results = results;
        cx.notify();
    }

    fn download(
        &mut self,
        video: (SharedString, String, SharedString),
        media: Media,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (id, url, title) = video;
        if !self.downloading.insert((id.clone(), media)) {
            return;
        }
        let executor = self.executor.clone();
        let task = self
            .tokio
            .spawn_blocking(move || executor.download(media, url));

        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            this.update_in(cx, |this, window, cx| {
                this.downloading.remove(&(id, media));
                match result {
                    Ok(Ok(path)) => {
                        let file = path.file_name().unwrap_or_default().to_string_lossy();
                        window.push_notification(Notification::success(format!("Saved {file}")), cx);
                    }
                    Ok(Err(err)) => {
                        eprintln!("Download failed: {err}");
                        window.push_notification(
                            Notification::error(format!("Couldn't download “{title}”")),
                            cx,
                        );
                    }
                    Err(err) => {
                        eprintln!("Download task failed: {err}");
                        window.push_notification(
                            Notification::error(format!("Couldn't download “{title}”")),
                            cx,
                        );
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    fn download_button(
        &self,
        video: &Video,
        media: Media,
        ready: bool,
        cx: &mut Context<Self>,
    ) -> Button {
        let (name, label) = match media {
            Media::Audio => ("audio", "Audio"),
            Media::Video => ("video", "Video"),
        };
        let busy = self.downloading.contains(&(video.id.clone(), media));
        let target = (video.id.clone(), video.url.clone(), video.title.clone());

        Button::new(SharedString::from(format!("{name}-{}", video.id)))
            .small()
            .outline()
            .icon(Icon::new(Lucide::Download))
            .label(label)
            .loading(busy)
            .disabled(busy || !ready)
            .on_click(cx.listener(move |this, _, window, cx| {
                this.download(target.clone(), media, window, cx);
            }))
    }

    fn render_video(&self, video: &Video, ready: bool, cx: &mut Context<Self>) -> AnyElement {
        let details = video
            .channel
            .iter()
            .map(|channel| channel.to_string())
            .chain(video.duration.map(format_duration))
            .collect::<Vec<_>>()
            .join(" · ");
        let radius = cx.theme().radius;

        h_flex()
            .gap_3()
            .child(
                div()
                    .w(px(120.))
                    .h(px(68.))
                    .flex_none()
                    .rounded(radius)
                    .bg(cx.theme().muted)
                    .when_some(video.thumbnail.clone(), |frame, url| {
                        frame.child(
                            img(url)
                                .size_full()
                                .rounded(radius)
                                .object_fit(ObjectFit::Cover),
                        )
                    }),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_0p5()
                    .child(div().text_sm().truncate().child(video.title.clone()))
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .truncate()
                            .child(details),
                    ),
            )
            .child(
                h_flex()
                    .flex_none()
                    .gap_2()
                    .child(self.download_button(video, Media::Audio, ready, cx))
                    .child(self.download_button(video, Media::Video, ready, cx)),
            )
            .into_any_element()
    }
}

impl Render for VideoList {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let muted = cx.theme().muted_foreground;
        let message = |text: &'static str| div().text_sm().text_color(muted).child(text);
        let tools = self.tools.read(cx);
        let ready = tools.ready();
        // Until yt-dlp, deno and ffmpeg are installed, say why the download buttons are off.
        let setup = (!ready).then(|| {
            if tools.checking() {
                "Getting downloads ready…"
            } else {
                "Couldn't set up downloads. Check your connection, then try again in Settings."
            }
        });

        div()
            .id("video-list")
            .flex_1()
            .min_h_0()
            .overflow_y_scrollbar()
            .child(
                v_flex()
                    .px_3()
                    .pb_3()
                    .gap_3()
                    .when_some(setup, |list, setup| list.child(message(setup)))
                    .map(|list| match &self.results {
                        Results::NotSearched => list,
                        Results::Found(videos) if videos.is_empty() => {
                            list.child(message("No videos found."))
                        }
                        Results::Found(videos) => list.children(
                            videos
                                .iter()
                                .map(|video| self.render_video(video, ready, cx))
                                .collect::<Vec<_>>(),
                        ),
                        Results::Failed => list.child(message(
                            "Couldn't search YouTube. Check your connection and try again.",
                        )),
                    }),
            )
    }
}

fn format_duration(seconds: u32) -> String {
    let (hours, minutes, seconds) = (seconds / 3600, seconds / 60 % 60, seconds % 60);
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes}:{seconds:02}")
    }
}

struct Settings {
    connection: api::Connection,
    tools: Entity<tools::Tools>,
    _subscriptions: Vec<Subscription>,
}

impl Settings {
    fn new(
        connection: api::Connection,
        tools: Entity<tools::Tools>,
        cx: &mut Context<Self>,
    ) -> Self {
        let subscription = cx.observe(&tools, |_, _, cx| cx.notify());
        Self {
            connection,
            tools,
            _subscriptions: vec![subscription],
        }
    }

    fn tool_row(tools: &tools::Tools, tool: Tool, cx: &App) -> impl IntoElement {
        let version = tools.version(tool);
        let status = match (tools.status(tool), version) {
            (tools::Status::Checking, None) => "Installing…",
            (tools::Status::Checking, Some(_)) => "Checking…",
            (tools::Status::UpToDate, _) => "Up to date",
            (tools::Status::Updated, _) => "Just updated",
            (tools::Status::Failed, None) => "Couldn't install",
            (tools::Status::Failed, Some(_)) => "Couldn't check for updates",
        };

        h_flex()
            .justify_between()
            .gap_3()
            .text_sm()
            .child(
                h_flex()
                    .gap_2()
                    .child(tool.name())
                    .when_some(version, |row, version| {
                        row.child(
                            div()
                                .text_color(cx.theme().muted_foreground)
                                .child(version.to_string()),
                        )
                    }),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(status),
            )
    }

    fn field(id: &'static str, label: &'static str, value: String, cx: &App) -> impl IntoElement {
        v_flex()
            .gap_1()
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(label),
            )
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_sm()
                            .font_family(cx.theme().mono_font_family.clone())
                            .truncate()
                            .child(value.clone()),
                    )
                    .child(Clipboard::new(id).value(value)),
            )
    }
}

impl Render for Settings {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tools = self.tools.read(cx);
        let checking = tools.checking();
        let rows = Tool::ALL
            .map(|tool| Self::tool_row(tools, tool, cx).into_any_element());
        let entity = self.tools.clone();
        let heading = |text: &'static str| div().text_sm().font_semibold().child(text);

        v_flex()
            .gap_5()
            .child(
                v_flex()
                    .gap_3()
                    .child(heading("Connect other apps"))
                    .map(|section| match &self.connection.address {
                        Ok(address) => {
                            section.child(Self::field("address", "Address", address.clone(), cx))
                        }
                        Err(problem) => section.child(
                            div()
                                .text_sm()
                                .text_color(cx.theme().danger)
                                .child(problem.clone()),
                        ),
                    })
                    .child(Self::field(
                        "api-key",
                        "API key",
                        self.connection.key.clone(),
                        cx,
                    )),
            )
            .child(
                v_flex()
                    .gap_3()
                    .child(
                        h_flex()
                            .justify_between()
                            .gap_3()
                            .child(heading("Download tools"))
                            .child(
                                Button::new("check-tools")
                                    .small()
                                    .outline()
                                    .label("Check for updates")
                                    .loading(checking)
                                    .disabled(checking)
                                    .on_click(move |_, _, cx| {
                                        entity.update(cx, |tools, cx| tools.check(cx));
                                    }),
                            ),
                    )
                    .child(v_flex().gap_2().children(rows)),
            )
    }
}

struct SearchBox {
    input: Entity<InputState>,
    results: Entity<VideoList>,
    settings: Entity<Settings>,
    youtube: RustyPipe,
    tokio: tokio::runtime::Handle,
    searching: bool,
    search_task: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl SearchBox {
    fn new(
        tokio: tokio::runtime::Handle,
        youtube: RustyPipe,
        executor: Executor,
        connection: api::Connection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Search..."));
        let tools = cx.new(|cx| tools::Tools::new(tokio.clone(), cx));
        let results =
            cx.new(|cx| VideoList::new(executor, tokio.clone(), tools.clone(), window, cx));
        let settings = cx.new(|cx| Settings::new(connection, tools, cx));

        let subscription = cx.subscribe(&input, |this, input, event: &InputEvent, cx| {
            if let InputEvent::PressEnter { .. } = event {
                let term = input.read(cx).value().to_string();
                this.search(&term, cx);
            }
        });

        Self {
            input,
            results,
            settings,
            youtube,
            tokio,
            searching: false,
            search_task: None,
            _subscriptions: vec![subscription],
        }
    }

    fn is_url(&self, cx: &Context<Self>) -> bool {
        let re = Regex::new(r"http(.+)").expect("Regex parsing failure on search bar");
        let current = self.input.read(cx).value().to_string().clone();
        let Some(captures) = re.captures(&current) else {
            return false;
        };
        captures.len() > 0
    }

    fn search(&mut self, term: &str, cx: &mut Context<Self>) {
        if term.trim().is_empty() {
            return;
        }
        println!("Searching for {term}");

        if self.is_url(cx) {
            println!("Searching for a url");
            return;
        }

        // rustypipe needs Tokio, so the request runs there and GPUI awaits its result.
        let youtube = self.youtube.clone();
        let query = term.to_string();
        let request = self
            .tokio
            .spawn(async move { youtube::search(&youtube, &query).await });

        // Replacing the task drops the previous one, so an older search can't overwrite newer results.
        self.searching = true;
        self.search_task = Some(cx.spawn(async move |this, cx| {
            let results = match request.await {
                Ok(Ok(hits)) => Results::Found(hits.into_iter().map(Video::from).collect()),
                Ok(Err(err)) => {
                    eprintln!("YouTube search failed: {err}");
                    Results::Failed
                }
                Err(err) => {
                    eprintln!("YouTube search task failed: {err}");
                    Results::Failed
                }
            };

            this.update(cx, |this, cx| {
                this.searching = false;
                this.results.update(cx, |list, cx| list.show(results, cx));
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    fn open_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let settings = self.settings.clone();
        window.open_dialog(cx, move |dialog, _, _| {
            dialog
                .title("Settings")
                .w(px(460.))
                .child(settings.clone())
        });
    }
}

impl Render for SearchBox {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .size_full()
            .child(
                h_flex()
                    .p_3()
                    .gap_2()
                    .child(
                        div().flex_1().child(
                            Input::new(&self.input)
                                .prefix(Icon::new(IconName::Search))
                                .when(self.searching, |input| {
                                    input.suffix(Spinner::new().small())
                                }),
                        ),
                    )
                    .child(
                        Button::new("settings")
                            .ghost()
                            .icon(Icon::new(Lucide::Settings))
                            .tooltip("Settings")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.open_settings(window, cx);
                            })),
                    ),
            )
            .child(self.results.clone())
    }
}

fn main() {
    let tokio = tokio::runtime::Runtime::new().expect("failed to start Tokio");
    let handle = tokio.handle().clone();

    let cache = dirs::cache_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("Rippy");
    let _ = std::fs::create_dir_all(&cache);
    let youtube = RustyPipe::builder()
        .storage_dir(&cache)
        .build()
        .expect("failed to create YouTube client");
    let executor = Executor::from_env();
    let connection = api::start(&handle, youtube.clone(), executor.clone());

    application().with_assets(AppAssets).run(move |cx| {
        init(cx);
        match http::CachedHttpClient::new(&cache.join("http"), handle.clone()) {
            Ok(client) => cx.set_http_client(Arc::new(client)),
            Err(err) => eprintln!("Thumbnails won't load: {err:#}"),
        }
        let options = WindowOptions {
            titlebar: Some(TitlebarOptions {
                title: Some("Rippy".into()),
                ..Default::default()
            }),
            ..Default::default()
        };
        open_window(options, cx, |window, cx| {
            cx.new(|cx| SearchBox::new(handle, youtube, executor, connection, window, cx))
        })
        .expect("failed to open window");
    });
}
