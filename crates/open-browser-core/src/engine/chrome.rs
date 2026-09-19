//! Finding Chrome and launching it.

use std::path::{Path, PathBuf};
use std::time::Duration;

use chromiumoxide::browser::{Browser, BrowserConfig};
use chromiumoxide::Page;
use futures_util::StreamExt;

use crate::error::{cdp, Error, Result};

/// Where to look when `$OPEN_BROWSER_CHROME` is unset. Order matters: a Chromium build is a better
/// automation target than the browser someone has their real life logged into, but Chrome is what
/// is actually installed, so it wins on availability.
#[cfg(target_os = "macos")]
const CANDIDATES: &[&str] = &[
    "/Applications/Chromium.app/Contents/MacOS/Chromium",
    "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
    "/Applications/Google Chrome Canary.app/Contents/MacOS/Google Chrome Canary",
    "/Applications/Brave Browser.app/Contents/MacOS/Brave Browser",
    "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
];

#[cfg(target_os = "linux")]
const CANDIDATES: &[&str] = &[
    "/usr/bin/chromium",
    "/usr/bin/chromium-browser",
    "/usr/bin/google-chrome",
    "/usr/bin/google-chrome-stable",
    "/usr/bin/brave-browser",
    "/usr/bin/microsoft-edge",
    "/snap/bin/chromium",
];

#[cfg(target_os = "windows")]
const CANDIDATES: &[&str] = &[
    r"C:\Program Files\Google\Chrome\Application\chrome.exe",
    r"C:\Program Files (x86)\Google\Chrome\Application\chrome.exe",
    r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe",
];

pub const INSTALL_HINT: &str =
    "https://www.google.com/chrome/ , or `brew install --cask chromium` / `apt install chromium`";

/// The browser binary this machine will use, or `None`.
pub fn locate_chrome() -> Option<PathBuf> {
    if let Some(raw) = std::env::var_os(crate::CHROME_ENV) {
        let path = PathBuf::from(raw);
        if path.is_file() {
            return Some(path);
        }
    }
    CANDIDATES.iter().map(PathBuf::from).find(|path| path.is_file())
}

/// Everything that varies between one launched browser and the next.
#[derive(Debug, Clone)]
pub struct LaunchOptions {
    /// Chrome's `--user-data-dir`. Persisting it is what makes a login survive a session restart.
    pub profile: PathBuf,
    pub headless: bool,
    pub window: (u32, u32),
    /// Extra Chrome flags, straight through.
    pub args: Vec<String>,
    pub launch_timeout: Duration,
}

impl LaunchOptions {
    pub fn new(profile: impl AsRef<Path>) -> Self {
        Self {
            profile: profile.as_ref().to_path_buf(),
            headless: true,
            window: (1280, 800),
            args: Vec::new(),
            launch_timeout: Duration::from_secs(30),
        }
    }
}

/// Start Chrome so that it outlives this process, and return its pid and CDP endpoint.
///
/// chromiumoxide's own `Browser::launch` makes the browser a child with `kill_on_drop`, which is
/// right for a one-shot but is exactly wrong for a session: the whole point is that `ob goto` can
/// exit and `ob click` can attach to the same logged-in browser a minute later. So the process is
/// started here directly and then connected to over the wire.
pub async fn start_detached(options: &LaunchOptions) -> Result<(u32, String)> {
    let executable = locate_chrome().ok_or(Error::NoBrowser { hint: INSTALL_HINT })?;
    std::fs::create_dir_all(&options.profile).map_err(|source| Error::Io {
        context: format!("creating the profile directory {}", options.profile.display()),
        source,
    })?;
    // Chrome writes the port it chose into this file. Removing a stale one first means the poll
    // below cannot read the port from a previous run of this same profile and connect to nothing.
    let port_file = options.profile.join("DevToolsActivePort");
    let _ = std::fs::remove_file(&port_file);

    let mut command = std::process::Command::new(&executable);
    command
        // Port 0: Chrome picks a free one and reports it, so two sessions never collide and no
        // port needs to be configured.
        .arg("--remote-debugging-port=0")
        .arg(format!("--user-data-dir={}", options.profile.display()))
        .arg(format!("--window-size={},{}", options.window.0, options.window.1))
        .arg("--no-first-run")
        .arg("--no-default-browser-check")
        .arg("--disable-background-networking")
        .arg("--disable-backgrounding-occluded-windows")
        .arg("--disable-renderer-backgrounding")
        .arg("--homepage=about:blank")
        .arg("about:blank")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    if options.headless {
        command.arg("--headless=new");
    }
    for arg in &options.args {
        command.arg(arg);
    }
    #[cfg(unix)]
    {
        // Its own process group, or the Ctrl-C that stops the terminal `ob session start` ran in
        // would take the session's browser down with it.
        use std::os::unix::process::CommandExt as _;
        command.process_group(0);
    }

    let child = command.spawn().map_err(|source| Error::Io {
        context: format!("launching {}", executable.display()),
        source,
    })?;
    let pid = child.id();
    // The child is deliberately not awaited or killed on drop; it is the session.
    std::mem::forget(child);

    let deadline = std::time::Instant::now() + options.launch_timeout;
    loop {
        if let Some(port) = read_port(&port_file) {
            return Ok((pid, format!("http://127.0.0.1:{port}")));
        }
        if std::time::Instant::now() >= deadline {
            // Leaving a browser running that nothing can reach would be worse than the failure.
            crate::session::terminate(pid);
            return Err(Error::Timeout {
                what: format!("{} to report its debugging port", executable.display()),
                seconds: options.launch_timeout.as_secs(),
            });
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}

/// The first line of `DevToolsActivePort` is the port; the second is the browser's WS path.
///
/// Chrome writes the file in one go, but a reader can still see it between `create` and `write`, so
/// an empty or unparseable read means "not yet" rather than "broken".
fn read_port(path: &Path) -> Option<u16> {
    let text = std::fs::read_to_string(path).ok()?;
    text.lines().next()?.trim().parse().ok()
}

/// How long to keep asking the handler for tabs before concluding there really are none.
///
/// This covers the gap between connecting and the handler's `Target.setDiscoverTargets` round
/// trip completing. It is a few milliseconds in practice, and the loop exits as soon as a tab
/// appears, so the only thing this bounds is the genuinely-empty case.
const ATTACH_TIMEOUT: Duration = Duration::from_secs(5);

/// A launched browser, with its event pump running.
///
/// chromiumoxide splits a browser into a handle and a stream that must be polled or nothing ever
/// resolves; forgetting to drive it is the classic way to make every call hang forever. Keeping the
/// pump's task with the handle means it cannot be forgotten, and dropping the handle stops it.
pub struct BrowserHandle {
    browser: Browser,
    pump: tokio::task::JoinHandle<()>,
}

impl BrowserHandle {
    pub async fn launch(options: &LaunchOptions) -> Result<Self> {
        let executable = locate_chrome().ok_or(Error::NoBrowser { hint: INSTALL_HINT })?;
        std::fs::create_dir_all(&options.profile).map_err(|source| Error::Io {
            context: format!("creating the profile directory {}", options.profile.display()),
            source,
        })?;

        let mut builder = BrowserConfig::builder()
            .chrome_executable(&executable)
            .user_data_dir(&options.profile)
            .window_size(options.window.0, options.window.1)
            .launch_timeout(options.launch_timeout);
        if !options.headless {
            builder = builder.with_head();
        }
        for arg in &options.args {
            builder = builder.arg(arg.clone());
        }
        let config = builder.build().map_err(Error::Other)?;

        let (browser, mut handler) =
            cdp(Browser::launch(config).await, || format!("launching {}", executable.display()))?;
        let pump = tokio::spawn(async move {
            // Errors here are the connection going away, which the next command reports with far
            // better context than a log line from a detached task could.
            while let Some(event) = handler.next().await {
                let _ = event;
            }
        });
        Ok(Self { browser, pump })
    }

    pub fn browser(&self) -> &Browser {
        &self.browser
    }

    /// The page actions run against: the session's own tab.
    ///
    /// Two things make this harder than "take the first page". A freshly connected handler has not
    /// finished discovering targets, so an immediate `pages()` comes back empty and a naive caller
    /// opens a second tab; and the handler stores targets in a `HashMap`, so once there is more
    /// than one, "first" is whichever the hash order gives. Between them, consecutive `ob`
    /// commands would each end up in a different tab — `ob goto` navigates one and `ob text` reads
    /// another, blank one.
    ///
    /// So a session records the target id of the tab it opened with, and every later command
    /// resolves exactly that.
    pub async fn page(&self, target: Option<&str>) -> Result<Page> {
        let deadline = std::time::Instant::now() + ATTACH_TIMEOUT;
        loop {
            let pages = cdp(self.browser.pages().await, || "listing open tabs".to_string())?;
            match target {
                Some(id) => {
                    if let Some(page) = pages.iter().find(|page| page.target_id().as_ref() == id) {
                        return Ok(page.clone());
                    }
                    // Other tabs exist but not that one, so it is gone rather than undiscovered —
                    // someone closed it. Waiting out the timeout would not bring it back.
                    if !pages.is_empty() {
                        break;
                    }
                }
                None => {
                    if let Some(page) = pages.into_iter().next() {
                        return Ok(page);
                    }
                }
            }
            if std::time::Instant::now() >= deadline {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
        cdp(self.browser.new_page("about:blank").await, || "opening a tab".to_string())
    }

    /// The id of the tab a freshly started session should pin itself to.
    ///
    /// Called once, at `session start`, and stored in the registry; every command after that
    /// passes it back to [`Self::page`].
    pub async fn primary_target(&self) -> Result<String> {
        Ok(self.page(None).await?.target_id().as_ref().to_string())
    }

    /// Attach to a browser that is already running, such as one a session started.
    pub async fn connect(endpoint: &str) -> Result<Self> {
        let (browser, mut handler) = cdp(Browser::connect(endpoint.to_string()).await, || {
            format!("connecting to {endpoint}")
        })?;
        let pump = tokio::spawn(async move {
            while let Some(event) = handler.next().await {
                let _ = event;
            }
        });
        Ok(Self { browser, pump })
    }

    pub async fn close(mut self) -> Result<()> {
        let _ = self.browser.close().await;
        let _ = self.browser.wait().await;
        self.pump.abort();
        Ok(())
    }
}

impl Drop for BrowserHandle {
    fn drop(&mut self) {
        self.pump.abort();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_half_written_port_file_reads_as_not_ready_rather_than_as_a_port() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("DevToolsActivePort");
        assert_eq!(read_port(&path), None, "a missing file is not ready");
        std::fs::write(&path, "").unwrap();
        assert_eq!(read_port(&path), None, "an empty file is not ready");
        std::fs::write(&path, "51234\n/devtools/browser/abc\n").unwrap();
        assert_eq!(read_port(&path), Some(51234));
    }

    #[test]
    fn an_explicit_chrome_path_is_honoured_only_when_it_exists() {
        // Both assertions are about the env var alone, so they hold on a machine with no browser
        // and on one with several.
        let missing = std::env::temp_dir().join("definitely-not-a-browser");
        std::env::set_var(crate::CHROME_ENV, &missing);
        assert_ne!(locate_chrome(), Some(missing));

        let real = std::env::current_exe().unwrap();
        std::env::set_var(crate::CHROME_ENV, &real);
        assert_eq!(locate_chrome(), Some(real));
        std::env::remove_var(crate::CHROME_ENV);
    }
}
