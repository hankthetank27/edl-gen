use crate::edl::CutRecord;
use crate::ltc_decode::{DecodeHandlers, LTCListener};
use crate::Opt;
use anyhow::Error;
use httparse::Request;
use std::collections::VecDeque;
use std::io::prelude::*;
use std::net::{TcpListener, TcpStream};
use vtc::Timecode;

struct CutLog {
    log: VecDeque<CutRecord>,
    count: usize,
}

impl CutLog {
    fn new() -> Self {
        CutLog {
            log: VecDeque::new(),
            count: 0,
        }
    }

    fn clear(&mut self) {
        self.count = 0;
        self.log.clear();
    }

    fn push(
        &mut self,
        timecode: Timecode,
        edit_type: String,
        source_tape: String,
        av_channnel: String,
    ) -> Result<&CutRecord, Error> {
        self.count += 1;
        let record = CutRecord::new(timecode, self.count, edit_type, source_tape, av_channnel)?;
        self.log.push_back(record);
        Ok(self.log.front().unwrap())
    }

    fn pop(&mut self) -> Option<CutRecord> {
        self.log.pop_front()
    }
}

pub fn listen(ltc_listener: LTCListener, opt: &Opt) {
    let port = format!("127.0.0.1:{}", opt.port);
    let listener = TcpListener::bind(&port).unwrap();
    let handles = ltc_listener.start_decode_stream();
    let mut log = CutLog::new();

    println!("listening on {}", &port);

    for stream in listener.incoming() {
        let stream = stream.unwrap();
        handle_connection(stream, &handles, &mut log);
    }
}

fn handle_connection(mut stream: TcpStream, decode_handlers: &DecodeHandlers, log: &mut CutLog) {
    let mut buffer = [0; 1024];
    stream.read(&mut buffer).unwrap();
    let mut headers = [httparse::EMPTY_HEADER; 16];
    let mut req = Request::new(&mut headers);

    let (status_line, content) = match req.parse(&buffer) {
        Ok(_) => match req.method {
            Some("GET") => match req.path {
                Some("/start") => {
                    decode_handlers.decode_on().unwrap();
                    log.clear();

                    // TODO: we might need to handle get_frame differently here in the
                    // case there is no audio to decode, as it blocks the thread
                    let tc = decode_handlers.recv_frame().unwrap();
                    log.push(
                        tc,
                        "cut".to_string(),
                        "tape1".to_string(),
                        "test".to_string(),
                    )
                    .unwrap();

                    let status_line = "HTTP/1.1 200 OK".to_string();
                    let content = format!("timecode logged: {:#?}", tc.timecode());
                    println!("Timecode Logged: {:#?}", tc.timecode());
                    (status_line, format!("Started decoding. {}", content))
                }
                Some("/log") => try_get_frame(decode_handlers, log),
                Some("/stop") => {
                    decode_handlers.decode_off().unwrap();
                    let (status_line, content) = try_get_frame(decode_handlers, log);
                    let content = format!("Stopped decoding with {}", content);
                    (status_line, content)
                }
                _ => not_found(),
            },
            Some("POST") => match req.path {
                Some("/start") => {
                    let body_length = req
                        .headers
                        .iter()
                        .find(|header| header.name == "Content-Length")
                        .unwrap();

                    println!("{:#?}", body_length);
                    todo!()
                }
                Some("/log") => todo!(),
                Some("/stop") => todo!(),
                _ => not_found(),
            },
            _ => not_found(),
        },
        Err(e) => {
            eprint!("Error parsing request: {}", e);
            let status_line = "HTTP/1.1 500 INTERNAL SERVER ERROR".to_string();
            let content = "Failed to parse request".to_string();
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

fn not_found() -> (String, String) {
    let status_line = "HTTP/1.1 404 NOT FOUND".to_string();
    let content = "Command not found".to_string();
    (status_line, content)
}

fn try_get_frame(decode_handlers: &DecodeHandlers, log: &mut CutLog) -> (String, String) {
    match decode_handlers.try_recv_frame() {
        Ok(tc) => {
            let curr_record = log
                .push(
                    tc,
                    "cut".to_string(),
                    "tape1".to_string(),
                    "test".to_string(),
                )
                .unwrap()
                .source_timecode();
            let prev_record = log.pop().unwrap();

            let status_line = "HTTP/1.1 200 OK".to_string();
            let content = format!(
                "Cut #{} logged: {} -- {}",
                prev_record.edit_number(),
                prev_record.source_timecode(),
                curr_record
            );
            println!("{content}");
            (status_line, content)
        }
        Err(_) => {
            let status_line = "HTTP/1.1 200 OK".to_string();
            let content =
                "Unable to get timecode. Make sure source is streaming and decoding has started."
                    .to_string();
            (status_line, content)
        }
    }
}
