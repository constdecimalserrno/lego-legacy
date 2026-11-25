//! Unix domain socket HTTP/1.1 transport (`--control-unix <path>`) (#309).
//!
//! Shares the exact same [`super::http::handle_connection`] HTTP/1.1 router as
//! the TCP server over `std::os::unix::net::UnixListener` (zero crates).

use super::http::{handle_connection, HttpContext};
use std::io;
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

/// Running Unix domain socket HTTP/1.1 control server.
pub struct UnixServer {
    path: PathBuf,
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl UnixServer {
    /// Bind a Unix domain socket at `path` and spawn its accept loop.
    ///
    /// Removes any pre-existing socket file at `path` before binding and
    /// removes the socket cleanly on [`Drop`].
    ///
    /// # Errors
    /// Returns any filesystem or socket bind error.
    pub fn start(path: PathBuf, ctx: Arc<HttpContext>) -> io::Result<Self> {
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent)?;
        }
        if path.exists() {
            let _ = std::fs::remove_file(&path);
        }
        let listener = UnixListener::bind(&path)?;
        listener.set_nonblocking(true)?;
        let stop = Arc::new(AtomicBool::new(false));
        let stop_flag = Arc::clone(&stop);
        let handle = thread::Builder::new()
            .name("control-unix".to_owned())
            .spawn(move || {
                while !stop_flag.load(Ordering::Relaxed) {
                    match listener.accept() {
                        Ok((stream, _)) => {
                            let _ = stream.set_nonblocking(false);
                            let _ = stream.set_read_timeout(Some(Duration::from_secs(30)));
                            let _ = stream.set_write_timeout(Some(Duration::from_secs(30)));
                            let conn_ctx = Arc::clone(&ctx);
                            let _ = thread::Builder::new()
                                .name("control-unix-req".to_owned())
                                .spawn(move || {
                                    handle_connection(stream, &conn_ctx);
                                });
                        }
                        Err(err) if err.kind() == io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(10));
                        }
                        Err(_) => {
                            if stop_flag.load(Ordering::Relaxed) {
                                break;
                            }
                            thread::sleep(Duration::from_millis(25));
                        }
                    }
                }
            })?;
        Ok(Self {
            path,
            stop,
            handle: Some(handle),
        })
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    #[must_use]
    pub fn url(&self) -> String {
        format!("unix://{}", self.path.display())
    }

    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
        let _ = std::fs::remove_file(&self.path);
    }
}

impl Drop for UnixServer {
    fn drop(&mut self) {
        self.stop();
    }
}
