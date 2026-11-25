//! Opening the native window from the command line: raise the one that is
//! already open, or start a detached process that outlives the caller.

use std::{
    env, fs,
    path::PathBuf,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use crate::{
    config::Config,
    error::{Error, Result},
    protocol::{Request, Response},
    transport::ipc,
};

pub const TERMINAL_VIEWER_OPEN: &str =
    "a terminal viewer is already open for this store; close it to use the native window";

const START_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Raised {
    /// A window was open and came to the front.
    Window,
    /// The running viewer is a terminal pane.
    Terminal,
    /// No viewer is running.
    Nothing,
}

/// Ask a running viewer to show its window, selecting artifact `id`.
pub fn raise(cfg: &Config, id: Option<String>) -> Result<Raised> {
    Ok(match ipc::send(cfg, Request::Show { id })? {
        Some(Response::Ok { .. }) => Raised::Window,
        Some(Response::Error { .. }) => Raised::Terminal,
        None => Raised::Nothing,
    })
}

fn log_path(cfg: &Config) -> PathBuf {
    cfg.home.join("window.log")
}

/// Show the native window and return once it is up. It keeps running after
/// this process exits.
pub fn open(cfg: &Config, id: Option<String>) -> Result<()> {
    match raise(cfg, id.clone())? {
        Raised::Window => return Ok(()),
        Raised::Terminal => return Err(Error::invalid(TERMINAL_VIEWER_OPEN)),
        Raised::Nothing => {}
    }

    cfg.ensure_dirs()?;
    let log = fs::File::create(log_path(cfg))?;
    let mut command = Command::new(env::current_exe()?);
    command
        .arg("window")
        .env("HERDR_ARTIFACTS_HOME", &cfg.home)
        // Do not keep the caller's directory in use while the window is open.
        .current_dir(&cfg.home)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(log);
    detach(&mut command);
    let mut child = command.spawn().map_err(|e| Error::Io(format!("could not start the window: {e}")))?;

    let started = Instant::now();
    while started.elapsed() < START_TIMEOUT {
        if let Some(status) = child.try_wait()? {
            // It may have lost the race against another window.
            if raise(cfg, id.clone())? == Raised::Window {
                return Ok(());
            }
            let details = fs::read_to_string(log_path(cfg)).unwrap_or_default();
            return Err(Error::Io(format!("the window exited ({status}): {}", details.trim())));
        }
        // Answered only once the window is processing requests. It starts on
        // the newest artifact, so ask again for the one that was published.
        if raise(cfg, id.clone()).is_ok_and(|r| r == Raised::Window) {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(100));
    }
    Err(Error::Io(format!("the window did not answer; see {}", log_path(cfg).display())))
}

/// Separate the window from the caller's console and process group, so closing
/// the pane that published does not close it.
#[cfg(windows)]
fn detach(command: &mut Command) {
    use std::os::windows::{io::AsRawHandle, process::CommandExt};
    use windows_sys::Win32::{
        Foundation::{HANDLE_FLAG_INHERIT, SetHandleInformation},
        System::Threading::{CREATE_NEW_PROCESS_GROUP, DETACHED_PROCESS},
    };

    command.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
    // Windows would otherwise hand the window our own stdout and stderr too,
    // and whoever captures them (an agent running `publish --show`) would wait
    // for the window to close.
    for handle in [std::io::stdin().as_raw_handle(), std::io::stdout().as_raw_handle(), std::io::stderr().as_raw_handle()]
    {
        // SAFETY: the handle belongs to this process; clearing a flag on an
        // invalid or console handle just fails, which is harmless.
        unsafe { SetHandleInformation(handle.cast(), HANDLE_FLAG_INHERIT, 0) };
    }
}

#[cfg(unix)]
fn detach(command: &mut Command) {
    use std::os::unix::process::CommandExt;
    command.process_group(0);
}
