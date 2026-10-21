//! A loopback HTTP/1.1 server for the CLI tests that need a publisher: it serves whatever bytes a
//! test hands it under whatever paths, and records what was asked for. Tests reach it over
//! `http://127.0.0.1:<port>`, never the internet.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;

/// A served file: its media type and its bytes.
type Route = (String, Vec<u8>);

#[derive(Clone)]
pub struct Server {
    port: u16,
    routes: Arc<Mutex<HashMap<String, Route>>>,
    requests: Arc<Mutex<Vec<String>>>,
}

impl Server {
    pub fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let server = Self {
            port: listener.local_addr().unwrap().port(),
            routes: Arc::default(),
            requests: Arc::default(),
        };
        let handler = server.clone();
        thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let handler = handler.clone();
                thread::spawn(move || handler.answer(stream));
            }
        });
        server
    }

    pub fn url(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}{path}", self.port)
    }

    /// Serves `body` as `content_type` at `path` until told otherwise.
    pub fn serve(&self, path: &str, content_type: &str, body: impl Into<Vec<u8>>) {
        self.routes
            .lock()
            .unwrap()
            .insert(path.to_owned(), (content_type.to_owned(), body.into()));
    }

    pub fn remove(&self, path: &str) {
        self.routes.lock().unwrap().remove(path);
    }

    /// Every path requested so far, in order.
    pub fn requests(&self) -> Vec<String> {
        self.requests.lock().unwrap().clone()
    }

    fn answer(&self, mut stream: TcpStream) {
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut request_line = String::new();
        if reader.read_line(&mut request_line).is_err() {
            return;
        }
        let path = request_line.split_whitespace().nth(1).unwrap_or("/").to_owned();
        loop {
            let mut line = String::new();
            if reader.read_line(&mut line).is_err() || line == "\r\n" || line.is_empty() {
                break;
            }
        }
        self.requests.lock().unwrap().push(path.clone());
        let route = self.routes.lock().unwrap().get(&path).cloned();
        let (status, content_type, body) = match route {
            Some((content_type, body)) => (200, content_type, body),
            None => (404, "text/plain".to_owned(), b"not found".to_vec()),
        };
        let head = format!(
            "HTTP/1.1 {status} Fixture\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        drop(stream.write_all(head.as_bytes()));
        drop(stream.write_all(&body));
    }
}
