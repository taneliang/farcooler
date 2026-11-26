// Report-only static asset fixture. CSP comes from the production Worker.
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::{env, fs, io};

fn respond(mut stream: TcpStream, dist: &Path, policy: &str) -> io::Result<()> {
    let mut request = String::new();
    BufReader::new(&stream).read_line(&mut request)?;
    let path = request.split_whitespace().nth(1).unwrap_or("").split('?').next().unwrap_or("");
    if path == "/favicon.ico" {
        return write!(stream, "HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n");
    }
    let (name, kind) = match path {
        "/" => ("index.html", "text/html; charset=utf-8"),
        asset if asset.starts_with("/a/") && !asset[3..].contains('/') => {
            let kind = match asset.rsplit('.').next() {
                Some("js") => "text/javascript; charset=utf-8",
                Some("css") => "text/css; charset=utf-8",
                _ => "",
            };
            (&asset[1..], kind)
        }
        _ => ("", ""),
    };
    let body = (!kind.is_empty()).then(|| fs::read(dist.join(name)).ok()).flatten();
    if let Some(body) = body {
        write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nContent-Security-Policy: {policy}\r\nX-Content-Type-Options: nosniff\r\nConnection: close\r\n\r\n", body.len())?;
        if request.starts_with("GET ") {
            stream.write_all(&body)?;
        }
    } else {
        write!(stream, "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")?;
    }
    Ok(())
}

fn main() -> io::Result<()> {
    let mut args = env::args().skip(1);
    let dist = args.next().expect("distribution directory");
    let policy = args.next().expect("content security policy");
    let listener = TcpListener::bind("127.0.0.1:0")?;
    println!("{}", listener.local_addr()?);
    for connection in listener.incoming() {
        respond(connection?, Path::new(&dist), &policy)?;
    }
    Ok(())
}
