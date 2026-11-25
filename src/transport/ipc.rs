//! Local IPC: a Unix domain socket on Linux/macOS, a named pipe on Windows
//! (both through `interprocess`' blocking local sockets, no async runtime).
//!
//! Access control is layered:
//! * Unix: the socket file lives in the 0700 home directory.
//! * Windows: named pipes are reachable by other local accounts, so every
//!   request must also carry the token stored in the per-user home directory.
//!
//! The server is one thread blocked in `accept` (zero CPU while idle) plus a
//! short-lived thread per connection. They only parse and authenticate;
//! requests are forwarded over a channel to the UI thread, which owns the
//! repository.

use std::{
    io::{self, BufRead, BufReader, Read, Write},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    thread,
    time::Duration,
};

use interprocess::local_socket::{ListenerOptions, Name, Stream, prelude::*};

use crate::{
    config::{Config, restrict},
    error::{Error, Result},
    protocol::{Envelope, PROTOCOL_VERSION, Request, Response},
    storage::random_hex,
};

const IO_TIMEOUT: Duration = Duration::from_secs(5);
/// Connections served at once; more are dropped immediately.
const MAX_CONNECTIONS: usize = 8;
const THREAD_STACK: usize = 256 * 1024;

/// A request waiting for the UI thread to answer it.
pub struct IpcRequest {
    pub request: Request,
    pub reply: mpsc::Sender<Response>,
}

fn socket_name(cfg: &Config) -> io::Result<Name<'static>> {
    #[cfg(windows)]
    {
        use interprocess::local_socket::{GenericNamespaced, ToNsName};
        // Stable per-home name so separate stores never share a pipe.
        let home = cfg.home.to_string_lossy().to_lowercase();
        let hash = home.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3));
        format!("herdr-artifacts-{hash:016x}").to_ns_name::<GenericNamespaced>()
    }
    #[cfg(not(windows))]
    {
        use interprocess::local_socket::{GenericFilePath, ToFsName};
        cfg.socket_path().to_fs_name::<GenericFilePath>()
    }
}

/// Read the IPC token, creating it on first use.
pub fn load_or_create_token(cfg: &Config) -> Result<String> {
    cfg.ensure_dirs()?;
    let path = cfg.token_path();
    if let Ok(token) = std::fs::read_to_string(&path) {
        let token = token.trim().to_string();
        if token.len() >= 32 {
            return Ok(token);
        }
    }
    let token = random_hex(32);
    std::fs::write(&path, &token)?;
    restrict(&path, 0o600);
    Ok(token)
}

/// Bind the socket and start the listener thread. Fails with
/// `AddrInUse` when another viewer already owns the socket.
pub fn spawn_server(cfg: &Config, tx: mpsc::Sender<IpcRequest>) -> Result<()> {
    let token = load_or_create_token(cfg)?;
    let max_bytes = cfg.limits.max_payload_bytes as u64 + 64 * 1024;
    let listener = match ListenerOptions::new().name(socket_name(cfg)?).create_sync() {
        Ok(l) => l,
        // A socket file left behind by a crashed viewer: if nobody answers
        // on it, it is stale and safe to replace.
        Err(e) if cfg!(unix) && e.kind() == io::ErrorKind::AddrInUse && connect(cfg).is_err() => {
            let _ = std::fs::remove_file(cfg.socket_path());
            ListenerOptions::new().name(socket_name(cfg)?).create_sync()?
        }
        Err(e) => return Err(e.into()),
    };
    restrict(&cfg.socket_path(), 0o600);

    let token: Arc<str> = token.into();
    let active = Arc::new(AtomicUsize::new(0));
    thread::Builder::new()
        .name("ipc-accept".into())
        .stack_size(THREAD_STACK)
        .spawn(move || {
            for conn in listener.incoming() {
                let Ok(conn) = conn else { continue };
                // Each connection gets a short-lived thread, so a client that
                // connects and then stalls cannot block the listener (named
                // pipes have no I/O timeouts). The cap bounds what a
                // misbehaving local client can make us hold.
                if active.fetch_add(1, Ordering::SeqCst) >= MAX_CONNECTIONS {
                    active.fetch_sub(1, Ordering::SeqCst);
                    continue; // dropping `conn` closes it
                }
                let (token, tx, done) = (token.clone(), tx.clone(), active.clone());
                let spawned = thread::Builder::new().name("ipc-conn".into()).stack_size(THREAD_STACK).spawn(move || {
                    let _ = serve_connection(conn, &token, max_bytes, &tx);
                    done.fetch_sub(1, Ordering::SeqCst);
                });
                if spawned.is_err() {
                    active.fetch_sub(1, Ordering::SeqCst);
                }
            }
        })
        .map_err(Error::from)?;
    Ok(())
}

fn serve_connection(conn: Stream, token: &str, max_bytes: u64, tx: &mpsc::Sender<IpcRequest>) -> io::Result<()> {
    // Best effort: supported on Unix sockets, not on Windows named pipes.
    let _ = conn.set_recv_timeout(Some(IO_TIMEOUT));
    let _ = conn.set_send_timeout(Some(IO_TIMEOUT));
    let mut reader = BufReader::new(conn);
    let mut line = String::new();
    reader.by_ref().take(max_bytes).read_line(&mut line)?;

    let response = match parse_request(&line, token) {
        Ok(request) => {
            let (reply_tx, reply_rx) = mpsc::channel();
            if tx.send(IpcRequest { request, reply: reply_tx }).is_err() {
                return Ok(()); // UI thread is gone; we are shutting down.
            }
            reply_rx
                .recv_timeout(IO_TIMEOUT)
                .unwrap_or_else(|_| Error::Io("viewer did not answer in time".into()).into())
        }
        Err(e) => e.into(),
    };
    let mut out = serde_json::to_vec(&response).map_err(io::Error::other)?;
    out.push(b'\n');
    reader.get_mut().write_all(&out)?;
    reader.get_mut().flush()
}

/// Authenticate and decode one request line.
pub fn parse_request(line: &str, token: &str) -> Result<Request> {
    if !line.ends_with('\n') {
        return Err(Error::TooLarge("request exceeds the payload limit or is not newline-terminated".into()));
    }
    let envelope: Envelope = serde_json::from_str(line)?;
    if !constant_time_eq(envelope.token.as_bytes(), token.as_bytes()) {
        return Err(Error::Unauthorized);
    }
    if envelope.v != PROTOCOL_VERSION {
        return Err(Error::Invalid(format!("unsupported protocol version {} (expected {PROTOCOL_VERSION})", envelope.v)));
    }
    Ok(envelope.request)
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn connect(cfg: &Config) -> io::Result<Stream> {
    Stream::connect(socket_name(cfg)?)
}

/// Send one request to a running viewer. `Ok(None)` means no viewer is
/// listening (the caller may then write to the store directly).
pub fn send(cfg: &Config, request: Request) -> Result<Option<Response>> {
    let conn = match connect(cfg) {
        Ok(c) => c,
        Err(_) => return Ok(None),
    };
    let token = std::fs::read_to_string(cfg.token_path())
        .map_err(|e| Error::Io(format!("a viewer is running but its token is unreadable: {e}")))?;
    // Best effort (see `serve_connection`); the viewer always answers within
    // `IO_TIMEOUT` once it has read the request.
    let _ = conn.set_recv_timeout(Some(IO_TIMEOUT * 2));
    let _ = conn.set_send_timeout(Some(IO_TIMEOUT));

    let envelope = Envelope { v: PROTOCOL_VERSION, token: token.trim().to_string(), request };
    let mut out = serde_json::to_vec(&envelope)?;
    out.push(b'\n');
    let mut reader = BufReader::new(conn);
    reader.get_mut().write_all(&out)?;
    reader.get_mut().flush()?;

    let mut line = String::new();
    reader.read_line(&mut line)?;
    if line.is_empty() {
        return Err(Error::Io("viewer closed the connection without answering".into()));
    }
    Ok(Some(serde_json::from_str(&line)?))
}

/// Whether a viewer currently answers on the socket.
pub fn is_running(cfg: &Config) -> bool {
    matches!(send(cfg, Request::Ping), Ok(Some(Response::Ok { .. })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authentication() {
        let ok = "{\"v\":1,\"token\":\"secret\",\"op\":\"ping\"}\n";
        assert_eq!(parse_request(ok, "secret").unwrap(), Request::Ping);
        assert_eq!(parse_request(ok, "other").unwrap_err(), Error::Unauthorized);
        assert_eq!(parse_request(ok, "secre").unwrap_err(), Error::Unauthorized);
        assert_eq!(parse_request("{\"v\":1,\"op\":\"ping\"}\n", "secret").unwrap_err().code(), "invalid");
        assert_eq!(parse_request("{\"v\":9,\"token\":\"secret\",\"op\":\"ping\"}\n", "secret").unwrap_err().code(), "invalid");
        assert_eq!(parse_request("{\"v\":1,\"token\":\"secret\",\"op\":\"ping\"}", "secret").unwrap_err().code(), "too_large");
        assert_eq!(parse_request("not json\n", "secret").unwrap_err().code(), "invalid");
    }
}
