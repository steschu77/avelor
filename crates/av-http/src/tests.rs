//! Requests against a one-shot HTTP server on a loopback port.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use av_utils::cancel::Cancel;
use av_utils::error::Error;

use crate::{Client, FormPart, Method};

// ----------------------------------------------------------------------------
fn client() -> Client {
    Client::new("av-http-test", Duration::from_secs(10)).unwrap()
}

// ----------------------------------------------------------------------------
/// Serve one request: `respond` writes the response for the raw request
/// (head and body), or stalls. The server's join handle yields the request.
fn serve(
    respond: impl FnOnce(&mut TcpStream, &str) + Send + 'static,
) -> (String, JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let request = read_request(&mut stream);
        respond(&mut stream, &request);
        request
    });
    (url, server)
}

// ----------------------------------------------------------------------------
fn read_request(stream: &mut TcpStream) -> String {
    let mut data = Vec::new();
    let mut buf = [0u8; 4096];
    loop {
        let n = stream.read(&mut buf).unwrap();
        if n == 0 {
            break;
        }
        data.extend_from_slice(&buf[..n]);
        let text = String::from_utf8_lossy(&data);
        if let Some(end) = text.find("\r\n\r\n") {
            let len = text[..end]
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().unwrap())
                })
                .unwrap_or(0);
            if data.len() >= end + 4 + len {
                break;
            }
        }
    }
    String::from_utf8(data).unwrap()
}

// ----------------------------------------------------------------------------
fn reply(stream: &mut TcpStream, status: &str, headers: &str, body: &[u8]) {
    let head = format!(
        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n{headers}\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes()).unwrap();
    stream.write_all(body).unwrap();
}

// ----------------------------------------------------------------------------
/// Hold the connection until the client hangs up.
fn stall(stream: &mut TcpStream) {
    let _ = stream.read(&mut [0u8; 1]);
}

// ----------------------------------------------------------------------------
#[test]
fn get_sends_query_and_headers() {
    let (url, server) = serve(|s, _| reply(s, "200 OK", "", b"ok"));
    let resp = client()
        .request(Method::Get, &format!("{url}/a/b?x=1"))
        .query(&[("q", "a b&c"), ("id", "me@example.com")])
        .bearer_auth("tok")
        .header("If-Match", "\"etag\"")
        .send()
        .unwrap();
    assert_eq!(resp.status(), 200);
    assert_eq!(resp.bytes().unwrap(), b"ok");

    let request = server.join().unwrap();
    assert!(
        request.starts_with("GET /a/b?x=1&q=a%20b%26c&id=me%40example.com HTTP/1.1\r\n"),
        "{request}"
    );
    assert!(
        request.contains("\r\nAuthorization: Bearer tok\r\n"),
        "{request}"
    );
    assert!(request.contains("\r\nIf-Match: \"etag\"\r\n"), "{request}");
    assert!(
        request
            .to_ascii_lowercase()
            .contains("accept-encoding: gzip, deflate"),
        "decompression is on: {request}"
    );
}

// ----------------------------------------------------------------------------
#[test]
fn post_sends_json_and_form_bodies() {
    let (url, server) = serve(|s, _| reply(s, "201 Created", "", b"{\"id\":7}"));
    let resp = client()
        .request(Method::Post, &url)
        .json(&serde_json::json!({"a": [1, 2]}))
        .unwrap()
        .send()
        .unwrap();
    assert_eq!(resp.status(), 201);
    let value: serde_json::Value = resp.json().unwrap();
    assert_eq!(value["id"], 7);
    let request = server.join().unwrap();
    assert!(request.starts_with("POST / HTTP/1.1\r\n"), "{request}");
    assert!(
        request.contains("\r\nContent-Type: application/json\r\n"),
        "{request}"
    );
    assert!(request.contains("\r\nContent-Length: 11\r\n"), "{request}");
    assert!(request.ends_with("\r\n\r\n{\"a\":[1,2]}"), "{request}");

    let (url, server) = serve(|s, _| reply(s, "200 OK", "", b""));
    let resp = client()
        .request(Method::Patch, &url)
        .form(&[("token", "a/b+c"), ("grant_type", "refresh_token")])
        .send()
        .unwrap();
    assert_eq!(resp.bytes().unwrap(), b"");
    let request = server.join().unwrap();
    assert!(request.starts_with("PATCH / HTTP/1.1\r\n"), "{request}");
    assert!(
        request.contains("\r\nContent-Type: application/x-www-form-urlencoded\r\n"),
        "{request}"
    );
    assert!(
        request.ends_with("\r\n\r\ntoken=a%2Fb%2Bc&grant_type=refresh_token"),
        "{request}"
    );
}

// ----------------------------------------------------------------------------
#[test]
fn multipart_body_has_fields_and_files() {
    let (url, server) = serve(|s, _| reply(s, "200 OK", "", b""));
    let resp = client()
        .request(Method::Post, &url)
        .multipart(&[
            FormPart::text("purpose", "user_data"),
            FormPart::file("file", "a \"b\".txt", "text/plain", b"hello\r\n"),
        ])
        .send()
        .unwrap();
    assert_eq!(resp.status(), 200);
    let request = server.join().unwrap();

    let marker = "Content-Type: multipart/form-data; boundary=";
    let start = request.find(marker).expect(&request) + marker.len();
    let boundary = &request[start..request[start..].find("\r\n").unwrap() + start];
    let body = &request[request.find("\r\n\r\n").unwrap() + 4..];
    assert_eq!(
        body,
        format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"purpose\"\r\n\r\n\
             user_data\r\n\
             --{boundary}\r\nContent-Disposition: form-data; name=\"file\"; \
             filename=\"a %22b%22.txt\"\r\nContent-Type: text/plain\r\n\r\n\
             hello\r\n\r\n\
             --{boundary}--\r\n"
        )
    );
}

// ----------------------------------------------------------------------------
#[test]
fn response_headers_are_readable() {
    let (url, server) = serve(|s, _| {
        reply(
            s,
            "200 OK",
            "X-Goog-Upload-URL: https://example.com/upload?id=1\r\n",
            b"",
        )
    });
    let resp = client().request(Method::Get, &url).send().unwrap();
    assert_eq!(
        resp.header("x-goog-upload-url").as_deref(),
        Some("https://example.com/upload?id=1")
    );
    assert_eq!(resp.header("Content-Length").as_deref(), Some("0"));
    assert_eq!(resp.header("X-Missing"), None);
    server.join().unwrap();
}

// ----------------------------------------------------------------------------
#[test]
fn error_statuses_are_responses() {
    let (url, server) = serve(|s, _| reply(s, "412 Precondition Failed", "", b"{\"error\":1}"));
    let resp = client().request(Method::Delete, &url).send().unwrap();
    assert_eq!(resp.status(), 412);
    assert!(!resp.is_success());
    assert_eq!(resp.bytes().unwrap(), b"{\"error\":1}");
    server.join().unwrap();
}

// ----------------------------------------------------------------------------
/// A reason parser as an API's: `{"reason": …, "message": …}`.
fn reason_of(body: &[u8]) -> Option<(String, String)> {
    let json: serde_json::Value = serde_json::from_slice(body).ok()?;
    let field = |name: &str| json[name].as_str().map(str::to_string);
    Some((field("reason")?, field("message")?))
}

// ----------------------------------------------------------------------------
#[test]
fn a_logged_send_answers_success_and_refusals_as_errors() {
    let (url, server) = serve(|s, _| reply(s, "200 OK", "", b"{}"));
    let resp = client().request(Method::Get, &url).send_logged(reason_of);
    assert_eq!(resp.unwrap().bytes().unwrap(), b"{}");
    server.join().unwrap();

    let body = br#"{"reason":"conditionNotMet","message":"Precondition Failed"}"#;
    let (url, server) = serve(|s, _| reply(s, "412 Precondition Failed", "", body));
    let err = client()
        .request(Method::Patch, &url)
        .send_logged(reason_of)
        .err();
    let (reason, msg) = ("conditionNotMet".into(), "Precondition Failed".into());
    assert_eq!(
        format!("{err:?}"),
        format!(
            "{:?}",
            Some(Error::Http {
                status: 412,
                reason,
                msg
            })
        )
    );
    server.join().unwrap();

    // A body the parser doesn't know is the message.
    let (url, server) =
        serve(|s, _| reply(s, "502 Bad Gateway", "", b" <html>Bad Gateway</html>\n"));
    let err = client()
        .request(Method::Get, &url)
        .send_logged(reason_of)
        .err();
    let msg = "<html>Bad Gateway</html>".into();
    let expected = Error::Http {
        status: 502,
        reason: String::new(),
        msg,
    };
    assert_eq!(format!("{err:?}"), format!("{:?}", Some(expected)));
    server.join().unwrap();

    let cancel = Cancel::new();
    cancel.cancel();
    let client = client();
    let req = client
        .request(Method::Get, "http://127.0.0.1:9/")
        .cancel_with(&cancel);
    assert!(matches!(req.send_logged(reason_of), Err(Error::Cancelled)));
}

// ----------------------------------------------------------------------------
#[test]
fn chunked_body_streams() {
    let (url, server) = serve(|s, _| {
        s.write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n")
            .unwrap();
        for chunk in ["data: one\n\n", "data: two\n\n"] {
            s.write_all(format!("{:x}\r\n{chunk}\r\n", chunk.len()).as_bytes())
                .unwrap();
            s.flush().unwrap();
            thread::sleep(Duration::from_millis(50));
        }
        s.write_all(b"0\r\n\r\n").unwrap();
    });
    let mut resp = client().request(Method::Get, &url).send().unwrap();
    let mut body = String::new();
    let mut buf = [0u8; 4];
    loop {
        match resp.read(&mut buf).unwrap() {
            0 => break,
            n => body.push_str(std::str::from_utf8(&buf[..n]).unwrap()),
        }
    }
    assert_eq!(body, "data: one\n\ndata: two\n\n");
    server.join().unwrap();
}

// ----------------------------------------------------------------------------
/// A read hands over what has arrived, without waiting for its buffer to
/// fill: a streamed answer shows piece by piece.
#[test]
fn a_read_returns_what_has_arrived() {
    let (url, server) = serve(|s, _| {
        s.write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n")
            .unwrap();
        s.write_all(b"4\r\none\n\r\n").unwrap();
        s.flush().unwrap();
        thread::sleep(Duration::from_millis(1500));
        s.write_all(b"0\r\n\r\n").unwrap();
    });
    let mut resp = client().request(Method::Get, &url).send().unwrap();
    let started = Instant::now();
    let mut buf = [0u8; 8192];
    let n = resp.read(&mut buf).unwrap();
    assert_eq!(&buf[..n], b"one\n");
    assert!(
        started.elapsed() < Duration::from_millis(1000),
        "waited {:?}",
        started.elapsed()
    );
    assert_eq!(resp.read(&mut buf).unwrap(), 0);
    server.join().unwrap();
}

// ----------------------------------------------------------------------------
#[test]
fn gzip_body_is_decompressed() {
    // "hello, gzip", gzip -n -9
    const GZIP: [u8; 31] = [
        0x1f, 0x8b, 0x08, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02, 0x03, 0xcb, 0x48, 0xcd, 0xc9, 0xc9,
        0xd7, 0x51, 0x48, 0xaf, 0xca, 0x2c, 0x00, 0x00, 0x4a, 0x9b, 0xb1, 0x5c, 0x0b, 0x00, 0x00,
        0x00,
    ];
    let (url, server) = serve(|s, _| reply(s, "200 OK", "Content-Encoding: gzip\r\n", &GZIP));
    let resp = client().request(Method::Get, &url).send().unwrap();
    assert_eq!(resp.bytes().unwrap(), b"hello, gzip");
    server.join().unwrap();
}

// ----------------------------------------------------------------------------
#[test]
fn stalled_server_times_out() {
    let (url, server) = serve(|s, _| stall(s));
    let started = Instant::now();
    let result = client()
        .request(Method::Get, &url)
        .timeout(Duration::from_millis(300))
        .send();
    assert!(matches!(result, Err(Error::Timeout)), "{:?}", result.err());
    assert!(started.elapsed() < Duration::from_secs(5));
    server.join().unwrap();
}

// ----------------------------------------------------------------------------
#[test]
fn cancel_aborts_waiting_for_the_response() {
    let (url, server) = serve(|s, _| stall(s));
    let cancel = Cancel::new();
    let canceller = {
        let cancel = cancel.clone();
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(300));
            cancel.cancel();
        })
    };
    let started = Instant::now();
    let result = client()
        .request(Method::Get, &url)
        .cancel_with(&cancel)
        .send();
    assert!(
        matches!(result, Err(Error::Cancelled)),
        "{:?}",
        result.err()
    );
    assert!(started.elapsed() < Duration::from_secs(5));
    canceller.join().unwrap();
    server.join().unwrap();
}

// ----------------------------------------------------------------------------
#[test]
fn cancel_aborts_reading_the_body() {
    let (url, server) = serve(|s, _| {
        s.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\nConnection: close\r\n\r\npart")
            .unwrap();
        stall(s);
    });
    let cancel = Cancel::new();
    let mut resp = client()
        .request(Method::Get, &url)
        .cancel_with(&cancel)
        .send()
        .unwrap();
    let mut buf = [0u8; 4];
    assert_eq!(resp.read(&mut buf).unwrap(), 4);

    let canceller = {
        let cancel = cancel.clone();
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(300));
            cancel.cancel();
        })
    };
    let err = resp.bytes().unwrap_err();
    assert!(matches!(err, Error::Cancelled), "{err:?}");
    canceller.join().unwrap();
    server.join().unwrap();

    // A cancelled `Cancel` fails later requests before they connect.
    let result = client()
        .request(Method::Get, &url)
        .cancel_with(&cancel)
        .send();
    assert!(
        matches!(result, Err(Error::Cancelled)),
        "{:?}",
        result.err()
    );
}

// ----------------------------------------------------------------------------
#[test]
fn refused_connection_is_a_network_error() {
    let port = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let result = client()
        .request(Method::Get, &format!("http://127.0.0.1:{port}/"))
        .send();
    match result {
        // With the HTTP stack's text.
        Err(Error::Network { msg }) => assert!(msg.len() > 10, "{msg}"),
        other => panic!("unexpected {:?}", other.err()),
    }
}

// ----------------------------------------------------------------------------
#[test]
#[ignore = "needs the internet"]
fn live_https_uses_http2() {
    let resp = client()
        .request(Method::Get, "https://www.googleapis.com/discovery/v1/apis")
        .query(&[("name", "calendar")])
        .send()
        .unwrap();
    assert_eq!(resp.status(), 200);
    assert!(resp.http2());
    let value: serde_json::Value = resp.json().unwrap();
    assert!(value["items"].is_array(), "{value}");
}
