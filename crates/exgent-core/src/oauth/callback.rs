use std::{
    io::{BufRead, BufReader, Write},
    net::{TcpListener, TcpStream},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver},
        Arc,
    },
    thread,
    time::Duration,
};

use reqwest::Url;

use super::AuthorizationCode;

pub(super) struct CallbackServer {
    pub(super) receiver: Receiver<Result<AuthorizationCode, String>>,
    pub(super) cancel: Arc<AtomicBool>,
}

pub(super) fn start_callback_server(
    host: &str,
    port: u16,
    expected_state: &str,
    callback_path: &str,
    service_name: &str,
) -> Result<CallbackServer, String> {
    let (sender, receiver) = mpsc::channel();
    let cancel = Arc::new(AtomicBool::new(false));
    let mut bind_errors = Vec::new();
    let mut listener_count = 0usize;

    for bind_host in callback_bind_hosts(host) {
        match TcpListener::bind((bind_host.as_str(), port)) {
            Ok(listener) => {
                listener.set_nonblocking(true).map_err(|error| {
                    format!("failed to configure callback server on {bind_host}:{port}: {error}")
                })?;
                listener_count += 1;
                spawn_callback_listener(
                    listener,
                    expected_state.to_string(),
                    callback_path.to_string(),
                    service_name.to_string(),
                    Arc::clone(&cancel),
                    sender.clone(),
                );
            }
            Err(error) => {
                bind_errors.push(format!("{bind_host}:{port}: {error}"));
            }
        }
    }

    if listener_count == 0 {
        return Err(format!(
            "failed to listen for OAuth callback: {}",
            bind_errors.join("; ")
        ));
    }

    Ok(CallbackServer { receiver, cancel })
}

pub(super) fn callback_bind_hosts(host: &str) -> Vec<String> {
    match host {
        "127.0.0.1" | "localhost" => vec!["127.0.0.1".to_string(), "::1".to_string()],
        value => vec![value.to_string()],
    }
}

fn spawn_callback_listener(
    listener: TcpListener,
    expected_state: String,
    callback_path: String,
    service_name: String,
    cancel: Arc<AtomicBool>,
    sender: mpsc::Sender<Result<AuthorizationCode, String>>,
) {
    thread::spawn(move || {
        let deadline = std::time::Instant::now() + Duration::from_secs(10 * 60);
        loop {
            if cancel.load(Ordering::Relaxed) || std::time::Instant::now() >= deadline {
                return;
            }

            match listener.accept() {
                Ok((mut stream, _addr)) => {
                    let result = handle_callback_stream(
                        &mut stream,
                        &expected_state,
                        &callback_path,
                        &service_name,
                    );
                    let _ = sender.send(result);
                    return;
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(100));
                }
                Err(error) => {
                    let _ = sender.send(Err(format!("callback server failed: {error}")));
                    return;
                }
            }
        }
    });
}

fn handle_callback_stream(
    stream: &mut TcpStream,
    expected_state: &str,
    callback_path: &str,
    service_name: &str,
) -> Result<AuthorizationCode, String> {
    let mut reader = BufReader::new(
        stream
            .try_clone()
            .map_err(|error| format!("failed to read callback request: {error}"))?,
    );
    let mut request_line = String::new();
    reader
        .read_line(&mut request_line)
        .map_err(|error| format!("failed to read callback request: {error}"))?;
    let path = request_line
        .split_whitespace()
        .nth(1)
        .ok_or_else(|| "invalid callback request".to_string())?;
    let url = Url::parse(&format!("http://localhost{path}"))
        .map_err(|error| format!("invalid callback URL: {error}"))?;

    if url.path() != callback_path {
        write_callback_response(stream, 404, "Callback route not found.");
        return Err("callback route not found".to_string());
    }

    if let Some(error) = url
        .query_pairs()
        .find(|(key, _)| key == "error")
        .map(|(_, value)| value.to_string())
    {
        write_callback_response(
            stream,
            400,
            &format!("{service_name} authentication did not complete."),
        );
        return Err(format!("{service_name} authentication error: {error}"));
    }

    let code = url
        .query_pairs()
        .find(|(key, _)| key == "code")
        .map(|(_, value)| value.to_string())
        .ok_or_else(|| "missing authorization code".to_string())?;
    let state = url
        .query_pairs()
        .find(|(key, _)| key == "state")
        .map(|(_, value)| value.to_string())
        .ok_or_else(|| "missing OAuth state".to_string())?;

    if state != expected_state {
        write_callback_response(stream, 400, "OAuth state mismatch.");
        return Err("OAuth state mismatch".to_string());
    }

    write_callback_response(
        stream,
        200,
        &format!("{service_name} authentication completed. You can close this window."),
    );
    Ok(AuthorizationCode { code, state })
}

fn write_callback_response(stream: &mut TcpStream, status: u16, message: &str) {
    let status_text = match status {
        200 => "OK",
        400 => "Bad Request",
        404 => "Not Found",
        _ => "Internal Server Error",
    };
    let body = format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>exgent OAuth</title></head><body><h1>{message}</h1></body></html>"
    );
    let response = format!(
        "HTTP/1.1 {status} {status_text}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes());
    let _ = stream.flush();
}

pub(super) fn oauth_callback_host() -> String {
    std::env::var("EXGENT_OAUTH_CALLBACK_HOST")
        .or_else(|_| std::env::var("PI_OAUTH_CALLBACK_HOST"))
        .unwrap_or_else(|_| "127.0.0.1".to_string())
}
