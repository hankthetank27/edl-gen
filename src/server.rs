use crate::edl::CutRecord;
use crate::ltc_decode::{DecodeHandlers, LTCListener};
use crate::Opt;
use anyhow::Error;
use httparse::{Request, Status};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::io::prelude::*;
use std::io::BufReader;
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

    pub fn listen(&mut self, ltc_listener: LTCListener) -> Result<(), anyhow::Error> {
        let listener = TcpListener::bind(&self.port).unwrap();
        let decode_handlers = ltc_listener.start_decode_stream();

        println!("listening on {}", &self.port);

        for stream in listener.incoming() {
            let stream = stream.unwrap();
            self.handle_connection(stream, &decode_handlers)?;
        }
        Ok(())
    }

    fn handle_connection(
        &mut self,
        mut stream: TcpStream,
        decode_handlers: &DecodeHandlers,
    ) -> Result<(), anyhow::Error> {
        let mut buf_reader = BufReader::new(&mut stream);
        let mut headers = [httparse::EMPTY_HEADER; 16];

        let mut req_ctx = ReqContext::new(
            Request::new(&mut headers),
            buf_reader.fill_buf().unwrap(),
            &mut self.cut_log,
            decode_handlers,
        );

        let res: GenericResponse = req_ctx
            .handle_req()
            .unwrap_or_else(|e| {
                eprintln!("{e}");
                server_err()
            })
            .into();

        stream.write_all(res.value.as_bytes())?;
        Ok(())
    }
}

type ResContent = (String, String);

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

    fn handle_req(&mut self) -> Result<ResContent, anyhow::Error> {
        match self.req.parse(self.buffer) {
            Ok(Status::Complete(header_len)) => {
                //TODO: parse_req_body should return a body struct (whatever that looks like) and
                //pass to route_req.
                let body = self.parse_req_body(header_len)?;
                self.route_req(&body)
            }

            //TODO: idk if this acutally works with the headers.len() call
            Ok(Status::Partial) => {
                let body = self.parse_req_body(self.req.headers.len())?;
                self.route_req(&body)
            }
            Err(e) => {
                eprint!("Error parsing request: {}", e);
                Ok(server_err())
            }
        }
    }

    fn route_req(&mut self, body: &EditRequest) -> Result<ResContent, anyhow::Error> {
        match self.req.method {
            Some("POST") => match self.req.path {
                Some("/start") => {
                    self.decode_handlers.decode_on().unwrap();
                    self.cut_log.clear();
                    println!("wating for audio...");
                    let (status_line, content) =
                        body.wait_for_frame(self.decode_handlers, self.cut_log)?;
                    let content = format!("Started decoding. {}", content);
                    println!("{}", content);
                    Ok((status_line, content))
                }
                Some("/stop") => {
                    self.decode_handlers.decode_off().unwrap();
                    let (status_line, content) =
                        body.try_get_frame(self.decode_handlers, self.cut_log)?;
                    Ok((status_line, format!("Stopped decoding with {}", content)))
                }
                Some("/log") => body.try_get_frame(self.decode_handlers, self.cut_log),
                _ => Ok(not_found()),
            },
            _ => Ok(not_found()),
        }
    }

    //TODO: this should return a parsed body value, and ideally be able to chain the frame processing
    //methods to it (ie. wait_for_frame, try_get_frame).
    fn parse_req_body(&self, header_len: usize) -> Result<EditRequest, anyhow::Error> {
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
            .unwrap_or(0);

        let body_start = header_len;
        let body_end = body_start + body_length;
        let body = &self.buffer[body_start..body_end];
        let body_str = std::str::from_utf8(body).map_err(anyhow::Error::msg)?;
        serde_json::from_str(body_str).map_err(anyhow::Error::msg)
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct EditRequest {
    edit_type: String,
    source_tape: String,
    av_channel: String,
}

impl EditRequest {
    fn wait_for_frame(
        &self,
        decode_handlers: &DecodeHandlers,
        log: &mut CutLog,
    ) -> Result<ResContent, anyhow::Error> {
        let tc = decode_handlers.recv_frame().map_err(anyhow::Error::msg)?;
        log.push(tc, &self.edit_type, &self.source_tape, &self.av_channel)
            .map_err(anyhow::Error::msg)?;

        let status_line = "HTTP/1.1 200 OK".to_string();
        let content = format!("timecode logged: {:#?}", tc.timecode());
        Ok((status_line, content))
    }

    fn try_get_frame(
        &self,
        decode_handlers: &DecodeHandlers,
        log: &mut CutLog,
    ) -> Result<ResContent, anyhow::Error> {
        match decode_handlers.try_recv_frame() {
            Ok(tc) => {
                let curr_record = log
                    .push(tc, &self.edit_type, &self.source_tape, &self.av_channel)
                    .map_err(anyhow::Error::msg)?
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
                Ok((status_line, content))
            }
            Err(_) => {
                let status_line = "HTTP/1.1 200 OK".to_string();
                let content =
                "Unable to get timecode. Make sure source is streaming and decoding has started."
                    .to_string();
                Ok((status_line, content))
            }
        }
    }
}

struct GenericResponse {
    value: String,
}

impl From<ResContent> for GenericResponse {
    fn from(value: ResContent) -> Self {
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

fn server_err() -> ResContent {
    let status_line = "HTTP/1.1 500 INTERNAL SERVER ERROR".to_string();
    let content = "Failed to parse request".to_string();
    (status_line, content)
}

fn not_found() -> ResContent {
    let status_line = "HTTP/1.1 404 NOT FOUND".to_string();
    let content = "Command not found".to_string();
    (status_line, content)
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
        edit_type: &str,
        source_tape: &str,
        av_channnel: &str,
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
