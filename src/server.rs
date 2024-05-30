use crate::edl::CutRecord;
use crate::ltc_decode::{DecodeHandlers, LTCListener};
use crate::Opt;
use anyhow::Error;
use httparse::{Request, Status};
use std::collections::VecDeque;
use std::io::prelude::*;
use std::net::{TcpListener, TcpStream};
use std::usize;
use vtc::Timecode;

pub struct Server {
    port: String,
    cut_log: CutLog,
}

impl Server {
    pub fn new(opt: &Opt) -> Self {
        Server {
            port: format!("127.0.0.1:{}", opt.port),
            cut_log: CutLog::new(),
        }
    }

    pub fn listen(&mut self, ltc_listener: LTCListener) {
        let listener = TcpListener::bind(&self.port).unwrap();
        let decode_handlers = ltc_listener.start_decode_stream();

        println!("listening on {}", &self.port);

        for stream in listener.incoming() {
            let stream = stream.unwrap();
            self.handle_connection(stream, &decode_handlers);
        }
    }

    fn handle_connection(&mut self, mut stream: TcpStream, decode_handlers: &DecodeHandlers) {
        let mut buffer = [0; 1024];
        stream.read(&mut buffer).unwrap();
        let mut headers = [httparse::EMPTY_HEADER; 16];
        let req = Request::new(&mut headers);

        let res: GenericResponse =
            ReqContext::new(req, &buffer, &mut self.cut_log, decode_handlers)
                .handle_req()
                .into();

        stream.write_all(res.value.as_bytes()).unwrap();
    }
}

pub struct ReqContext<'req> {
    req: Request<'req, 'req>,
    buffer: &'req [u8],
    cut_log: &'req mut CutLog,
    decode_handlers: &'req DecodeHandlers<'req>,
}

impl<'req> ReqContext<'req> {
    fn new(
        req: Request<'req, 'req>,
        buffer: &'req [u8],
        cut_log: &'req mut CutLog,
        decode_handlers: &'req DecodeHandlers<'req>,
    ) -> Self {
        ReqContext {
            req,
            buffer,
            cut_log,
            decode_handlers,
        }
    }

    fn handle_req(&mut self) -> (String, String) {
        match self.req.parse(self.buffer) {
            Ok(Status::Complete(header_len)) => {
                //TODO: parse_req_body should return a body struct (whatever that looks like) and
                //pass to route_req.
                self.parse_req_body(header_len);
                self.route_req()
            }

            //TODO: idk if this acutally works with the headers.len() call
            Ok(Status::Partial) => {
                self.parse_req_body(self.req.headers.len());
                self.route_req()
            }
            Err(e) => {
                eprint!("Error parsing request: {}", e);
                let status_line = "HTTP/1.1 500 INTERNAL SERVER ERROR".to_string();
                let content = "Failed to parse request".to_string();
                (status_line, content)
            }
        }
    }

    fn route_req(&mut self) -> (String, String) {
        match self.req.method {
            Some("POST") => match self.req.path {
                Some("/start") => {
                    self.decode_handlers.decode_on().unwrap();
                    self.cut_log.clear();
                    let (status_line, content) =
                        wait_for_frame(self.decode_handlers, &mut self.cut_log);
                    (status_line, format!("Started decoding. {}", content))
                }
                Some("/stop") => {
                    self.decode_handlers.decode_off().unwrap();
                    let (status_line, content) =
                        try_get_frame(self.decode_handlers, &mut self.cut_log);
                    (status_line, format!("Stopped decoding with {}", content))
                }
                Some("/log") => try_get_frame(self.decode_handlers, &mut self.cut_log),
                _ => not_found(),
            },
            _ => not_found(),
        }
    }

    //TODO: this should return a parsed body value, and ideally be able to chain the frame processing
    //methods to it (ie. wait_for_frame, try_get_frame).
    fn parse_req_body(&self, header_len: usize) {
        let body_length = self
            .req
            .headers
            .iter()
            .find(|header| header.name == "Content-Length")
            .and_then(|header| {
                std::str::from_utf8(header.value)
                    .ok()?
                    .parse::<usize>()
                    .ok()
            })
            .unwrap();

        let body_start = header_len;
        let body_end = body_start + body_length;
        let body = &self.buffer[body_start..body_end];
        let body_str = std::str::from_utf8(body).unwrap();

        println!("{:#?}", body_str);
    }
}

struct GenericResponse {
    value: String,
}

impl From<(String, String)> for GenericResponse {
    fn from(value: (String, String)) -> Self {
        let (status_line, content) = value;
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
        GenericResponse {
            value: format!("{status_line}\r\nContent-Length: {length}\r\n\r\n{content}"),
        }
    }
}

fn not_found() -> (String, String) {
    let status_line = "HTTP/1.1 404 NOT FOUND".to_string();
    let content = "Command not found".to_string();
    (status_line, content)
}

// TODO: we might need to handle get_frame differently here in the
// case there is no audio to decode, as it blocks the thread.
// add timeout?
fn wait_for_frame(decode_handlers: &DecodeHandlers, log: &mut CutLog) -> (String, String) {
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
