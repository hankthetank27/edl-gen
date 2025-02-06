use anyhow::{anyhow, Context as AnyhowCtx, Error};
use httparse::{Request as ReqParser, Status};
use parking_lot::{Mutex, MutexGuard};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use std::io::{prelude::*, BufReader};
use std::net::{TcpListener, TcpStream};

use std::sync::mpsc::Sender;
use std::time::Duration;
use std::{
    sync::{mpsc, Arc},
    thread,
};

use crate::edl_writer::frame_queue::EditType;
use crate::{
    edl_writer::{frame_queue::FrameQueue, AVChannels, Edit, Edl, FrameDataPair},
    ltc_decoder::{DecodeErr, DecodeHandlers},
    state::Opt,
};

pub struct Server {
    host: String,
}

impl Server {
    pub fn new(port: u16) -> Self {
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
        let (tx_ltc_wait_worker, rx_ltc_wait_worker) =
            mpsc::channel::<(EditRequestData, Context)>();
        let mut ctx: Context = Arc::new(Mutex::new(ContextInner {
            frame_queue: FrameQueue::new(),
            rec_state: EdlRecordingState::Stopped,
            selected_src_data: SourceTapeRequestData::default(),
            decode_handlers: Arc::new(decode_handlers),
            tx_ltc_wait_worker,
            edl: None,
            opt,
        }));

        log::info!("Server launched and listening at {}", &self.host);

        // Spawn a dedicated worker thread for waiting on LTC start
        thread::spawn(move || {
            while let Ok((mut req_data, mut ctx)) = rx_ltc_wait_worker.recv() {
                match req_data.wait_for_first_frame(&mut ctx) {
                    Ok(body) => body.recording_status,
                    Err(e) => {
                        log::error!("Unable to log start: {e}");
                        ctx.lock().set_rec_state(EdlRecordingState::Stopped)
                    }
                };
            }
        });

        for stream in listener.incoming() {
            self.handle_connection(stream?, &mut ctx)
                .unwrap_or_else(|e| {
                    log::error!("Server error: {:#}", e);
                });
            match rx_stop_serv.lock().try_recv() {
                Ok(_) => break,
                Err(mpsc::TryRecvError::Empty) => continue,
                Err(e) => log::error!("Unable to read halt server message: {}", e),
            }
        }

        tx_serv_stopped.send(())?;
        log::info!("\nServer stopped.");
        Ok(())
    }

    fn handle_connection(&mut self, mut socket: TcpStream, ctx: &mut Context) -> Result<(), Error> {
        let mut buf_reader = BufReader::new(&mut socket);
        let mut headers = [httparse::EMPTY_HEADER; 16];
        let mut headers = ReqParser::new(&mut headers);

        let res = buf_reader
            .fill_buf()
            .context("Unable to fill buffer")
            .and_then(|buf| Request::new(&mut headers, buf))
            .and_then(|mut req| req.route(ctx))
            .and_then(|res| res.json())
            .unwrap_or_else(|e| {
                log::error!("Error processing request: {:#}", e);
                server_err()
            });

        socket
            .write_all(SerializedResponse::from(res).value.as_bytes())
            .context("Response could not be sent")
    }
}

#[derive(Debug, Clone, Copy)]
enum StatusCode {
    S200,
    S202,
    S404,
    S418,
    S500,
}

type Context = Arc<Mutex<ContextInner>>;

pub struct ContextInner {
    frame_queue: FrameQueue,
    decode_handlers: Arc<DecodeHandlers>,
    edl: Option<Edl>,
    rec_state: EdlRecordingState,
    selected_src_data: SourceTapeRequestData,
    tx_ltc_wait_worker: Sender<(EditRequestData, Context)>,
    opt: Opt,
}

//Here we will put the websocket notifcations
impl ContextInner {
    fn set_rec_state(&mut self, state: EdlRecordingState) -> EdlRecordingState {
        self.rec_state = state;
        state
    }
}

#[derive(Debug, Copy, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[cfg_attr(test, derive(PartialEq))]
enum EdlRecordingState {
    Started,
    Stopped,
    Waiting,
}

enum EditBody {
    Edit(Edit),
    Edits(Vec<Edit>),
}

#[derive(Serialize, Debug)]
#[cfg_attr(test, derive(Deserialize))]
struct ResBody {
    recording_status: EdlRecordingState,
    edit: Option<Edit>,
    final_edits: Option<Vec<Edit>>,
}

impl ResBody {
    fn new(recording_status: EdlRecordingState, edit_or_edits: Option<EditBody>) -> Self {
        let (edit, final_edits) = edit_or_edits
            .map(|eb| match eb {
                EditBody::Edit(e) => (Some(e), None),
                EditBody::Edits(es) => (None, Some(es)),
            })
            .unwrap_or((None, None));
        ResBody {
            recording_status,
            edit,
            final_edits,
        }
    }
}

#[derive(Debug)]
struct Response {
    content: Value,
    status: StatusCode,
}

impl Response {
    fn new(content: Value, status: StatusCode) -> Self {
        Response { content, status }
    }

    fn json(mut self) -> Result<Self, Error> {
        self.content =
            serde_json::to_value(&self.content).context("Could not parse HTTP Response to JSON")?;
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
            Err(e) => Err(anyhow!("Could not parse header length: {}", e)),
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
                Some("/start") => self.handle_start(ctx).inspect_err(|_| {
                    ctx.lock().set_rec_state(EdlRecordingState::Stopped);
                }),
                Some("/end") => self.handle_end(ctx).inspect_err(|_| {
                    ctx.lock().set_rec_state(EdlRecordingState::Started);
                }),
                Some("/log") => self.handle_log(ctx),
                Some("/select-src") => self.handle_select_src(ctx),
                _ => Ok(not_found()),
            },
            Some("GET") => match self.path {
                Some("/edl-recording-state") => {
                    ResBody::new(ctx.lock().rec_state, None).try_into_200()
                }
                Some("/SIGKILL") => Ok(kill_server()),
                _ => Ok(not_found()),
            },
            _ => Ok(not_found()),
        }
    }

    fn handle_start(&mut self, ctx: &mut Context) -> Result<Response, Error> {
        let mut ctx_guard = ctx.lock();
        match ctx_guard.rec_state {
            EdlRecordingState::Stopped => {
                ctx_guard.set_rec_state(EdlRecordingState::Waiting);
                log::info!("EDL recording start requested. Waiting for LTC signal.");

                ctx_guard.decode_handlers.decode_on()?;
                ctx_guard.frame_queue.clear();
                ctx_guard.edl = Some(Edl::new(&ctx_guard.opt)?);

                let mut edit_req = self
                    .body()?
                    .context("Request does not contain a body")?
                    .expect_edit()?;

                edit_req
                    .try_start_now(&mut ctx_guard)
                    .and_then(|res| res.try_into_200().map_err(StartErr::Anyhow))
                    .or_else(|err| match err {
                        StartErr::Timeout => {
                            let ctx_send = Arc::clone(ctx);
                            ctx_guard.tx_ltc_wait_worker.send((edit_req, ctx_send))?;
                            ResBody::new(ctx_guard.rec_state, None).try_into_202()
                        }
                        StartErr::Anyhow(e) => Err(e),
                    })
            }
            s @ EdlRecordingState::Started | s @ EdlRecordingState::Waiting => {
                log::warn!("Recording has already started. You cannot start in this state.");
                ResBody::new(s, None).try_into_202()
            }
        }
    }

    fn handle_end(&mut self, ctx: &mut Context) -> Result<Response, Error> {
        let mut ctx_guard = ctx.lock();
        match ctx_guard.rec_state {
            EdlRecordingState::Started => {
                ctx_guard.set_rec_state(EdlRecordingState::Waiting);
                log::info!("Ending recording...");

                let edits = self
                    .body()?
                    .context("Request does not contain a body")?
                    .expect_edit()?
                    .try_log_final_edit(&mut ctx_guard)?;

                ctx_guard.decode_handlers.decode_off()?;
                let rec_state = ctx_guard.set_rec_state(EdlRecordingState::Stopped);
                log::info!("EDL recording ended");

                ResBody::new(rec_state, Some(EditBody::Edits(edits))).try_into_200()
            }
            EdlRecordingState::Waiting => {
                log::info!("Ending recording...");
                ctx_guard.decode_handlers.decode_off()?;
                let rec_state = ctx_guard.set_rec_state(EdlRecordingState::Stopped);
                log::info!("EDL recording ended");
                ResBody::new(rec_state, None).try_into_200()
            }
            s @ EdlRecordingState::Stopped => {
                log::warn!("Recording not yet started!");
                ResBody::new(s, None).try_into_202()
            }
        }
    }

    fn handle_log(&mut self, ctx: &mut Context) -> Result<Response, Error> {
        let mut ctx_guard = ctx.lock();
        match ctx_guard.rec_state {
            EdlRecordingState::Started => self
                .body()?
                .context("Request does not contain a body")?
                .expect_edit()?
                .try_log_edit(&mut ctx_guard)?
                .try_into_200(),
            s @ EdlRecordingState::Stopped | s @ EdlRecordingState::Waiting => {
                log::warn!("Recording not yet started!");
                ResBody::new(s, None).try_into_202()
            }
        }
    }

    fn handle_select_src(&mut self, ctx: &mut Context) -> Result<Response, Error> {
        self.body()?
            .context("Request does not contain a body")?
            .expect_source()?
            .try_select_src(ctx)
    }

    fn body(&mut self) -> Result<Option<ReqBody>, Error> {
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

        match body_length >= 1 {
            true => {
                let body_start = self.header_offset;
                let body_end = body_start + body_length;
                let body = &self.buffer[body_start..body_end];
                let body_str =
                    std::str::from_utf8(body).context("Request body is not valid UTF-8")?;
                Ok(Some(
                    serde_json::from_str(body_str).context("Request body is not valid JSON")?,
                ))
            }
            false => Ok(None),
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase", tag = "req_type")]
#[cfg_attr(test, derive(Serialize))]
pub enum ReqBody {
    Edit(EditRequestData),
    Source(SourceTapeRequestData),
}

impl ReqBody {
    fn expect_source(self) -> Result<SourceTapeRequestData, Error> {
        match self {
            ReqBody::Source(src) => Ok(src),
            ReqBody::Edit(_) => Err(anyhow!(
                "Unexpected request type: expected source, got edit"
            )),
        }
    }

    fn expect_edit(self) -> Result<EditRequestData, Error> {
        match self {
            ReqBody::Edit(src) => Ok(src),
            ReqBody::Source(_) => Err(anyhow!(
                "Unexpected request type: expected edit, got source"
            )),
        }
    }
}

#[derive(Deserialize)]
#[cfg_attr(test, derive(Serialize))]
pub struct EditRequestData {
    pub(crate) edit_type: String,
    pub(crate) edit_duration_frames: Option<u32>,
    pub(crate) wipe_num: Option<u32>,
    pub(crate) source_tape: Option<String>,
    pub(crate) av_channels: Option<AVChannels>,
}

enum StartErr {
    Timeout,
    Anyhow(Error),
}

impl EditRequestData {
    fn blank_frame() -> Self {
        EditRequestData {
            edit_type: "cut".into(),
            edit_duration_frames: None,
            wipe_num: None,
            source_tape: None,
            av_channels: None,
        }
    }

    fn try_log_edit(&mut self, ctx_guard: &mut MutexGuard<ContextInner>) -> Result<ResBody, Error> {
        Ok(ResBody::new(
            EdlRecordingState::Started,
            Some(EditBody::Edit(self.write_edit_to_edl(ctx_guard)?)),
        ))
    }

    fn try_start_now(
        &mut self,
        ctx_guard: &mut MutexGuard<ContextInner>,
    ) -> Result<ResBody, StartErr> {
        self.try_queue_current_frame(ctx_guard)
            .map_err(|e| match e {
                DecodeErr::Timeout => StartErr::Timeout,
                _ => StartErr::Anyhow(anyhow!("Error decoding frame: {e}")),
            })?;
        Ok(ResBody::new(
            ctx_guard.set_rec_state(EdlRecordingState::Started),
            None,
        ))
    }

    fn try_log_final_edit(
        &mut self,
        ctx_guard: &mut MutexGuard<ContextInner>,
    ) -> Result<Vec<Edit>, Error> {
        self.source_tape = None;
        self.av_channels = None;
        self.write_edit_to_edl(ctx_guard).and_then(|e| {
            let mut res = vec![e];
            match EditType::try_from(self.edit_type.as_str()).unwrap() {
                EditType::Cut => (),
                _ => res.push(EditRequestData::blank_frame().write_edit_to_edl(ctx_guard)?),
            };
            Ok(res)
        })
    }

    fn write_edit_to_edl(
        &mut self,
        ctx_guard: &mut MutexGuard<ContextInner>,
    ) -> Result<Edit, Error> {
        self.build_edit_from_current_and_prev(ctx_guard)
            .context("Could not log edit")
            .and_then(|edit| {
                ctx_guard
                    .edl
                    .as_mut()
                    .context("EDL file does not exist")?
                    .write_from_edit(edit)
            })
    }

    fn build_edit_from_current_and_prev(
        &mut self,
        ctx_guard: &mut MutexGuard<ContextInner>,
    ) -> Result<Edit, DecodeErr> {
        self.try_queue_current_frame(ctx_guard)?;
        let prev_record = ctx_guard
            .frame_queue
            .pop()
            .context("No previous value in frame_queue")?;
        let curr_record = ctx_guard
            .frame_queue
            .front()
            .context("No current value in frame_queue")?;
        Ok(FrameDataPair::new(&prev_record, curr_record).try_into()?)
    }

    fn try_queue_current_frame(
        &mut self,
        ctx_guard: &mut MutexGuard<ContextInner>,
    ) -> Result<(), DecodeErr> {
        let tc = ctx_guard
            .decode_handlers
            .recv_frame_timeout(Duration::from_millis(1000))?;
        let edit_data = self.map_selected_source(ctx_guard);
        ctx_guard.frame_queue.push(tc, edit_data)?;
        Ok(())
    }

    pub fn map_selected_source(&mut self, ctx_guard: &MutexGuard<ContextInner>) -> &Self {
        if self.source_tape.is_none() {
            self.source_tape = ctx_guard.selected_src_data.source_tape.clone();
        }
        if self.av_channels.is_none() {
            self.av_channels = ctx_guard.selected_src_data.av_channels;
        }
        self
    }

    fn wait_for_first_frame(&mut self, ctx: &mut Context) -> Result<ResBody, Error> {
        let decode_handlers = Arc::clone(&ctx.lock().decode_handlers);
        let tc = decode_handlers.recv_frame()?;
        let edit_data = self.map_selected_source(&ctx.lock());

        let mut ctx_guard = ctx.lock();
        ctx_guard.frame_queue.push(tc, edit_data)?;
        Ok(ResBody::new(
            ctx_guard.set_rec_state(EdlRecordingState::Started),
            None,
        ))
    }
}

#[derive(Debug, Deserialize, Default, Clone, Serialize)]
pub struct SourceTapeRequestData {
    source_tape: Option<String>,
    av_channels: Option<AVChannels>,
}

impl SourceTapeRequestData {
    fn try_select_src(&self, ctx: &mut Context) -> Result<Response, Error> {
        ctx.lock().selected_src_data = self.clone();
        if let Some(src) = self.source_tape.as_ref() {
            log::info!("Source tape selected: {}", src);
        }
        if let Some(av) = self.av_channels {
            log::info!("AV channels selected: {}", String::from(av));
        }
        Ok(Response::new(serde_json::to_value(self)?, StatusCode::S200))
    }
}

trait IntoResponse {
    type Error;
    fn try_into_200(&self) -> Result<Response, Self::Error>;
    fn try_into_202(&self) -> Result<Response, Self::Error>;
}

impl IntoResponse for ResBody {
    type Error = Error;

    fn try_into_200(&self) -> Result<Response, Self::Error> {
        Ok(Response::new(
            serde_json::to_value(self).context("Could not serialize response body")?,
            StatusCode::S200,
        ))
    }

    fn try_into_202(&self) -> Result<Response, Self::Error> {
        Ok(Response::new(
            serde_json::to_value(self).context("Could not serialize response body")?,
            StatusCode::S202,
        ))
    }
}

struct SerializedResponse {
    value: String,
}

impl From<Response> for SerializedResponse {
    fn from(res: Response) -> Self {
        let content = res.content.to_string();
        let length = content.len();
        let status_line: &str = res.status.into();
        SerializedResponse {
            value: format!(
                "HTTP/1.1 {status_line}\r\nContent-Type: application/json\r\nContent-Length: {length}\r\n\r\n{content}"
            ),
        }
    }
}

impl From<StatusCode> for &str {
    fn from(value: StatusCode) -> Self {
        match value {
            StatusCode::S200 => "200 OK",
            StatusCode::S202 => "202 ACCEPTED",
            StatusCode::S404 => "404 NOT FOUND",
            StatusCode::S418 => "418 I'M A TEAPOT",
            StatusCode::S500 => "500 INTERNAL SERVER ERROR",
        }
    }
}

fn kill_server() -> Response {
    Response::new("Exiting...".into(), StatusCode::S418)
}

fn server_err() -> Response {
    Response::new("Failed to parse request".into(), StatusCode::S500)
}

fn not_found() -> Response {
    Response::new("Command not found".into(), StatusCode::S404)
}

#[cfg(test)]
mod test;
