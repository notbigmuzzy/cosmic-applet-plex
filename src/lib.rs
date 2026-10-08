use std::{path::PathBuf, time::Duration};

#[derive(Clone)]
pub struct Plex {
    client: reqwest::Client,
    base: reqwest::Url,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    Offline,
    Unauthorized,
    InvalidResponse,
    Server,
    MissingToken,
    TokenFile,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScanReport {
    pub accepted: usize,
    pub failed: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Session {
    pub title: String,
    pub device: String,
    pub state: String,
    pub position_ms: u64,
    pub duration_ms: Option<u64>,
    pub poster: Option<String>,
}

impl Plex {
    pub fn local() -> Self {
        Self::new("http://127.0.0.1:32400/").expect("valid local Plex URL")
    }

    pub fn new(base: &str) -> Result<Self, Error> {
        let base = reqwest::Url::parse(base).map_err(|_| Error::InvalidResponse)?;
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| Error::Server)?;
        Ok(Self { client, base })
    }

    async fn response(&self, path: &str, token: Option<&str>) -> Result<reqwest::Response, Error> {
        let url = self.base.join(path).map_err(|_| Error::InvalidResponse)?;
        let mut request = self.client.get(url).header("Accept", "application/xml");
        if let Some(token) = token {
            let mut value =
                reqwest::header::HeaderValue::from_str(token).map_err(|_| Error::TokenFile)?;
            value.set_sensitive(true);
            request = request.header("X-Plex-Token", value);
        }
        let response = request.send().await.map_err(|_| Error::Offline)?;
        match response.status().as_u16() {
            401 | 403 => return Err(Error::Unauthorized),
            200..=299 => {}
            _ => return Err(Error::Server),
        }
        Ok(response)
    }

    async fn get(&self, path: &str, token: Option<&str>) -> Result<String, Error> {
        self.response(path, token)
            .await?
            .text()
            .await
            .map_err(|_| Error::InvalidResponse)
    }

    pub async fn sessions(&self, token: &str) -> Result<Vec<Session>, Error> {
        if token.trim().is_empty() {
            return Err(Error::MissingToken);
        }
        parse_sessions(&self.get("status/sessions", Some(token)).await?)
    }

    pub async fn poster(&self, path: &str, token: &str) -> Result<Vec<u8>, Error> {
        let source = self.base.join(path).map_err(|_| Error::InvalidResponse)?;
        if source.origin() != self.base.origin()
            || !source.path().starts_with("/library/metadata/")
            || source.query().is_some()
            || source.fragment().is_some()
        {
            return Err(Error::InvalidResponse);
        }
        let mut url = self
            .base
            .join("photo/:/transcode")
            .map_err(|_| Error::InvalidResponse)?;
        url.query_pairs_mut()
            .append_pair("width", "144")
            .append_pair("height", "216")
            .append_pair("minSize", "1")
            .append_pair("upscale", "0")
            .append_pair("url", source.path());
        let mut response = self.response(url.as_str(), Some(token)).await?;
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| Error::InvalidResponse)? {
            if bytes.len() + chunk.len() > 4 * 1024 * 1024 {
                return Err(Error::InvalidResponse);
            }
            bytes.extend_from_slice(&chunk);
        }
        if bytes.is_empty() {
            Err(Error::InvalidResponse)
        } else {
            Ok(bytes)
        }
    }

    pub async fn status(&self) -> Result<(), Error> {
        let body = self.get("identity", None).await?;
        let document = roxmltree::Document::parse(&body).map_err(|_| Error::InvalidResponse)?;
        let root = document.root_element();
        if root.tag_name().name() != "MediaContainer"
            || root.attribute("machineIdentifier").is_none()
        {
            return Err(Error::InvalidResponse);
        }
        Ok(())
    }

    pub async fn rescan(&self, token: &str) -> Result<ScanReport, Error> {
        if token.trim().is_empty() {
            return Err(Error::MissingToken);
        }
        let body = self.get("library/sections", Some(token)).await?;
        let keys = section_keys(&body)?;
        let mut report = ScanReport {
            accepted: 0,
            failed: 0,
        };
        for key in keys {
            match self
                .get(&format!("library/sections/{key}/refresh"), Some(token))
                .await
            {
                Ok(_) => report.accepted += 1,
                Err(Error::Unauthorized) => return Err(Error::Unauthorized),
                Err(_) => report.failed += 1,
            }
        }
        Ok(report)
    }
}

fn parse_sessions(body: &str) -> Result<Vec<Session>, Error> {
    let document = roxmltree::Document::parse(body).map_err(|_| Error::InvalidResponse)?;
    let root = document.root_element();
    if !root.has_tag_name("MediaContainer") {
        return Err(Error::InvalidResponse);
    }
    root.children()
        .filter(|node| node.has_tag_name("Video") || node.has_tag_name("Track"))
        .map(|node| {
            let title = node.attribute("title").ok_or(Error::InvalidResponse)?;
            let title = match node.attribute("type") {
                Some("episode") => {
                    let series = node.attribute("grandparentTitle").unwrap_or("TV episode");
                    let season = node.attribute("parentIndex").unwrap_or("?");
                    let episode = node.attribute("index").unwrap_or("?");
                    format!("{series} - S{season} E{episode} - {title}")
                }
                Some("track") => match node.attribute("grandparentTitle") {
                    Some(artist) => format!("{artist} - {title}"),
                    None => title.to_owned(),
                },
                _ => title.to_owned(),
            };
            let player = node.children().find(|child| child.has_tag_name("Player"));
            let state = match player.and_then(|player| player.attribute("state")) {
                Some("playing") => "Playing",
                Some("paused") => "Paused",
                Some("buffering") => "Buffering",
                Some("stopped") => "Stopped",
                _ => "Unknown",
            }
            .to_owned();
            let preferred_poster = match node.attribute("type") {
                Some("episode") => node.attribute("grandparentThumb"),
                Some("track") => node.attribute("parentThumb"),
                _ => None,
            };
            Ok(Session {
                title,
                device: player
                    .and_then(|player| {
                        player
                            .attribute("title")
                            .or_else(|| player.attribute("product"))
                    })
                    .unwrap_or("Unknown device")
                    .to_owned(),
                state,
                position_ms: node
                    .attribute("viewOffset")
                    .and_then(|value| value.parse().ok())
                    .unwrap_or(0),
                duration_ms: node
                    .attribute("duration")
                    .and_then(|value| value.parse().ok()),
                poster: preferred_poster
                    .or_else(|| node.attribute("thumb"))
                    .map(str::to_owned),
            })
        })
        .collect()
}

fn section_keys(body: &str) -> Result<Vec<u32>, Error> {
    let document = roxmltree::Document::parse(body).map_err(|_| Error::InvalidResponse)?;
    let root = document.root_element();
    if root.tag_name().name() != "MediaContainer" {
        return Err(Error::InvalidResponse);
    }
    root.children()
        .filter(|node| node.has_tag_name("Directory"))
        .map(|node| {
            node.attribute("key")
                .and_then(|key| key.parse().ok())
                .ok_or(Error::InvalidResponse)
        })
        .collect()
}

pub fn token_path() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".config")
        });
    base.join("cosmic-applet-plex/token")
}

pub fn read_token() -> Result<String, Error> {
    let token = std::fs::read_to_string(token_path()).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            Error::MissingToken
        } else {
            Error::TokenFile
        }
    })?;
    let token = token.trim().to_owned();
    if token.is_empty() {
        Err(Error::MissingToken)
    } else {
        Ok(token)
    }
}

pub fn write_token(token: &str) -> Result<(), Error> {
    write_token_at(&token_path(), token)
}

fn write_token_at(path: &std::path::Path, token: &str) -> Result<(), Error> {
    use std::{
        io::Write,
        os::unix::fs::{OpenOptionsExt, PermissionsExt},
    };
    let token = token.trim();
    if token.is_empty() {
        return Err(Error::MissingToken);
    }
    reqwest::header::HeaderValue::from_str(token).map_err(|_| Error::TokenFile)?;
    let parent = path.parent().ok_or(Error::TokenFile)?;
    std::fs::create_dir_all(parent).map_err(|_| Error::TokenFile)?;
    std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700))
        .map_err(|_| Error::TokenFile)?;
    let temporary = path.with_extension("new");
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&temporary)
        .map_err(|_| Error::TokenFile)?;
    let result = file
        .write_all(token.as_bytes())
        .and_then(|_| file.sync_all())
        .and_then(|_| std::fs::rename(&temporary, path));
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result.map_err(|_| Error::TokenFile)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };

    async fn server(
        replies: Vec<(&'static str, &'static str, &'static str)>,
    ) -> (Plex, tokio::task::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let plex = Plex::new(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
        let task = tokio::spawn(async move {
            for (path, status, body) in replies {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = Vec::new();
                loop {
                    let mut buffer = [0; 1024];
                    let count = socket.read(&mut buffer).await.unwrap();
                    assert!(count > 0);
                    request.extend_from_slice(&buffer[..count]);
                    if request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
                        break;
                    }
                }
                let request = String::from_utf8(request).unwrap();
                assert!(request.starts_with(&format!("GET {path} HTTP/1.1")));
                if path.starts_with("/library/")
                    || path.starts_with("/status/")
                    || path.starts_with("/photo/")
                {
                    assert!(
                        request
                            .to_lowercase()
                            .contains("x-plex-token: test-token\r\n")
                    );
                    assert!(!request.lines().next().unwrap().contains("test-token"));
                }
                let response = format!(
                    "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                socket.write_all(response.as_bytes()).await.unwrap();
            }
        });
        (plex, task)
    }

    #[tokio::test]
    async fn recognizes_plex_identity() {
        let (plex, task) = server(vec![(
            "/identity",
            "200 OK",
            "<MediaContainer machineIdentifier=\"local\"/>",
        )])
        .await;
        assert_eq!(plex.status().await, Ok(()));
        task.await.unwrap();
    }

    #[tokio::test]
    async fn rejects_non_plex_response() {
        let (plex, task) = server(vec![("/identity", "200 OK", "<html/>")]).await;
        assert_eq!(plex.status().await, Err(Error::InvalidResponse));
        task.await.unwrap();
    }

    #[tokio::test]
    async fn detects_offline_server() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let plex = Plex::new(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
        drop(listener);
        assert_eq!(plex.status().await, Err(Error::Offline));
    }

    #[tokio::test]
    async fn rescans_all_libraries_and_reports_partial_failure() {
        let (plex, task) = server(vec![
            (
                "/library/sections",
                "200 OK",
                "<MediaContainer><Directory key=\"1\"/><Directory key=\"7\"/></MediaContainer>",
            ),
            ("/library/sections/1/refresh", "200 OK", ""),
            (
                "/library/sections/7/refresh",
                "500 Internal Server Error",
                "",
            ),
        ])
        .await;
        assert_eq!(
            plex.rescan("test-token").await,
            Ok(ScanReport {
                accepted: 1,
                failed: 1
            })
        );
        task.await.unwrap();
    }

    #[tokio::test]
    async fn reports_invalid_token() {
        let (plex, task) = server(vec![("/library/sections", "401 Unauthorized", "")]).await;
        assert_eq!(plex.rescan("test-token").await, Err(Error::Unauthorized));
        task.await.unwrap();
    }

    #[tokio::test]
    async fn reads_authenticated_sessions_and_empty_state() {
        let (plex, task) = server(vec![(
            "/status/sessions",
            "200 OK",
            "<MediaContainer size=\"0\"/>",
        )])
        .await;
        assert_eq!(plex.sessions("test-token").await, Ok(vec![]));
        task.await.unwrap();
        let (plex, task) = server(vec![("/status/sessions", "403 Forbidden", "")]).await;
        assert_eq!(plex.sessions("test-token").await, Err(Error::Unauthorized));
        task.await.unwrap();
    }

    #[test]
    fn parses_movie_episode_and_music_sessions() {
        let sessions = parse_sessions(r#"<MediaContainer>
            <Video type="movie" title="Movie &amp; More" viewOffset="90000" duration="7200000" thumb="/library/metadata/1/thumb/2"><Player title="Living Room" state="playing"/></Video>
            <Video type="episode" title="Pilot" grandparentTitle="A Series" parentIndex="1" index="2" grandparentThumb="/library/metadata/3/thumb/4"><Player product="Plex Web" state="paused"/></Video>
            <Track type="track" title="Song" grandparentTitle="Artist" parentThumb="/library/metadata/5/thumb/6"><Player title="Desktop" state="buffering"/></Track>
        </MediaContainer>"#).unwrap();
        assert_eq!(sessions.len(), 3);
        assert_eq!(sessions[0].title, "Movie & More");
        assert_eq!(sessions[0].device, "Living Room");
        assert_eq!(sessions[0].state, "Playing");
        assert_eq!(sessions[0].position_ms, 90000);
        assert_eq!(sessions[0].duration_ms, Some(7200000));
        assert_eq!(sessions[1].title, "A Series - S1 E2 - Pilot");
        assert_eq!(sessions[1].device, "Plex Web");
        assert_eq!(sessions[1].state, "Paused");
        assert_eq!(sessions[2].title, "Artist - Song");
        assert_eq!(sessions[2].state, "Buffering");
        assert!(parse_sessions("<html/>").is_err());
        assert!(parse_sessions("not xml").is_err());
        assert!(parse_sessions("<MediaContainer><Video/></MediaContainer>").is_err());
    }

    #[tokio::test]
    async fn rejects_external_poster_urls_before_sending_token() {
        let plex = Plex::local();
        for path in [
            "https://example.com/library/metadata/1/thumb",
            "//example.com/library/metadata/1/thumb",
            "/identity",
            "/library/metadata/1/thumb?secret=value",
        ] {
            assert_eq!(
                plex.poster(path, "test-token").await,
                Err(Error::InvalidResponse)
            );
        }
    }

    #[tokio::test]
    async fn fetches_bounded_poster_with_authenticated_transcode_request() {
        let (plex, task) = server(vec![(
            "/photo/:/transcode?width=144&height=216&minSize=1&upscale=0&url=%2Flibrary%2Fmetadata%2F1%2Fthumb%2F2",
            "200 OK", "poster-data",
        )]).await;
        assert_eq!(
            plex.poster("/library/metadata/1/thumb/2", "test-token")
                .await,
            Ok(b"poster-data".to_vec())
        );
        task.await.unwrap();
    }

    #[test]
    fn rejects_unsafe_or_missing_section_keys() {
        assert_eq!(
            section_keys("<MediaContainer><Directory key=\"../identity\"/></MediaContainer>"),
            Err(Error::InvalidResponse)
        );
        assert_eq!(
            section_keys("<MediaContainer><Directory/></MediaContainer>"),
            Err(Error::InvalidResponse)
        );
        assert_eq!(section_keys("<MediaContainer/>"), Ok(vec![]));
    }

    #[test]
    fn saves_token_privately_and_replaces_it() {
        use std::os::unix::fs::PermissionsExt;
        let folder = std::env::temp_dir().join(format!("cosmic-plex-test-{}", std::process::id()));
        let path = folder.join("token");
        write_token_at(&path, "first-token\n").unwrap();
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        write_token_at(&path, "second-token").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "second-token");
        assert_eq!(write_token_at(&path, ""), Err(Error::MissingToken));
        std::fs::remove_dir_all(folder).unwrap();
    }
}
