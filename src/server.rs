use crate::ltc_decode::{DecodeHandlers, DecodeState, LTCListener};
use crate::Opt;
use std::io::{prelude::*, BufReader};
use std::net::{TcpListener, TcpStream};

pub fn listen(opt: &Opt, ltc_listener: LTCListener) {
    let port = format!("127.0.0.1:{}", opt.port);
    let listener = TcpListener::bind(&port).unwrap();
    let handles = ltc_listener.start_decode_stream();

    println!("listening on {}", &port);

    for stream in listener.incoming() {
        let stream = stream.unwrap();
        handle_connection(stream, &handles);
    }
}

fn handle_connection(mut stream: TcpStream, decode_handlers: &DecodeHandlers) {
    let buf_reader = BufReader::new(&mut stream);
    let request_line = buf_reader.lines().next().unwrap().unwrap();

    let (status_line, content) = match request_line.as_str() {
        "GET /start HTTP/1.1" => {
            decode_handlers
                .decode_state_sender
                .send(DecodeState::On)
                .unwrap();
            let status_line = "HTTP/1.1 200 OK";
            let content = "Started decoding".to_string();
            (status_line, content)
        }

        "GET /stop HTTP/1.1" => {
            decode_handlers
                .decode_state_sender
                .send(DecodeState::Off)
                .unwrap();
            let status_line = "HTTP/1.1 200 OK";
            let content = "Stopped decoding".to_string();
            (status_line, content)
        }

        "GET /log HTTP/1.1" => match decode_handlers.frame_recv.try_recv() {
            Some(tc) => {
                let status_line = "HTTP/1.1 200 OK";
                let content = format!("timecode logged: {}", tc.format_time());
                println!("Timecode Logged: {:#?}", tc);
                (status_line, content)
            }
            None => {
                let status_line = "HTTP/1.1 200 OK";
                let content = "Unable to get timecode. Try begin decoding.".to_string();
                (status_line, content)
            }
        },

        _ => {
            let status_line = "HTTP/1.1 404 NOT FOUND";
            let content = "Command not found".to_string();
            (status_line, content)
        }
    };

    let content = format!(
        r##"
            <!DOCTYPE html>
            <html lang="en">
              <head>
                <meta charset="utf-8">
                <title>EDL Generator</title>
              </head>
              <body>
                <p>{}</p>
              </body>
            </html>
        "##,
        content
    );

    let length = content.len();
    let response = format!("{status_line}\r\nContent-Length: {length}\r\n\r\n{content}");
    stream.write_all(response.as_bytes()).unwrap();
}
