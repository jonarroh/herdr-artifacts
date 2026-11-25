//! Copying text to the clipboard without a clipboard crate.
//!
//! First choice is the platform's own clipboard: the Win32 API on Windows,
//! and elsewhere a standard tool fed through stdin (never a shell): `pbcopy`
//! on macOS, `wl-copy` / `xclip` / `xsel` on Linux. If that fails we fall back to the OSC 52 escape
//! sequence, which asks the terminal itself to set the clipboard (this is
//! also what reaches the local machine over SSH). `HERDR_ARTIFACTS_CLIPBOARD`
//! can force `osc52` or `native`.

use std::{env, io};

#[cfg(not(windows))]
use std::{
    io::Write,
    process::{Command, Stdio},
};

use crossterm::{clipboard::CopyToClipboard, execute};

/// Terminals cap OSC 52 payloads; beyond this we refuse rather than truncate.
const OSC52_MAX_BYTES: usize = 100_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    Native,
    Osc52,
}

pub fn copy(text: &str) -> io::Result<Method> {
    match env::var("HERDR_ARTIFACTS_CLIPBOARD").ok().as_deref() {
        Some("osc52") => osc52(text).map(|()| Method::Osc52),
        Some("native") => native(text).map(|()| Method::Native),
        _ => match native(text) {
            Ok(()) => Ok(Method::Native),
            Err(_) => osc52(text).map(|()| Method::Osc52),
        },
    }
}

fn osc52(text: &str) -> io::Result<()> {
    if text.len() > OSC52_MAX_BYTES {
        return Err(io::Error::other(format!("text is too large for OSC 52 ({} bytes)", text.len())));
    }
    execute!(io::stdout(), CopyToClipboard::to_clipboard_from(text))
}

fn native(text: &str) -> io::Result<()> {
    #[cfg(windows)]
    {
        win::set(&windows_text(text))
    }
    #[cfg(target_os = "macos")]
    {
        pipe_to("pbcopy", &[], text.as_bytes())
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let candidates: [(&str, &[&str]); 3] =
            [("wl-copy", &[]), ("xclip", &["-selection", "clipboard"]), ("xsel", &["--clipboard", "--input"])];
        let mut last = io::Error::new(io::ErrorKind::NotFound, "no clipboard tool found");
        for (program, args) in candidates {
            match pipe_to(program, args, text.as_bytes()) {
                Ok(()) => return Ok(()),
                Err(e) => last = e,
            }
        }
        Err(last)
    }
}

/// `CF_UNICODETEXT` payload: UTF-16, CRLF line endings (the Windows clipboard
/// convention) and a terminating NUL.
pub fn windows_text(text: &str) -> Vec<u16> {
    let mut out = Vec::with_capacity(text.len() + 1);
    let mut previous = '\0';
    for c in text.chars().filter(|c| *c != '\0') {
        if c == '\n' && previous != '\r' {
            out.push(u16::from(b'\r'));
        }
        let mut units = [0u16; 2];
        out.extend_from_slice(c.encode_utf16(&mut units));
        previous = c;
    }
    out.push(0);
    out
}

/// The Win32 clipboard, called directly: no helper process, and unlike
/// `clip.exe` it neither depends on the console code page nor leaves a
/// byte-order mark in the copied text.
#[cfg(windows)]
mod win {
    use std::{ffi::c_void, io, ptr, thread, time::Duration};

    #[link(name = "user32")]
    unsafe extern "system" {
        fn OpenClipboard(owner: *mut c_void) -> i32;
        fn CloseClipboard() -> i32;
        fn EmptyClipboard() -> i32;
        fn SetClipboardData(format: u32, memory: *mut c_void) -> *mut c_void;
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GlobalAlloc(flags: u32, bytes: usize) -> *mut c_void;
        fn GlobalLock(memory: *mut c_void) -> *mut c_void;
        fn GlobalUnlock(memory: *mut c_void) -> i32;
        fn GlobalFree(memory: *mut c_void) -> *mut c_void;
    }

    const CF_UNICODETEXT: u32 = 13;
    const GMEM_MOVEABLE: u32 = 0x0002;

    /// `wide` must be NUL-terminated UTF-16.
    pub fn set(wide: &[u16]) -> io::Result<()> {
        // Another program may hold the clipboard for a moment; retry briefly.
        let mut opened = false;
        for _ in 0..10 {
            // SAFETY: a null owner is allowed; the call has no other preconditions.
            if unsafe { OpenClipboard(ptr::null_mut()) } != 0 {
                opened = true;
                break;
            }
            thread::sleep(Duration::from_millis(15));
        }
        if !opened {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: the clipboard is open and is closed again below on every path.
        let result = unsafe { write(wide) };
        // SAFETY: paired with the successful OpenClipboard above.
        unsafe { CloseClipboard() };
        result
    }

    /// SAFETY: the caller must have opened the clipboard.
    unsafe fn write(wide: &[u16]) -> io::Result<()> {
        unsafe {
            if EmptyClipboard() == 0 {
                return Err(io::Error::last_os_error());
            }
            let memory = GlobalAlloc(GMEM_MOVEABLE, std::mem::size_of_val(wide));
            if memory.is_null() {
                return Err(io::Error::last_os_error());
            }
            let target = GlobalLock(memory).cast::<u16>();
            if target.is_null() {
                let error = io::Error::last_os_error();
                GlobalFree(memory);
                return Err(error);
            }
            // The block was allocated with exactly `wide`'s size in bytes.
            ptr::copy_nonoverlapping(wide.as_ptr(), target, wide.len());
            GlobalUnlock(memory);
            // On success the system owns the block; on failure we still do.
            if SetClipboardData(CF_UNICODETEXT, memory).is_null() {
                let error = io::Error::last_os_error();
                GlobalFree(memory);
                return Err(error);
            }
            Ok(())
        }
    }
}

#[cfg(not(windows))]
fn pipe_to(program: &str, args: &[&str], input: &[u8]) -> io::Result<()> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    // Dropping stdin after the write closes the pipe so the tool can finish.
    child.stdin.take().ok_or_else(|| io::Error::other("no stdin"))?.write_all(input)?;
    let status = child.wait()?;
    if status.success() { Ok(()) } else { Err(io::Error::other(format!("{program} exited with {status}"))) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_payload_is_nul_terminated_utf16_with_crlf() {
        let units = |s: &str| s.encode_utf16().collect::<Vec<u16>>();
        assert_eq!(windows_text("añ"), [units("añ"), vec![0]].concat());
        assert_eq!(windows_text("a\nb\r\nc"), [units("a\r\nb\r\nc"), vec![0]].concat(), "LF becomes CRLF exactly once");
        // Characters outside the BMP become surrogate pairs; NULs cannot cut the text short.
        assert_eq!(windows_text("😀").len(), 2 + 1);
        assert_eq!(windows_text("a\0b"), [units("ab"), vec![0]].concat());
    }
}
