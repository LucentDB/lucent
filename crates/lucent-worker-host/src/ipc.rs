//! Platform abstraction over the worker's single-client IPC listener.
//! Unix: domain socket with 0700 permissions. Windows: named pipe whose
//! DACL grants full control to the current user SID only (the tokio
//! ServerOptions API cannot express this — see win_security.rs).

use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

#[cfg(unix)]
pub enum IpcListener {
    Unix(tokio::net::UnixListener),
}

#[cfg(windows)]
pub enum IpcListener {
    Pipe(tokio::net::windows::named_pipe::NamedPipeServer),
}

#[cfg(unix)]
pub enum IpcStream {
    Unix(tokio::net::UnixStream),
}

#[cfg(windows)]
pub enum IpcStream {
    Pipe(tokio::net::windows::named_pipe::NamedPipeServer),
}

/// Bind the worker's IPC endpoint. `addr` is a socket file path on Unix and
/// a named-pipe name (`\\.\pipe\...`) on Windows. Generic over `AsRef<Path>`
/// so both `PathBuf` (tests) and `String` (main) call sites keep compiling
/// unchanged.
pub fn bind(addr: impl AsRef<std::path::Path>) -> io::Result<IpcListener> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let addr = addr.as_ref();
        let _ = std::fs::remove_file(addr);
        let listener = tokio::net::UnixListener::bind(addr)?;
        std::fs::set_permissions(addr, std::fs::Permissions::from_mode(0o700))?;
        Ok(IpcListener::Unix(listener))
    }
    #[cfg(windows)]
    {
        crate::win_security::bind_pipe(&addr.as_ref().to_string_lossy())
    }
}

impl IpcListener {
    /// Accept the worker's single client. On Windows this is `connect()`:
    /// a named-pipe instance IS the server object; there is no listener type.
    pub async fn accept(self) -> io::Result<IpcStream> {
        match self {
            #[cfg(unix)]
            IpcListener::Unix(listener) => {
                let (stream, _addr) = listener.accept().await?;
                Ok(IpcStream::Unix(stream))
            }
            #[cfg(windows)]
            IpcListener::Pipe(server) => {
                server.connect().await?;
                Ok(IpcStream::Pipe(server))
            }
        }
    }
}

impl AsyncRead for IpcStream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        match self.get_mut() {
            #[cfg(unix)]
            IpcStream::Unix(s) => Pin::new(s).poll_read(cx, buf),
            #[cfg(windows)]
            IpcStream::Pipe(p) => Pin::new(p).poll_read(cx, buf),
        }
    }
}

impl AsyncWrite for IpcStream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        match self.get_mut() {
            #[cfg(unix)]
            IpcStream::Unix(s) => Pin::new(s).poll_write(cx, buf),
            #[cfg(windows)]
            IpcStream::Pipe(p) => Pin::new(p).poll_write(cx, buf),
        }
    }
    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            #[cfg(unix)]
            IpcStream::Unix(s) => Pin::new(s).poll_flush(cx),
            #[cfg(windows)]
            IpcStream::Pipe(p) => Pin::new(p).poll_flush(cx),
        }
    }
    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            #[cfg(unix)]
            IpcStream::Unix(s) => Pin::new(s).poll_shutdown(cx),
            #[cfg(windows)]
            IpcStream::Pipe(p) => Pin::new(p).poll_shutdown(cx),
        }
    }
}
