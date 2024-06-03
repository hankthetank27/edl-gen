use crate::cut_log::CutLog;
use crate::edl::{AVChannels, Edit, Edl};
use crate::ltc_decode::{DecodeErr, DecodeHandlers, LTCListener};
use crate::Opt;
use anyhow::Error;
use anyhow::{anyhow, Context};
use httparse::{Request, Status};
use serde::{Deserialize, Serialize};
use std::io::prelude::*;
use std::io::BufReader;
use std::net::{TcpListener, TcpStream};
use std::usize;

pub struct Server<'a> {
    port: String,
    cut_log: CutLog,
    opt: &'a Opt,
}

impl<'a> Server<'a> {
    pub fn new(opt: &'a Opt) -> Self {
        Server {
            port: format!("127.0.0.1:{}", opt.port),
            cut_log: CutLog::new(),
            opt,
        }
    }

    pub fn listen(&mut self, ltc_listener: LTCListener) -> Result<(), Error> {
        let listener =
            TcpListener::bind(&self.port).context("Server could not initate TCP connection")?;
        let decode_handlers = ltc_listener.start_decode_stream();
        let edl = Edl::new(self.opt)?;

        println!("listening on {}", &self.port);

        for stream in listener.incoming() {
            self.handle_connection(stream?, &decode_handlers, &edl)
                .unwrap_or_else(|e| {
                    eprintln!("Request could not be sent: {:#}", e);
                });
        }

        Ok(())
    }

    fn handle_connection(
        &mut self,
        mut stream: TcpStream,
        decode_handlers: &DecodeHandlers,
        edl: &Edl,
    ) -> Result<(), Error> {
        let mut buf_reader = BufReader::new(&mut stream);
        let mut headers = [httparse::EMPTY_HEADER; 16];

        let res: GenericResponse = ReqContext::new(
            Request::new(&mut headers),
            buf_reader.fill_buf()?,
            &mut self.cut_log,
            decode_handlers,
        )
        .handle_req()
        .unwrap_or_else(|e| {
            eprintln!("Error processing request: {:#}", e);
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
    // TODO: attach open file here
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

    fn handle_req(&mut self) -> Result<ResContent, Error> {
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

    fn route_req(&mut self, body: &EditRequest) -> Result<ResContent, Error> {
        match self.req.method {
            Some("POST") => match self.req.path {
                Some("/start") => {
                    self.decode_handlers.decode_on()?;
                    self.cut_log.clear();
                    println!("wating for audio...");
                    let (status_line, content) =
                        body.wait_for_first_frame(self.decode_handlers, self.cut_log)?;
                    let content = format!("Started decoding. {}", content);
                    println!("{}", content);
                    Ok((status_line, content))
                }
                Some("/stop") => {
                    self.decode_handlers.decode_off()?;
                    let (status_line, content) =
                        body.try_log_edit(self.decode_handlers, self.cut_log)?;
                    Ok((status_line, format!("Stopped decoding with {}", content)))
                }
                Some("/log") => body.try_log_edit(self.decode_handlers, self.cut_log),
                _ => Ok(not_found()),
            },
            _ => Ok(not_found()),
        }
    }

    fn parse_req_body(&self, header_len: usize) -> Result<EditRequest, Error> {
        let body_length = self
            .req
            .headers
            .iter()
            .find(|header| header.name.to_lowercase() == "content-length")
            .ok_or_else(|| anyhow!("'Content-Length' header is missing"))
            .and_then(|header| {
                std::str::from_utf8(header.value)
                    .context("'Content-Length' header is not valid UTF-8")
            })
            .and_then(|header| {
                header
                    .parse::<usize>()
                    .context("'Content-Length' header is not a valid number")
            })?;

        let body_start = header_len;
        let body_end = body_start + body_length;
        let body = &self.buffer[body_start..body_end];
        let body_str = std::str::from_utf8(body).context("Request body is not valid UTF-8")?;
        serde_json::from_str(body_str).context("Request body is not valid JSON")
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct EditRequest {
    edit_type: String,
    source_tape: String,
    av_channel: AVChannels,
}

impl EditRequest {
    fn wait_for_first_frame(
        &self,
        decode_handlers: &DecodeHandlers,
        log: &mut CutLog,
    ) -> Result<ResContent, Error> {
        let tc = decode_handlers.recv_frame()?;
        log.push(tc, &self.edit_type, &self.source_tape, &self.av_channel)?;

        let status_line = "HTTP/1.1 200 OK".to_string();
        let content = format!("timecode logged: {:#?}", tc.timecode());
        Ok((status_line, content))
    }

    fn try_log_edit(
        &self,
        decode_handlers: &DecodeHandlers,
        log: &mut CutLog,
    ) -> Result<ResContent, Error> {
        match self.parse_edit_from_log(decode_handlers, log) {
            Ok(edit) => Ok(edit.log_edit()?.into()),
            Err(DecodeErr::NoVal(_)) => Ok(frame_unavailable()),
            Err(e) => Err(Error::msg(e)),
        }
    }

    fn parse_edit_from_log(
        &self,
        decode_handlers: &DecodeHandlers,
        log: &mut CutLog,
    ) -> Result<Edit, DecodeErr> {
        let tc = decode_handlers.try_recv_frame()?;
        log.push(tc, &self.edit_type, &self.source_tape, &self.av_channel)?;
        let prev_record = log.pop().context("No value in cut_log")?;
        let curr_record = log.front().context("No value in cut_log")?;
        Ok(Edit::from_cuts(&prev_record, curr_record)?)
    }
}

impl From<Edit> for ResContent {
    fn from(value: Edit) -> Self {
        let content = format!("{:#?}", value);
        let status_line = "HTTP/1.1 200 OK".to_string();
        (status_line, content)
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

fn frame_unavailable() -> ResContent {
    (
        "HTTP/1.1 200 OK".to_string(),
        "Unable to get timecode. Make sure source is streaming and decoding has started."
            .to_string(),
    )
}

fn server_err() -> ResContent {
    (
        "HTTP/1.1 500 INTERNAL SERVER ERROR".to_string(),
        "Failed to parse request".to_string(),
    )
}

fn not_found() -> ResContent {
    (
        "HTTP/1.1 404 NOT FOUND".to_string(),
        "Command not found".to_string(),
    )
}
