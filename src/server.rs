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

            // TODO: we might need to handle get_frame differently here in the
            // case there is no audio to decode, as it blocks the thread
            let (status_line, content) = get_frame(decode_handlers);
            (status_line, format!("Started decoding. {}", content))
        }

        "GET /stop HTTP/1.1" => {
            decode_handlers
                .decode_state_sender
                .send(DecodeState::Off)
                .unwrap();

            let status_line = "HTTP/1.1 200 OK".to_string();
            let content = "Stopped decoding".to_string();
            (status_line, content)
        }

        "GET /log HTTP/1.1" => try_get_frame(decode_handlers),

        _ => {
            let status_line = "HTTP/1.1 404 NOT FOUND".to_string();
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

fn get_frame(decode_handlers: &DecodeHandlers) -> (String, String) {
    let tc = decode_handlers.frame_recv.recv();
    let status_line = "HTTP/1.1 200 OK".to_string();
    let content = format!("timecode logged: {}", tc.format_time());
    println!("Timecode Logged: {:#?}", tc);
    (status_line, content)
}

fn try_get_frame(decode_handlers: &DecodeHandlers) -> (String, String) {
    match decode_handlers.frame_recv.try_recv() {
        Some(tc) => {
            let status_line = "HTTP/1.1 200 OK".to_string();
            let content = format!("timecode logged: {}", tc.format_time());
            println!("Timecode Logged: {:#?}", tc);
            (status_line, content)
        }
        None => {
            let status_line = "HTTP/1.1 200 OK".to_string();
            let content =
                "Unable to get timecode. Make sure source is streaming and decoding has started."
                    .to_string();
            (status_line, content)
        }
    }
}
