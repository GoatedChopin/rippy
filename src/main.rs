// Release builds on Windows open without a console window behind the app.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::collections::HashSet;
use std::path::PathBuf;
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
mod feed;
mod http;
mod songsterr;
mod tools;
mod update;
mod youtube;
mod yourip;

use songsterr::Songsterr;
use tools::Tool;
use youtube::SearchHit;
use yourip::{Executor, Folder, Media};

// The default assets only carry the icons gpui-kit's own components use.
gpui_kit::assets::icon_assets!(ExtraIcons, [Download, Guitar]);

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

/// A Songsterr link pasted into the search box, and how saving its tab went.
struct TabCard {
    song: u64,
    state: TabState,
}

enum TabState {
    Working,
    Saved {
        title: SharedString,
        details: SharedString,
        path: PathBuf,
    },
    Failed,
}

struct VideoList {
    results: Results,
    tabs: Vec<TabCard>,
    executor: Executor,
    songsterr: Songsterr,
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
            tabs: Vec::new(),
            executor,
            songsterr: Songsterr::new(),
            tokio,
            tools,
            downloading: HashSet::new(),
            _subscriptions: vec![subscription],
        }
    }

    fn show(&mut self, results: Results, cx: &mut Context<Self>) {
        self.results = results;
        // A new search clears finished tabs; ones still on their way stay.
        self.tabs.retain(|card| matches!(card.state, TabState::Working));
        cx.notify();
    }

    /// Whether quitting now would leave a download or a tab half done.
    fn busy(&self) -> bool {
        !self.downloading.is_empty()
            || self.tabs.iter().any(|card| matches!(card.state, TabState::Working))
    }

    /// Saves a Songsterr song as a Guitar Pro file in the tabs folder. Doesn't need yt-dlp and
    /// the other tools, so it works while they install.
    fn get_tab(&mut self, song: u64, cx: &mut Context<Self>) {
        if self
            .tabs
            .iter()
            .any(|card| card.song == song && matches!(card.state, TabState::Working))
        {
            return;
        }
        self.tabs.retain(|card| card.song != song);
        self.tabs.insert(
            0,
            TabCard {
                song,
                state: TabState::Working,
            },
        );

        let songsterr = self.songsterr.clone();
        let folder = self.executor.folder(Folder::Tabs);
        let task = self.tokio.spawn(async move {
            let tab = songsterr.tab(song, None).await?;
            let path = songsterr::save(&tab, &folder).await?;
            anyhow::Ok((tab, path))
        });
        cx.spawn(async move |this, cx| {
            let state = match task.await {
                Ok(Ok((tab, path))) => {
                    let text = |key: &str| tab.report[key].as_str().unwrap_or_default().to_string();
                    let tracks = tab.report["tracks"].as_array().map_or(0, Vec::len);
                    let tracks = if tracks == 1 {
                        "1 track".to_string()
                    } else {
                        format!("{tracks} tracks")
                    };
                    let details = [text("artist"), tracks, "Guitar Pro".to_string()]
                        .into_iter()
                        .filter(|part| !part.is_empty())
                        .collect::<Vec<_>>()
                        .join(" · ");
                    TabState::Saved {
                        title: text("title").into(),
                        details: details.into(),
                        path,
                    }
                }
                Ok(Err(err)) => {
                    eprintln!("Songsterr tab {song} failed: {err:#}");
                    TabState::Failed
                }
                Err(err) => {
                    eprintln!("Songsterr tab task failed: {err}");
                    TabState::Failed
                }
            };
            this.update(cx, |this, cx| {
                if let Some(card) = this.tabs.iter_mut().find(|card| card.song == song) {
                    card.state = state;
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    fn render_tab(&self, card: &TabCard, cx: &mut Context<Self>) -> AnyElement {
        let radius = cx.theme().radius;
        let muted = cx.theme().muted_foreground;
        let (title, details): (SharedString, Option<SharedString>) = match &card.state {
            TabState::Working => ("Getting the tab from Songsterr…".into(), None),
            TabState::Saved { title, details, .. } => (title.clone(), Some(details.clone())),
            TabState::Failed => (
                "Couldn't get that tab from Songsterr".into(),
                Some("Check the link and your connection, then try again.".into()),
            ),
        };

        h_flex()
            .gap_3()
            .child(
                div()
                    .w(px(120.))
                    .h(px(68.))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(radius)
                    .bg(cx.theme().muted)
                    .child(Icon::new(Lucide::Guitar).text_color(muted)),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_0p5()
                    .child(div().text_sm().truncate().child(title))
                    .when_some(details, |column, details| {
                        column.child(div().text_xs().text_color(muted).truncate().child(details))
                    }),
            )
            .map(|row| match &card.state {
                TabState::Working => row.child(div().flex_none().child(Spinner::new().small())),
                TabState::Saved { path, .. } => {
                    let path = path.clone();
                    row.child(
                        Button::new(SharedString::from(format!("show-tab-{}", card.song)))
                            .small()
                            .outline()
                            .icon(Icon::new(Lucide::FolderOpen))
                            .label("Show in folder")
                            .on_click(move |_, _, cx| cx.reveal_path(&path)),
                    )
                }
                TabState::Failed => row,
            })
            .into_any_element()
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
                    .children(
                        self.tabs
                            .iter()
                            .map(|card| self.render_tab(card, cx))
                            .collect::<Vec<_>>(),
                    )
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
    updater: Entity<update::Updater>,
    executor: Executor,
    _subscriptions: Vec<Subscription>,
}

impl Settings {
    fn new(
        connection: api::Connection,
        tools: Entity<tools::Tools>,
        updater: Entity<update::Updater>,
        executor: Executor,
        cx: &mut Context<Self>,
    ) -> Self {
        let subscriptions = vec![
            cx.observe(&tools, |_, _, cx| cx.notify()),
            cx.observe(&updater, |_, _, cx| cx.notify()),
        ];
        Self {
            connection,
            tools,
            updater,
            executor,
            _subscriptions: subscriptions,
        }
    }

    fn choose_folder(&mut self, kind: Folder, window: &mut Window, cx: &mut Context<Self>) {
        let picked = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Choose".into()),
        });
        let executor = self.executor.clone();
        cx.spawn_in(window, async move |this, cx| {
            let folder = match picked.await {
                Ok(Ok(paths)) => paths.and_then(|paths| paths.into_iter().next()),
                // Linux opens the picker through the desktop portal, which some setups don't run.
                result => {
                    if let Ok(Err(err)) = result {
                        eprintln!("Couldn't open the folder picker: {err:#}");
                    }
                    this.update_in(cx, |_, window, cx| {
                        window.push_notification(
                            Notification::error("Couldn't open a folder picker on this computer."),
                            cx,
                        );
                    })
                    .ok();
                    return;
                }
            };
            let Some(folder) = folder else {
                return;
            };
            let saved = executor.set_folder(kind, folder);
            this.update_in(cx, |_, window, cx| {
                if let Err(err) = saved {
                    eprintln!("Couldn't save the download folder: {err}");
                    window.push_notification(
                        Notification::error("Couldn't save that folder. Try another one."),
                        cx,
                    );
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn folder_row(&self, kind: Folder, cx: &mut Context<Self>) -> AnyElement {
        let (id, label) = match kind {
            Folder::Audio => ("change-audio-folder", "Audio"),
            Folder::Video => ("change-video-folder", "Video"),
            Folder::Tabs => ("change-tabs-folder", "Tabs from Songsterr"),
        };
        let folder = self.executor.folder(kind).to_string_lossy().to_string();

        h_flex()
            .justify_between()
            .gap_3()
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_0p5()
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(label),
                    )
                    .child(div().text_sm().truncate().child(folder)),
            )
            .child(
                Button::new(id)
                    .small()
                    .outline()
                    .label("Change…")
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.choose_folder(kind, window, cx);
                    })),
            )
            .into_any_element()
    }

    fn rippy_row(&self, cx: &mut Context<Self>) -> AnyElement {
        let status = match self.updater.read(cx).status() {
            update::Status::Checking => "Checking…".into(),
            update::Status::UpToDate => "Up to date".into(),
            update::Status::Downloading(version) => format!("Getting {version}…").into(),
            update::Status::Ready(version) => format!("{version} starts when Rippy restarts").into(),
            update::Status::Failed(problem) | update::Status::Unavailable(problem) => {
                SharedString::from(*problem)
            }
        };

        h_flex()
            .justify_between()
            .gap_3()
            .text_sm()
            .child(
                h_flex().gap_2().child("Rippy").child(
                    div()
                        .text_color(cx.theme().muted_foreground)
                        .child(update::VERSION),
                ),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .truncate()
                    .child(status),
            )
            .into_any_element()
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
        let rippy = self.rippy_row(cx);
        let tools = self.tools.read(cx);
        let checking = tools.checking() || self.updater.read(cx).checking();
        let rows = Tool::ALL
            .map(|tool| Self::tool_row(tools, tool, cx).into_any_element());
        let (tools, updater) = (self.tools.clone(), self.updater.clone());
        let heading = |text: &'static str| div().text_sm().font_semibold().child(text);
        let audio_folder = self.folder_row(Folder::Audio, cx);
        let video_folder = self.folder_row(Folder::Video, cx);
        let tabs_folder = self.folder_row(Folder::Tabs, cx);

        v_flex()
            .gap_5()
            .child(
                v_flex()
                    .gap_3()
                    .child(heading("Save downloads to"))
                    .child(audio_folder)
                    .child(video_folder)
                    .child(tabs_folder),
            )
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
                            .child(heading("Updates"))
                            .child(
                                Button::new("check-for-updates")
                                    .small()
                                    .outline()
                                    .label("Check for updates")
                                    .loading(checking)
                                    .disabled(checking)
                                    .on_click(move |_, _, cx| {
                                        updater.update(cx, |updater, cx| updater.check(cx));
                                        tools.update(cx, |tools, cx| tools.check(cx));
                                    }),
                            ),
                    )
                    .child(v_flex().gap_2().child(rippy).children(rows)),
            )
    }
}

struct SearchBox {
    input: Entity<InputState>,
    results: Entity<VideoList>,
    settings: Entity<Settings>,
    updater: Entity<update::Updater>,
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
        let updater = cx.new(|cx| update::Updater::new(tokio.clone(), cx));
        let results = cx.new(|cx| {
            VideoList::new(executor.clone(), tokio.clone(), tools.clone(), window, cx)
        });
        let settings =
            cx.new(|cx| Settings::new(connection, tools, updater.clone(), executor, cx));

        let subscriptions = vec![
            cx.subscribe(&input, |this, input, event: &InputEvent, cx| {
                if let InputEvent::PressEnter { .. } = event {
                    let term = input.read(cx).value().to_string();
                    this.search(&term, cx);
                }
            }),
            // The toolbar offers a restart once an update is in place.
            cx.observe(&updater, |_, _, cx| cx.notify()),
        ];

        Self {
            input,
            results,
            settings,
            updater,
            youtube,
            tokio,
            searching: false,
            search_task: None,
            _subscriptions: subscriptions,
        }
    }

    fn restart(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // Quitting now would leave a download half done.
        if self.results.read(cx).busy() {
            window.push_notification(
                Notification::info("Restart once your downloads finish."),
                cx,
            );
            return;
        }
        match self.updater.read(cx).relaunch() {
            Ok(()) => cx.quit(),
            Err(err) => {
                eprintln!("Couldn't restart Rippy: {err}");
                window.push_notification(
                    Notification::error("Couldn't restart Rippy. Close it and open it again."),
                    cx,
                );
            }
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

        // A Songsterr link saves the song's tab instead of searching.
        if let Some(song) = songsterr::link_song_id(term) {
            self.search_task = None;
            self.searching = false;
            self.results.update(cx, |list, cx| list.get_tab(song, cx));
            cx.notify();
            return;
        }

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
                    .when_some(self.updater.read(cx).ready().cloned(), |toolbar, version| {
                        toolbar.child(
                            Button::new("restart-to-update")
                                .primary()
                                .label("Restart to update")
                                .tooltip(format!("Rippy {version} is ready"))
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.restart(window, cx);
                                })),
                        )
                    })
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
    // GPUI reports problems through `log`; errors show by default, RUST_LOG=info shows more.
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("error")).init();
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
            // Linux matches the window to rippy.desktop by this.
            app_id: Some("rippy".into()),
            ..Default::default()
        };
        open_window(options, cx, |window, cx| {
            cx.new(|cx| SearchBox::new(handle, youtube, executor, connection, window, cx))
        })
        .expect("failed to open window");
    });
}
