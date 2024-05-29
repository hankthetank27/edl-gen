use vtc::Timecode;

use crate::edl::CutRecord;
use crate::ltc_decode::{DecodeHandlers, LTCListener};
use crate::Opt;
use std::collections::VecDeque;
use std::io::{prelude::*, BufReader};
use std::net::{TcpListener, TcpStream};

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
    ) -> Result<CutRecord, anyhow::Error> {
        self.count += 1;
        let record = CutRecord::new(timecode, self.count, edit_type, source_tape, av_channnel)?;
        self.log.push_back(record.clone());
        Ok(record)
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
    let buf_reader = BufReader::new(&mut stream);
    let request_line = buf_reader.lines().next().unwrap().unwrap();

    let (status_line, content) = match request_line.as_str() {
        "GET /start HTTP/1.1" => {
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

        "GET /log HTTP/1.1" => try_get_frame(decode_handlers, log),

        "GET /stop HTTP/1.1" => {
            decode_handlers.decode_off().unwrap();
            let (status_line, content) = try_get_frame(decode_handlers, log);
            let content = format!("Stopped decoding with {}", content);
            (status_line, content)
        }

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
                .unwrap();
            let prev_record = log.pop().unwrap();

            let status_line = "HTTP/1.1 200 OK".to_string();
            let content = format!(
                "Cut #{} logged: {} -- {}",
                prev_record.edit_number(),
                prev_record.source_timecode(),
                curr_record.source_timecode()
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
