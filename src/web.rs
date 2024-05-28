use std::io::prelude::*;
use std::net::{TcpListener, TcpStream};

use crate::ltc::FrameChannel;
use crate::Opt;

pub fn listen(opt: &Opt, ltc_reciver: FrameChannel) {
    let port = format!("127.0.0.1:{}", opt.port);
    let listener = TcpListener::bind(&port).unwrap();

    println!("listening on {}", &port);

    for stream in listener.incoming() {
        let stream = stream.unwrap();
        handle_connection(stream, &ltc_reciver);
    }
}

fn handle_connection(mut stream: TcpStream, ltc_reciver: &FrameChannel) {
    let res = ltc_reciver.recv();
    println!("Timecode Logged: {:#?}", res);

    let status_line = "HTTP/1.1 200 OK";
    let content = format!(
        r##"
        <!DOCTYPE html>
        <html lang="en">
          <head>
            <meta charset="utf-8">
            <title>Hello!</title>
          </head>
          <body>
            <p>Timecode Logged: {}</p>
          </body>
        </html>
    "##,
        res.format_time()
    );

    let length = content.len();
    let response = format!("{status_line}\r\nContent-Length: {length}\r\n\r\n{content}");
    stream.write_all(response.as_bytes()).unwrap();
}
