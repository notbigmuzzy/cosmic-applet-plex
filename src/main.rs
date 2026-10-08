use cosmic::{
    iced::{Subscription, window::Id},
    prelude::*,
    surface, widget,
};
use cosmic_applet_plex::{Error, Plex, ScanReport, Session, read_token, write_token};
use std::{
    collections::HashMap,
    rc::Rc,
    time::{Duration, Instant},
};

#[derive(Clone)]
struct PlayingSession {
    info: Session,
    poster: Option<widget::image::Handle>,
}

fn main() -> cosmic::iced::Result {
    cosmic::applet::run::<Applet>(())
}

struct Applet {
    core: cosmic::Core,
    popup: Option<Id>,
    plex: Plex,
    status: Option<Result<(), Error>>,
    checking: bool,
    scanning: bool,
    scan_success_until: Option<Instant>,
    sessions: Option<Result<Vec<PlayingSession>, Error>>,
    sessions_loading: bool,
    sessions_generation: u64,
    notice: Option<String>,
    settings_open: bool,
    token_input: String,
}

#[derive(Clone)]
enum Message {
    TogglePopup,
    PopupClosed(Id),
    Check,
    Status(Result<(), Error>),
    Rescan,
    Scanned(Result<ScanReport, Error>),
    ClearScanSuccess,
    RefreshSessions,
    Sessions(u64, Result<Vec<PlayingSession>, Error>),
    OpenPlex,
    Opened(bool),
    ToggleSettings,
    TokenChanged(String),
    SaveToken,
}

impl std::fmt::Debug for Message {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Message")
            .field("kind", &std::mem::discriminant(self))
            .finish()
    }
}

impl Applet {
    fn check(&mut self) -> Task<cosmic::Action<Message>> {
        if self.checking {
            return Task::none();
        }
        self.checking = true;
        let plex = self.plex.clone();
        Task::perform(async move { plex.status().await }, |result| {
            cosmic::Action::App(Message::Status(result))
        })
    }

    fn online(&self) -> bool {
        matches!(self.status, Some(Ok(())))
    }

    fn refresh_sessions(&mut self) -> Task<cosmic::Action<Message>> {
        if self.popup.is_none() || self.sessions_loading || !self.online() {
            return Task::none();
        }
        let token = match read_token() {
            Ok(token) => token,
            Err(error) => {
                self.sessions = Some(Err(error));
                return Task::none();
            }
        };
        let cached: HashMap<_, _> = self
            .sessions
            .as_ref()
            .and_then(|result| result.as_ref().ok())
            .into_iter()
            .flatten()
            .filter_map(|session| Some((session.info.poster.clone()?, session.poster.clone()?)))
            .collect();
        self.sessions_loading = true;
        let generation = self.sessions_generation;
        let plex = self.plex.clone();
        Task::perform(
            async move {
                let sessions = plex.sessions(&token).await?;
                let sessions =
                    cosmic::iced::futures::future::join_all(sessions.into_iter().map(|info| {
                        let plex = &plex;
                        let token = &token;
                        let cached = &cached;
                        async move {
                            let poster = match &info.poster {
                                Some(path) => match cached.get(path) {
                                    Some(handle) => Some(handle.clone()),
                                    None => plex
                                        .poster(path, token)
                                        .await
                                        .ok()
                                        .map(widget::image::Handle::from_bytes),
                                },
                                None => None,
                            };
                            PlayingSession { info, poster }
                        }
                    }))
                    .await;
                Ok(sessions)
            },
            move |result| cosmic::Action::App(Message::Sessions(generation, result)),
        )
    }

    fn playback_view(&self) -> Option<Element<'_, Message>> {
        Some(match &self.sessions {
            None => return None,
            Some(Err(Error::MissingToken)) => {
                widget::text("Add a Plex token to view playback.").into()
            }
            Some(Err(error)) => widget::text(error_text(error)).into(),
            Some(Ok(sessions)) if sessions.is_empty() => return None,
            Some(Ok(sessions)) => {
                let mut list = widget::column([]).spacing(16);
                for session in sessions {
                    let poster: Element<'_, Message> = match &session.poster {
                        Some(handle) => widget::image(handle.clone())
                            .width(64)
                            .height(96)
                            .content_fit(cosmic::iced::ContentFit::Contain)
                            .into(),
                        None => widget::container(
                            widget::icon::from_name("video-x-generic-symbolic").size(24),
                        )
                        .center_x(cosmic::iced::Length::Fixed(64.0))
                        .center_y(cosmic::iced::Length::Fixed(96.0))
                        .into(),
                    };
                    let duration = session
                        .info
                        .duration_ms
                        .map(format_time)
                        .unwrap_or_else(|| "--:--".into());
                    let position = session
                        .info
                        .duration_ms
                        .map_or(session.info.position_ms, |duration| {
                            session.info.position_ms.min(duration)
                        });
                    let details = widget::column([])
                        .push(widget::text::heading(&session.info.title))
                        .push(widget::text::caption(&session.info.device))
                        .push(widget::text(&session.info.state))
                        .push(widget::text::caption(format!(
                            "{} / {duration}",
                            format_time(position)
                        )))
                        .spacing(6)
                        .width(cosmic::iced::Length::Fill);
                    list = list.push(widget::row([]).push(poster).push(details).spacing(12));
                }
                widget::container(widget::scrollable(list).height(cosmic::iced::Length::Shrink))
                    .max_height(360)
                    .into()
            }
        })
    }
}

impl cosmic::Application for Applet {
    type Executor = cosmic::executor::Default;
    type Flags = ();
    type Message = Message;
    const APP_ID: &'static str = "io.github.cosmic-applet-plex";

    fn core(&self) -> &cosmic::Core {
        &self.core
    }
    fn core_mut(&mut self) -> &mut cosmic::Core {
        &mut self.core
    }

    fn init(core: cosmic::Core, _: ()) -> (Self, Task<cosmic::Action<Message>>) {
        let mut app = Self {
            core,
            popup: None,
            plex: Plex::local(),
            status: None,
            checking: false,
            scanning: false,
            scan_success_until: None,
            sessions: None,
            sessions_loading: false,
            sessions_generation: 0,
            notice: None,
            settings_open: false,
            token_input: String::new(),
        };
        let task = app.check();
        (app, task)
    }

    fn on_close_requested(&self, id: Id) -> Option<Message> {
        Some(Message::PopupClosed(id))
    }

    fn subscription(&self) -> Subscription<Message> {
        let status = cosmic::iced::time::every(Duration::from_secs(10)).map(|_| Message::Check);
        if self.popup.is_some() {
            Subscription::batch([
                status,
                cosmic::iced::time::every(Duration::from_secs(5)).map(|_| Message::RefreshSessions),
            ])
        } else {
            status
        }
    }

    fn view(&self) -> Element<'_, Message> {
        self.core
            .applet
            .icon_button_from_handle(
                widget::icon::from_svg_bytes(
                    include_bytes!("../data/plex-symbolic.svg").as_slice(),
                )
                .into(),
            )
            .on_press(Message::TogglePopup)
            .into()
    }

    fn view_window(&self, _: Id) -> Element<'_, Message> {
        let (icon, status) = match &self.status {
            None => ("content-loading-symbolic", "Checking connection..."),
            Some(Ok(())) => ("emblem-ok-symbolic", "Plex server is online"),
            Some(Err(Error::Offline)) => {
                ("network-offline-symbolic", "No connection to Plex server")
            }
            Some(Err(Error::Unauthorized)) => (
                "system-lock-screen-symbolic",
                "Server requires authorization",
            ),
            Some(Err(_)) => (
                "dialog-warning-symbolic",
                "Invalid response from Plex server",
            ),
        };
        let scan_icon = widget::icon(widget::icon::from_name("view-refresh-symbolic").into())
            .size(16)
            .class(cosmic::theme::Svg::Custom(Rc::new({
                let success = scan_feedback_active(self.scan_success_until, Instant::now());
                move |_| widget::svg::Style {
                    color: success.then_some(cosmic::iced::Color::from_rgb8(46, 184, 92)),
                }
            })));
        let scan_button = widget::button::custom(scan_icon)
            .class(cosmic::theme::Button::Icon)
            .width(40)
            .height(40)
            .padding(12)
            .on_press_maybe((self.online() && !self.scanning).then_some(Message::Rescan));
        let actions = widget::row([])
            .push(widget::tooltip(
                scan_button,
                widget::text(if self.scanning {
                    "Sending scan requests..."
                } else {
                    "Rescan folders"
                }),
                widget::tooltip::Position::Top,
            ))
            .push(
                widget::button::icon(widget::icon::from_name("web-browser-symbolic"))
                    .width(40)
                    .height(40)
                    .padding(12)
                    .tooltip("Open Plex")
                    .on_press(Message::OpenPlex),
            )
            .push(
                widget::button::icon(widget::icon::from_name("preferences-system-symbolic"))
                    .width(40)
                    .height(40)
                    .padding(12)
                    .tooltip("Plex token")
                    .on_press(Message::ToggleSettings),
            )
            .spacing(16);
        let mut content = widget::column([])
            .push(
                widget::row([])
                    .push(widget::text::title3("Plex").width(cosmic::iced::Length::Fill))
                    .push(widget::tooltip(
                        widget::icon::from_name(icon).size(20),
                        widget::text(status),
                        widget::tooltip::Position::Bottom,
                    ))
                    .align_y(cosmic::iced::Alignment::Center)
                    .width(cosmic::iced::Length::Fill),
            )
            .push(widget::divider::horizontal::default())
            .push(widget::container(actions).center_x(cosmic::iced::Length::Fill))
            .spacing(12)
            .padding(16);
        if let Some(playback) = self.playback_view() {
            content = content
                .push(widget::divider::horizontal::default())
                .push(playback);
        }
        if let Some(notice) = &self.notice {
            content = content.push(widget::text(notice.as_str()));
        }
        if self.settings_open {
            content = content
                .push(widget::divider::horizontal::default())
                .push(
                    widget::text_input::secure_input("X-Plex-Token", &self.token_input, None, true)
                        .on_input(Message::TokenChanged),
                )
                .push(
                    widget::button::standard("Save token")
                        .leading_icon(widget::icon::from_name("document-save-symbolic"))
                        .on_press_maybe(
                            (!self.token_input.trim().is_empty() && !self.scanning)
                                .then_some(Message::SaveToken),
                        ),
                );
        }
        self.core.applet.popup_container(content).into()
    }

    fn update(&mut self, message: Message) -> Task<cosmic::Action<Message>> {
        match message {
            Message::TogglePopup => {
                self.sessions_generation += 1;
                self.sessions_loading = false;
                self.sessions = None;
                if let Some(id) = self.popup.take() {
                    self.token_input.clear();
                    self.settings_open = false;
                    return Task::done(cosmic::Action::Surface(surface::Action::DestroyPopup(id)));
                }
                let id = Id::unique();
                self.popup = Some(id);
                let popup = surface::action::app_popup::<Self>(
                    |_| surface::action::LiveSettings::default(),
                    move |app| {
                        app.core.applet.get_popup_settings(
                            app.core.main_window_id().unwrap(),
                            id,
                            None,
                            None,
                            None,
                        )
                    },
                    None,
                );
                return Task::batch([
                    Task::done(cosmic::Action::Surface(popup)),
                    self.check(),
                    self.refresh_sessions(),
                ]);
            }
            Message::PopupClosed(id) => {
                if self.popup == Some(id) {
                    self.popup = None;
                    self.token_input.clear();
                    self.settings_open = false;
                    self.sessions_generation += 1;
                    self.sessions_loading = false;
                    self.sessions = None;
                }
            }
            Message::Check => return self.check(),
            Message::Status(result) => {
                self.checking = false;
                if let Err(error) = &result {
                    self.sessions_generation += 1;
                    self.sessions_loading = false;
                    self.sessions = Some(Err(error.clone()));
                }
                self.status = Some(result);
                return self.refresh_sessions();
            }
            Message::RefreshSessions => return self.refresh_sessions(),
            Message::Sessions(generation, result) => {
                if generation == self.sessions_generation && self.popup.is_some() {
                    self.sessions_loading = false;
                    self.sessions = Some(result);
                }
            }
            Message::Rescan if self.online() && !self.scanning => {
                let token = match read_token() {
                    Ok(token) => token,
                    Err(error) => {
                        self.notice = Some(error_text(&error).into());
                        self.settings_open = true;
                        return Task::none();
                    }
                };
                self.scanning = true;
                self.scan_success_until = None;
                self.notice = None;
                let plex = self.plex.clone();
                return Task::perform(async move { plex.rescan(&token).await }, |result| {
                    cosmic::Action::App(Message::Scanned(result))
                });
            }
            Message::Rescan => {}
            Message::Scanned(result) => {
                self.scanning = false;
                if matches!(&result, Ok(report) if report.accepted > 0 && report.failed == 0) {
                    self.notice = None;
                    self.scan_success_until = Some(Instant::now() + Duration::from_secs(5));
                    return Task::batch([
                        self.check(),
                        Task::perform(
                            async {
                                tokio::time::sleep(Duration::from_secs(5)).await;
                            },
                            |_| cosmic::Action::App(Message::ClearScanSuccess),
                        ),
                    ]);
                }
                self.notice = Some(match result {
                    Ok(report) if report.accepted == 0 && report.failed == 0 => {
                        "No libraries to scan.".into()
                    }
                    Ok(report) => {
                        format!("Accepted: {}. Failed: {}.", report.accepted, report.failed)
                    }
                    Err(error) => error_text(&error).into(),
                });
                return self.check();
            }
            Message::ClearScanSuccess => {
                if !scan_feedback_active(self.scan_success_until, Instant::now()) {
                    self.scan_success_until = None;
                }
            }
            Message::OpenPlex => {
                return Task::perform(
                    async {
                        tokio::task::spawn_blocking(|| {
                            std::process::Command::new("xdg-open")
                                .arg("http://127.0.0.1:32400/web")
                                .status()
                                .is_ok_and(|status| status.success())
                        })
                        .await
                        .unwrap_or(false)
                    },
                    |success| cosmic::Action::App(Message::Opened(success)),
                );
            }
            Message::Opened(false) => self.notice = Some("Could not open the browser.".into()),
            Message::Opened(true) => {}
            Message::ToggleSettings => {
                self.settings_open = !self.settings_open;
                self.token_input.clear();
            }
            Message::TokenChanged(token) => self.token_input = token,
            Message::SaveToken => {
                self.notice = Some(match write_token(&self.token_input) {
                    Ok(()) => {
                        self.token_input.clear();
                        self.settings_open = false;
                        "Token saved.".into()
                    }
                    Err(error) => error_text(&error).into(),
                });
                self.sessions_generation += 1;
                self.sessions_loading = false;
                self.sessions = None;
                return self.refresh_sessions();
            }
        }
        Task::none()
    }

    fn style(&self) -> Option<cosmic::iced::theme::Style> {
        Some(cosmic::applet::style())
    }
}

fn format_time(milliseconds: u64) -> String {
    let seconds = milliseconds / 1000;
    if seconds >= 3600 {
        format!(
            "{}:{:02}:{:02}",
            seconds / 3600,
            (seconds / 60) % 60,
            seconds % 60
        )
    } else {
        format!("{}:{:02}", seconds / 60, seconds % 60)
    }
}

fn scan_feedback_active(until: Option<Instant>, now: Instant) -> bool {
    until.is_some_and(|until| now < until)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_playback_times() {
        assert_eq!(format_time(0), "0:00");
        assert_eq!(format_time(90000), "1:30");
        assert_eq!(format_time(3661000), "1:01:01");
    }

    #[test]
    fn scan_feedback_expires_after_five_seconds() {
        let now = Instant::now();
        let until = Some(now + Duration::from_secs(5));
        assert!(scan_feedback_active(until, now));
        assert!(scan_feedback_active(
            until,
            now + Duration::from_millis(4999)
        ));
        assert!(!scan_feedback_active(until, now + Duration::from_secs(5)));
        assert!(!scan_feedback_active(None, now));
    }
}

fn error_text(error: &Error) -> &'static str {
    match error {
        Error::Offline => "No connection to Plex server.",
        Error::Unauthorized => "Plex token was rejected or does not have permission to scan.",
        Error::MissingToken => "Enter a Plex token to scan libraries.",
        Error::TokenFile => "Invalid token or token file unavailable.",
        Error::InvalidResponse => "Invalid response from Plex server.",
        Error::Server => "Plex server rejected the request.",
    }
}
