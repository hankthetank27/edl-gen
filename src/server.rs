use crate::cut_log::CutLog;
use crate::edl::{AVChannels, Edit, Edl};
use crate::ltc_decode::{DecodeErr, DecodeHandlers, LTCListener};
use crate::Opt;
use anyhow::{anyhow, Context as AnyhowCtx, Error};
use httparse::{Request as ReqParser, Status};
use serde::{Deserialize, Serialize};
use std::io::{prelude::*, BufReader};
use std::net::{TcpListener, TcpStream};

pub struct Server<'a> {
    port: String,
    opt: &'a Opt,
}

impl<'a> Server<'a> {
    pub fn new(opt: &'a Opt) -> Self {
        Server {
            port: format!("127.0.0.1:{}", opt.port),
            opt,
        }
    }

    pub fn listen(&mut self, ltc_listener: LTCListener) -> Result<(), Error> {
        let listener =
            TcpListener::bind(&self.port).context("Server could not initate TCP connection")?;
        let mut ctx = Context {
            edl: Edl::new(self.opt)?,
            decode_handlers: ltc_listener.start_decode_stream(),
            cut_log: CutLog::new(),
        };

        println!("listening on {}", &self.port);

        for stream in listener.incoming() {
            self.handle_connection(stream?, &mut ctx)
                .unwrap_or_else(|e| {
                    eprintln!("Request could not be sent: {:#}", e);
                });
        }

        Ok(())
    }

    fn handle_connection(&mut self, mut stream: TcpStream, ctx: &mut Context) -> Result<(), Error> {
        let mut buf_reader = BufReader::new(&mut stream);
        let mut headers = [httparse::EMPTY_HEADER; 16];

        let res: GenericResponse =
            Request::new(&mut ReqParser::new(&mut headers), buf_reader.fill_buf()?)?
                .route(ctx)
                .unwrap_or_else(|e| {
                    eprintln!("Error processing request: {:#}", e);
                    server_err()
                })
                .into();

        stream.write_all(res.value.as_bytes())?;

        Ok(())
    }
}

#[derive(Debug)]
pub struct Context<'req> {
    cut_log: CutLog,
    decode_handlers: DecodeHandlers<'req>,
    edl: Edl,
}

#[derive(Debug)]
struct Response {
    content: String,
    status_line: String,
}

#[derive(Debug)]
pub struct Request<'r> {
    headers: &'r mut [httparse::Header<'r>],
    method: Option<&'r str>,
    path: Option<&'r str>,
    header_offset: usize,
    buffer: &'r [u8],
}

impl<'r> Request<'r> {
    //TODO: break all this stuff off the 'req' property into own properties, including a body
    fn new(req: &'r mut ReqParser<'r, 'r>, buffer: &'r [u8]) -> Result<Self, Error> {
        let header_offset = match req.parse(buffer) {
            Ok(Status::Complete(header_offset)) => Ok(header_offset),
            //TODO: this is funky. try with firefox and see.
            Ok(Status::Partial) => Ok(req.headers.len()),
            Err(e) => Err(anyhow!("Could not parse header lenght: {}", e)),
        }?;
        Ok(Request {
            headers: req.headers,
            method: req.method,
            path: req.path,
            header_offset,
            buffer,
        })
    }

    // fn initalize_with_offset(&mut self) -> Result<usize, Error> {
    //     match self.req.parse(self.buffer) {
    //         Ok(Status::Complete(header_offset)) => Ok(header_offset),
    //         Ok(Status::Partial) => Ok(self.req.headers.len()),
    //         Err(e) => Err(anyhow!("Could not parse header lenght: {}", e)),
    //     }
    // }

    fn route(&mut self, ctx: &mut Context) -> Result<Response, Error> {
        match self.method {
            Some("POST") => match self.path {
                Some("/start") => {
                    ctx.decode_handlers.decode_on()?;
                    ctx.cut_log.clear();
                    println!("wating for audio...");
                    let mut response = self.body()?.wait_for_first_frame(ctx)?;
                    println!("ready!");
                    response.content = format!("Started decoding. {}", response.content);
                    Ok(response)
                }
                Some("/stop") => {
                    ctx.decode_handlers.decode_off()?;
                    let mut response = self.body()?.try_log_edit(ctx)?;
                    response.content = format!("Stopped decoding with {}", response.content);
                    Ok(response)
                }
                Some("/log") => self.body()?.try_log_edit(ctx),
                _ => Ok(not_found()),
            },
            _ => Ok(not_found()),
        }
    }

    fn body(&mut self) -> Result<EditRequestData, Error> {
        let body_length = self
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

        let body_start = self.header_offset;
        let body_end = body_start + body_length;
        let body = &self.buffer[body_start..body_end];
        let body_str = std::str::from_utf8(body).context("ReqParser body is not valid UTF-8")?;
        serde_json::from_str(body_str).context("ReqParser body is not valid JSON")
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct EditRequestData {
    edit_type: String,
    source_tape: String,
    av_channel: AVChannels,
}

impl EditRequestData {
    fn wait_for_first_frame(&self, ctx: &mut Context) -> Result<Response, Error> {
        let tc = ctx.decode_handlers.recv_frame()?;
        ctx.cut_log
            .push(tc, &self.edit_type, &self.source_tape, &self.av_channel)?;
        Ok(format!("timecode logged: {:#?}", tc.timecode()).into())
    }

    fn try_log_edit(&self, ctx: &mut Context) -> Result<Response, Error> {
        match self.parse_edit_from_log(ctx) {
            Ok(edit) => Ok(ctx.edl.write_from_edit(edit)?.into()),
            Err(DecodeErr::NoVal(_)) => Ok(frame_unavailable()),
            Err(e) => Err(Error::msg(e)),
        }
    }

    fn parse_edit_from_log(&self, ctx: &mut Context) -> Result<Edit, DecodeErr> {
        let tc = ctx.decode_handlers.try_recv_frame()?;
        ctx.cut_log
            .push(tc, &self.edit_type, &self.source_tape, &self.av_channel)?;
        let prev_record = ctx.cut_log.pop().context("No value in cut_log")?;
        let curr_record = ctx.cut_log.front().context("No value in cut_log")?;
        Ok(Edit::from_cuts(&prev_record, curr_record)?)
    }
}

impl From<String> for Response {
    fn from(value: String) -> Self {
        let content = format!("{:#?}", value);
        let status_line = "HTTP/1.1 200 OK".to_string();

        Response {
            status_line,
            content,
        }
    }
}

struct GenericResponse {
    value: String,
}

impl From<Response> for GenericResponse {
    fn from(value: Response) -> Self {
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
            value.content
        );

        let length = content.len();
        let status_line = value.status_line;
        GenericResponse {
            value: format!("{status_line}\r\nContent-Length: {length}\r\n\r\n{content}"),
        }
    }
}

fn frame_unavailable() -> Response {
    Response {
        status_line: "HTTP/1.1 200 OK".to_string(),
        content: "Unable to get timecode. Make sure source is streaming and decoding has started."
            .to_string(),
    }
}

fn server_err() -> Response {
    Response {
        status_line: "HTTP/1.1 500 INTERNAL SERVER ERROR".to_string(),
        content: "Failed to parse request".to_string(),
    }
}

fn not_found() -> Response {
    Response {
        status_line: "HTTP/1.1 404 NOT FOUND".to_string(),
        content: "Command not found".to_string(),
    }
}
