//! The lifecycle the loopback fixture servers share: a listener that serves each connection on a
//! thread of its own until it is dropped, and a wait that a dropped server or a departed client
//! cuts short. Dropping the server stops accepting, ends every connection it still holds, and joins
//! every thread it started, so nothing it made outlives the test that made it.

use std::collections::HashMap;
use std::io::ErrorKind;
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

/// How often a waiting thread looks at whether it should stop.
const POLL: Duration = Duration::from_millis(20);

/// A loopback listener and the threads serving it; see the module docs.
pub(super) struct Listening {
    pub(super) port: u16,
    closed: Arc<AtomicBool>,
    /// A handle on every connection still being served, to shut it down when the server is dropped.
    /// Its thread takes it out when it ends, so the socket closes then, as a dropped stream does.
    connections: Arc<Mutex<HashMap<usize, TcpStream>>>,
    acceptor: Option<JoinHandle<()>>,
}

impl Listening {
    /// Listens on a free loopback port and runs `serve` for each connection on a thread of its own,
    /// handing it the flag that says the server has been dropped.
    pub(super) fn start(serve: impl Fn(TcpStream, &AtomicBool) + Send + Sync + 'static) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        // Nonblocking, so the accept loop sees the server dropped without a connection to wake it.
        listener.set_nonblocking(true).unwrap();
        let closed = Arc::new(AtomicBool::new(false));
        let connections = Arc::new(Mutex::new(HashMap::new()));
        let (ended, held, serve) = (Arc::clone(&closed), Arc::clone(&connections), Arc::new(serve));
        let acceptor = thread::spawn(move || {
            let mut workers = Vec::new();
            let mut next = 0;
            while !ended.load(Ordering::SeqCst) {
                let stream = match listener.accept() {
                    Ok((stream, _)) => stream,
                    Err(error) if error.kind() == ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(2));
                        continue;
                    }
                    Err(_) => continue,
                };
                // Some platforms hand an accepted socket the listener's nonblocking mode.
                stream.set_nonblocking(false).unwrap();
                next += 1;
                let id = next;
                held.lock().unwrap().insert(id, stream.try_clone().unwrap());
                let (serve, ended, held) = (Arc::clone(&serve), Arc::clone(&ended), Arc::clone(&held));
                workers.push(thread::spawn(move || {
                    serve(stream, &ended);
                    held.lock().unwrap().remove(&id);
                }));
            }
            for worker in workers {
                drop(worker.join());
            }
        });
        Self {
            port,
            closed,
            connections,
            acceptor: Some(acceptor),
        }
    }
}

impl Drop for Listening {
    fn drop(&mut self) {
        self.closed.store(true, Ordering::SeqCst);
        // Unblocks every read and write a connection's thread is in, which then ends.
        for (_, stream) in self.connections.lock().unwrap().drain() {
            drop(stream.shutdown(Shutdown::Both));
        }
        if let Some(acceptor) = self.acceptor.take() {
            drop(acceptor.join());
        }
    }
}

/// Waits out `pause`, or less once the client hangs up or `closed` is set: true when the whole
/// pause passed. A thread waiting for a test that has moved on lets go within 20 ms.
pub(super) fn linger(stream: &TcpStream, pause: Duration, closed: &AtomicBool) -> bool {
    let started = Instant::now();
    let mut byte = [0u8; 1];
    loop {
        let left = pause.saturating_sub(started.elapsed());
        if left.is_zero() {
            break;
        }
        if closed.load(Ordering::SeqCst) {
            return false;
        }
        let slice = left.min(POLL);
        drop(stream.set_read_timeout(Some(slice)));
        match stream.peek(&mut byte) {
            Ok(0) => return false,
            // Bytes the client sent that nobody has read hide its hanging up; `closed` still ends it.
            Ok(_) => thread::sleep(slice),
            Err(error) if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {}
            Err(_) => return false,
        }
    }
    drop(stream.set_read_timeout(None));
    true
}
