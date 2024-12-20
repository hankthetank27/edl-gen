use anyhow::{anyhow, Context as AnyhowCtx, Error};
use eframe::egui::mutex::Mutex;
use httparse::{Request as ReqParser, Status};
use serde::{Deserialize, Serialize};
use std::io::{prelude::*, BufReader};
use std::net::{TcpListener, TcpStream};
use std::sync::{mpsc, Arc};

use crate::edl::{AVChannels, Edit, Edl, FrameDataPair};
use crate::frame_queue::FrameQueue;
use crate::ltc_decode::{DecodeErr, DecodeHandlers};
use crate::state::Opt;

pub struct Server {
    host: String,
}

impl Server {
    pub fn new(port: &usize) -> Self {
        Server {
            host: format!("127.0.0.1:{}", port),
        }
    }

    pub fn listen(
        &mut self,
        rx_stop_serv: Arc<Mutex<mpsc::Receiver<()>>>,
        tx_serv_stopped: mpsc::Sender<()>,
        decode_handlers: DecodeHandlers,
        opt: Opt,
    ) -> Result<(), Error> {
        let listener =
            TcpListener::bind(&self.host).context("Server could not initate TCP connection")?;
        let mut ctx = Context {
            frame_queue: FrameQueue::new(),
            rec_state: EdlRecordingState::Stopped,
            edl: None,
            decode_handlers,
            opt,
        };

        log::info!("Server listening on {}", &self.host);

        for stream in listener.incoming() {
            self.handle_connection(stream?, &mut ctx)
                .unwrap_or_else(|e| {
                    log::error!("Request could not be sent: {:#}", e);
                    StatusCode::S500
                });

            match rx_stop_serv.lock().try_recv() {
                Ok(_) => break,
                Err(mpsc::TryRecvError::Empty) => continue,
                Err(e) => log::error!("Unable to read halt server message: {}", e),
            }
        }

        tx_serv_stopped.send(())?;
        log::info!("Goodbye!");
        Ok(())
    }

    fn handle_connection(
        &mut self,
        mut stream: TcpStream,
        ctx: &mut Context,
    ) -> Result<StatusCode, Error> {
        let mut buf_reader = BufReader::new(&mut stream);
        let mut headers = [httparse::EMPTY_HEADER; 16];

        let res: Response =
            Request::new(&mut ReqParser::new(&mut headers), buf_reader.fill_buf()?)?
                .route(ctx)
                .unwrap_or_else(|e| {
                    log::error!("Error processing request: {:#}", e);
                    server_err()
                })
                .json()?;

        let status = res.status;
        stream.write_all(SerializedResponse::from(res).value.as_bytes())?;

        Ok(status)
    }
}

#[derive(Debug, Clone, Copy)]
enum StatusCode {
    S200,
    S404,
    S418,
    S500,
}

pub struct Context {
    frame_queue: FrameQueue,
    decode_handlers: DecodeHandlers,
    edl: Option<Edl>,
    rec_state: EdlRecordingState,
    opt: Opt,
}

#[derive(Debug)]
enum EdlRecordingState {
    Started,
    Stopped,
}

#[derive(Debug)]
struct Response {
    content: String,
    status: StatusCode,
}

impl Response {
    fn new(content: String, status: StatusCode) -> Self {
        Response { content, status }
    }

    fn json(mut self) -> Result<Self, Error> {
        self.content = serde_json::to_string(&self.content)
            .context("Could not parse HTTP Response to JSON")?;
        Ok(self)
    }
}

#[derive(Debug)]
pub struct Request<'req> {
    headers: &'req mut [httparse::Header<'req>],
    method: Option<&'req str>,
    path: Option<&'req str>,
    header_offset: usize,
    buffer: &'req [u8],
}

impl<'req> Request<'req> {
    fn new(req_parser: &'req mut ReqParser<'req, 'req>, buffer: &'req [u8]) -> Result<Self, Error> {
        let header_offset = match req_parser.parse(buffer) {
            Ok(Status::Complete(header_offset)) => Ok(header_offset),

            //TODO: this is funky. try with firefox and see.
            Ok(Status::Partial) => Ok(req_parser.headers.len()),
            Err(e) => Err(anyhow!("Could not parse header lenght: {}", e)),
        }?;

        Ok(Request {
            headers: req_parser.headers,
            method: req_parser.method,
            path: req_parser.path,
            header_offset,
            buffer,
        })
    }

    fn route(&mut self, ctx: &mut Context) -> Result<Response, Error> {
        match self.method {
            Some("POST") => match self.path {
                Some("/start") => self.handle_start(ctx),
                Some("/end") => self.handle_end(ctx),
                Some("/log") => self.handle_log(ctx),
                _ => Ok(not_found()),
            },
            Some("GET") => match self.path {
                Some("/SIGKILL") => Ok(kill_server()),
                _ => Ok(not_found()),
            },
            _ => Ok(not_found()),
        }
    }

    fn handle_start(&mut self, ctx: &mut Context) -> Result<Response, Error> {
        match &ctx.rec_state {
            EdlRecordingState::Stopped => {
                ctx.rec_state = EdlRecordingState::Started;
                ctx.decode_handlers.decode_on()?;
                ctx.frame_queue.clear();
                ctx.edl = Some(Edl::new(&ctx.opt)?);
                let mut response = self.body()?.wait_for_first_frame(ctx)?;
                response.content = format!("Started decoding. {}", response.content);
                Ok(response)
            }
            EdlRecordingState::Started => {
                let msg = "Recording has already started. You cannot start in this state.";
                log::warn!("{msg}");
                Ok(Response::new(msg.into(), StatusCode::S404))
            }
        }
        .or_else(|e| {
            ctx.rec_state = EdlRecordingState::Stopped;
            e
        })
    }

    fn handle_end(&mut self, ctx: &mut Context) -> Result<Response, Error> {
        match &ctx.rec_state {
            EdlRecordingState::Started => {
                ctx.rec_state = EdlRecordingState::Stopped;
                let mut response = self.body()?.try_log_edit(ctx)?;
                ctx.decode_handlers.decode_off()?;
                log::info!("Ended recording!");
                response.content = format!("Stopped decoding with {}", response.content);
                Ok(response)
            }
            EdlRecordingState::Stopped => {
                let msg = "Recording not yet started!";
                log::warn!("{msg}");
                Ok(Response::new(msg.into(), StatusCode::S404))
            }
            .or_else(|e| {
                ctx.rec_state = EdlRecordingState::Started;
                e
            }),
        }
    }

    fn handle_log(&mut self, ctx: &mut Context) -> Result<Response, Error> {
        match &ctx.rec_state {
            EdlRecordingState::Started => self.body()?.try_log_edit(ctx),
            EdlRecordingState::Stopped => {
                let msg = "Recording not yet started!";
                log::warn!("{msg}");
                Ok(Response::new(msg.into(), StatusCode::S404))
            }
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
pub struct EditRequestData {
    pub(crate) edit_type: String,
    pub(crate) edit_duration_frames: Option<u32>,
    pub(crate) wipe_num: Option<u32>,
    pub(crate) source_tape: String,
    pub(crate) av_channels: AVChannels,
}

impl EditRequestData {
    fn try_log_edit(&self, ctx: &mut Context) -> Result<Response, Error> {
        match self.parse_edit_from_log(ctx) {
            Ok(edit) => Ok(ctx
                .edl
                .as_mut()
                .context("EDL file does not exist")?
                .write_from_edit(edit)?
                .into()),
            Err(DecodeErr::NoVal(_)) => Ok(frame_unavailable()),
            Err(e) => Err(Error::msg(e)),
        }
    }

    fn parse_edit_from_log(&self, ctx: &mut Context) -> Result<Edit, DecodeErr> {
        let tc = ctx.decode_handlers.try_recv_frame()?;
        ctx.frame_queue.push(tc, self)?;
        let prev_record = ctx.frame_queue.pop().context("No value in frame_queue")?;
        let curr_record = ctx.frame_queue.front().context("No value in frame_queue")?;
        Ok(Edit::try_from(FrameDataPair::new(
            &prev_record,
            curr_record,
        ))?)
    }

    fn wait_for_first_frame(&self, ctx: &mut Context) -> Result<Response, Error> {
        log::info!("Waiting for timecode signal to start...");
        let tc = match ctx.decode_handlers.recv_frame() {
            Ok(f) => f,
            Err(DecodeErr::NoVal(_)) => return Ok("Exited".to_string().into()),
            Err(DecodeErr::Anyhow(e)) => return Err(anyhow!(e)),
        };
        ctx.frame_queue.push(tc, self)?;
        log::info!("Timecode signal detected and recording started!\n");
        Ok(format!("timecode logged: {:#?}", tc.timecode()).into())
    }
}

impl From<String> for Response {
    fn from(value: String) -> Self {
        Response::new(value, StatusCode::S200)
    }
}

struct SerializedResponse {
    value: String,
}

impl From<Response> for SerializedResponse {
    fn from(value: Response) -> Self {
        let content = value.content;
        let length = content.len();
        let status_line = String::from(value.status);

        SerializedResponse {
            value: format!(
                "{status_line}\r\nContent-Type: application/json\r\nContent-Length: {length}\r\n\r\n{content}"
            ),
        }
    }
}

impl From<StatusCode> for String {
    fn from(value: StatusCode) -> Self {
        match value {
            StatusCode::S200 => "200 OK",
            StatusCode::S404 => "404 NOT FOUND",
            StatusCode::S418 => "418 I'M A TEAPOT",
            StatusCode::S500 => "500 INTERNAL SERVER ERROR",
        }
        .to_string()
    }
}

fn kill_server() -> Response {
    Response::new("Exiting...".into(), StatusCode::S418)
}

fn frame_unavailable() -> Response {
    Response::new(
        "Unable to get timecode. Make sure source is streaming and decoding has started.".into(),
        StatusCode::S200,
    )
}

fn server_err() -> Response {
    Response::new("Failed to parse request".into(), StatusCode::S500)
}

fn not_found() -> Response {
    Response::new("Command not found".into(), StatusCode::S404)
}
