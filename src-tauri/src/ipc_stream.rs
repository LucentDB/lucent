//! Mirror of the worker's `IpcStream` for the app side: UnixStream on Unix,
//! named-pipe client on Windows. Keeps the client's framing code independent
//! of the platform concrete stream type.

use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

#[cfg(unix)]
pub enum ClientStream {
    Unix(tokio::net::UnixStream),
}

#[cfg(windows)]
pub enum ClientStream {
    Pipe(tokio::net::windows::named_pipe::NamedPipeClient),
}

impl AsyncRead for ClientStream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        match self.get_mut() {
            #[cfg(unix)]
            ClientStream::Unix(s) => Pin::new(s).poll_read(cx, buf),
            #[cfg(windows)]
            ClientStream::Pipe(p) => Pin::new(p).poll_read(cx, buf),
        }
    }
}

impl AsyncWrite for ClientStream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        match self.get_mut() {
            #[cfg(unix)]
            ClientStream::Unix(s) => Pin::new(s).poll_write(cx, buf),
            #[cfg(windows)]
            ClientStream::Pipe(p) => Pin::new(p).poll_write(cx, buf),
        }
    }
    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            #[cfg(unix)]
            ClientStream::Unix(s) => Pin::new(s).poll_flush(cx),
            #[cfg(windows)]
            ClientStream::Pipe(p) => Pin::new(p).poll_flush(cx),
        }
    }
    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            #[cfg(unix)]
            ClientStream::Unix(s) => Pin::new(s).poll_shutdown(cx),
            #[cfg(windows)]
            ClientStream::Pipe(p) => Pin::new(p).poll_shutdown(cx),
        }
    }
}
