use anyhow::{anyhow, Context as AnyhowCtx, Error};
use httparse::{Request as ReqParser, Status};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::{TcpListener, TcpStream},
};

use std::sync::{mpsc, Arc};

use crate::{
    edl_writer::{frame_queue::FrameQueue, AVChannels, Edit, Edl, FrameDataPair},
    ltc_decoder::{DecodeErr, DecodeHandlers},
    state::Opt,
};

pub struct Server {
    host: String,
}

impl Server {
    pub fn new(port: &usize) -> Self {
        Server {
            host: format!("127.0.0.1:{}", port),
        }
    }

    #[tokio::main]
    pub async fn listen(
        &mut self,
        rx_stop_serv: Arc<Mutex<mpsc::Receiver<()>>>,
        tx_serv_stopped: mpsc::Sender<()>,
        decode_handlers: DecodeHandlers,
        opt: Opt,
    ) -> Result<(), Error> {
        let listener = TcpListener::bind(&self.host)
            .await
            .context("Server could not initate TCP connection")?;
        let ctx: Context = Arc::new(Mutex::new(ContextInner {
            frame_queue: FrameQueue::new(),
            rec_state: EdlRecordingState::Stopped,
            selected_src_tape: None,
            decode_handlers: Arc::new(decode_handlers),
            edl: None,
            opt,
        }));

        log::info!("Server launched and listening at {}", &self.host);

        loop {
            let (socket, _) = listener.accept().await.context("Unable to connect")?;
            // A new task is spawned for each inbound socket. The socket is
            // moved to the new task and processed there.
            let ctx = Arc::clone(&ctx);
            tokio::spawn(async move {
                Server::handle_connection(socket, ctx)
                    .await
                    .unwrap_or_else(|e| {
                        log::error!("Request could not be sent: {:#}", e);
                        StatusCode::S500
                    });
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

    async fn handle_connection(
        mut socket: TcpStream,
        mut ctx: Context,
    ) -> Result<StatusCode, Error> {
        let mut buf_reader = BufReader::new(&mut socket);
        let mut headers = [httparse::EMPTY_HEADER; 16];

        let res: Response = Request::new(
            &mut ReqParser::new(&mut headers),
            buf_reader.fill_buf().await?,
        )?
        .route(&mut ctx)
        .unwrap_or_else(|e| {
            log::error!("Error processing request: {:#}", e);
            server_err()
        })
        .json()?;

        let status = res.status;
        socket
            .write_all(SerializedResponse::from(res).value.as_bytes())
            .await?;
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

type Context = Arc<Mutex<ContextInner>>;

pub struct ContextInner {
    frame_queue: FrameQueue,
    decode_handlers: Arc<DecodeHandlers>,
    edl: Option<Edl>,
    rec_state: EdlRecordingState,
    selected_src_tape: Option<String>,
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
                Some("/start") => self.handle_start(ctx).or_else(|e| {
                    ctx.lock().rec_state = EdlRecordingState::Stopped;
                    Err(e)
                }),
                Some("/end") => self.handle_end(ctx).or_else(|e| {
                    ctx.lock().rec_state = EdlRecordingState::Started;
                    Err(e)
                }),
                Some("/log") => self.handle_log(ctx),
                Some("/select-src") => self.handle_select_src(ctx),
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
        let mut ctx_guard = ctx.lock();
        match ctx_guard.rec_state {
            EdlRecordingState::Stopped => {
                ctx_guard.rec_state = EdlRecordingState::Started;
                ctx_guard.decode_handlers.decode_on()?;
                ctx_guard.frame_queue.clear();
                ctx_guard.edl = Some(Edl::new(&ctx_guard.opt)?);
                // Drop the mutex lock before potentially long operations
                drop(ctx_guard);
                let mut response = self.body()?.expect_edit()?.wait_for_first_frame(ctx)?;
                response.content = format!("Started decoding. {}", response.content);
                Ok(response)
            }
            EdlRecordingState::Started => {
                let msg = "Recording has already started. You cannot start in this state.";
                log::warn!("{msg}");
                Ok(Response::new(msg.into(), StatusCode::S404))
            }
        }
    }

    fn handle_end(&mut self, ctx: &mut Context) -> Result<Response, Error> {
        let mut ctx_guard = ctx.lock();
        match ctx_guard.rec_state {
            EdlRecordingState::Started => {
                ctx_guard.rec_state = EdlRecordingState::Stopped;
                drop(ctx_guard);
                let mut response = self.body()?.expect_edit()?.try_log_edit(ctx)?;
                ctx.lock().decode_handlers.decode_off()?;
                log::info!("\nEnded recording.");
                response.content = format!("Stopped decoding with {}", response.content);
                Ok(response)
            }
            EdlRecordingState::Stopped => {
                let msg = "Recording not yet started!";
                log::warn!("{msg}");
                Ok(Response::new(msg.into(), StatusCode::S404))
            }
        }
    }

    fn handle_log(&mut self, ctx: &mut Context) -> Result<Response, Error> {
        let ctx_guard = ctx.lock();
        match ctx_guard.rec_state {
            EdlRecordingState::Started => {
                drop(ctx_guard);
                self.body()?.expect_edit()?.try_log_edit(ctx)
            }
            EdlRecordingState::Stopped => {
                let msg = "Recording not yet started!";
                log::warn!("{msg}");
                Ok(Response::new(msg.into(), StatusCode::S404))
            }
        }
    }

    fn handle_select_src(&mut self, ctx: &mut Context) -> Result<Response, Error> {
        self.body()?.expect_source()?.try_select_src(ctx)
    }

    fn body(&mut self) -> Result<ReqBody, Error> {
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
#[serde(untagged)]
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

#[derive(Debug, Serialize, Deserialize)]
pub struct EditRequestData {
    pub(crate) edit_type: String,
    pub(crate) edit_duration_frames: Option<u32>,
    pub(crate) wipe_num: Option<u32>,
    pub(crate) source_tape: Option<String>,
    pub(crate) av_channels: AVChannels,
}

impl EditRequestData {
    fn try_log_edit(&mut self, ctx: &mut Context) -> Result<Response, Error> {
        match self.parse_edit_from_log(ctx) {
            Ok(edit) => {
                let mut ctx_guard = ctx.lock();
                Ok(ctx_guard
                    .edl
                    .as_mut()
                    .context("EDL file does not exist")?
                    .write_from_edit(edit)?
                    .into())
            }
            Err(DecodeErr::NoVal(_)) => Ok(frame_unavailable()),
            Err(e) => Err(Error::msg(e)),
        }
    }

    fn parse_edit_from_log(&mut self, ctx: &mut Context) -> Result<Edit, DecodeErr> {
        let tc = ctx.lock().decode_handlers.try_recv_frame()?;
        let edit_data = self.map_selected_source(ctx);
        let mut ctx_guard = ctx.lock();
        ctx_guard.frame_queue.push(tc, edit_data)?;
        let prev_record = ctx_guard
            .frame_queue
            .pop()
            .context("No value in frame_queue")?;
        let curr_record = ctx_guard
            .frame_queue
            .front()
            .context("No value in frame_queue")?;
        Ok(Edit::try_from(FrameDataPair::new(
            &prev_record,
            curr_record,
        ))?)
    }

    fn wait_for_first_frame(&mut self, ctx: &mut Context) -> Result<Response, Error> {
        log::info!("\nWaiting for timecode signal to start...");
        let decode_handlers = Arc::clone(&ctx.lock().decode_handlers);
        let tc = match decode_handlers.recv_frame() {
            Ok(f) => f,
            Err(DecodeErr::NoVal(_)) => return Ok("Exited".to_string().into()),
            Err(DecodeErr::Anyhow(e)) => return Err(anyhow!(e)),
        };
        let edit_data = self.map_selected_source(ctx);
        ctx.lock().frame_queue.push(tc, edit_data)?;
        log::info!("Timecode signal detected and recording started.");
        Ok(format!("timecode logged: {:#?}", tc.timecode()).into())
    }

    pub fn map_selected_source(&mut self, ctx: &Context) -> &Self {
        if self.source_tape.is_none() {
            self.source_tape = ctx.lock().selected_src_tape.clone();
        }
        self
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SourceTapeRequestData {
    source_tape: String,
}

impl SourceTapeRequestData {
    fn try_select_src(&self, ctx: &mut Context) -> Result<Response, Error> {
        ctx.lock().selected_src_tape = Some(self.source_tape.clone());
        let msg = format!("Source tape selected: {}", self.source_tape);
        log::info!("{msg}");
        Ok(msg.into())
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
                "HTTP/1.1 {status_line}\r\nContent-Type: application/json\r\nContent-Length: {length}\r\n\r\n{content}"
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

#[cfg(test)]
mod test {
    use eframe::egui::Context;
    use parking_lot::Mutex;

    use crate::{
        edl_writer::AVChannels,
        ltc_decoder::LTCListener,
        server::{EditRequestData, Server, SourceTapeRequestData},
        state::Logger,
        test::{cpal_device::MockDevice, state::test_opt},
    };
    use std::{
        sync::{mpsc, Arc},
        thread,
        time::Duration,
    };

    struct TestServer {
        device: MockDevice,
        port: usize,
        tx_stop_serv: mpsc::Sender<()>,
    }

    // TODO: add test for multiple start events triggering while waiting
    impl TestServer {
        fn new(port: usize, file_name: String) -> Self {
            let opt = test_opt(port, file_name);
            let device = opt.ltc_device.as_ref().unwrap().device.clone();

            Logger::init(&Context::default());

            let decode_handlers = LTCListener::new(opt.clone()).unwrap().listen();
            let (tx_stop_serv, rx_stop_serv) = mpsc::channel::<()>();
            let (tx_serv_stopped, _rx_serv_stopped) = mpsc::channel::<()>();
            let rx_stop_serv = Arc::new(Mutex::new(rx_stop_serv));

            thread::spawn(move || {
                Server::new(&opt.port)
                    .listen(rx_stop_serv, tx_serv_stopped, decode_handlers, opt)
                    .unwrap();
            });
            thread::sleep(Duration::from_millis(300));

            Self {
                device,
                port,
                tx_stop_serv,
            }
        }
    }

    #[test]
    fn test_basic_edit_events() {
        let TestServer {
            device,
            port,
            tx_stop_serv,
        } = TestServer::new(6670, "test_basic_edit_events".to_string());

        device.tx_start_playing.send(()).unwrap();

        let start_res = minreq::post(format!("http://127.0.0.1:{port}/start"))
            .with_header("Content-Type", "application/json")
            .with_body(
                serde_json::to_string(&EditRequestData {
                    edit_type: "cut".into(),
                    edit_duration_frames: None,
                    wipe_num: None,
                    source_tape: Some("tape1".into()),
                    av_channels: AVChannels::default(),
                })
                .unwrap(),
            )
            .send()
            .unwrap();
        let cut_1_res = minreq::post(format!("http://127.0.0.1:{port}/log"))
            .with_header("Content-Type", "application/json")
            .with_body(
                serde_json::to_string(&EditRequestData {
                    edit_type: "cut".into(),
                    edit_duration_frames: None,
                    wipe_num: None,
                    source_tape: Some("tape2".into()),
                    av_channels: AVChannels::default(),
                })
                .unwrap(),
            )
            .send()
            .unwrap();
        let cut_2_res = minreq::post(format!("http://127.0.0.1:{port}/log"))
            .with_header("Content-Type", "application/json")
            .with_body(
                serde_json::to_string(&EditRequestData {
                    edit_type: "cut".into(),
                    edit_duration_frames: None,
                    wipe_num: None,
                    source_tape: Some("tape1".into()),
                    av_channels: AVChannels::default(),
                })
                .unwrap(),
            )
            .send()
            .unwrap();
        let wipe_1_res = minreq::post(format!("http://127.0.0.1:{port}/log"))
            .with_header("Content-Type", "application/json")
            .with_body(
                serde_json::to_string(&EditRequestData {
                    edit_type: "wipe".into(),
                    edit_duration_frames: Some(2),
                    wipe_num: None,
                    source_tape: Some("tape1".into()),
                    av_channels: AVChannels::default(),
                })
                .unwrap(),
            )
            .send()
            .unwrap();
        let dis_1_res = minreq::post(format!("http://127.0.0.1:{port}/log"))
            .with_header("Content-Type", "application/json")
            .with_body(
                serde_json::to_string(&EditRequestData {
                    edit_type: "dissolve".into(),
                    edit_duration_frames: Some(2),
                    wipe_num: None,
                    source_tape: Some("tape1".into()),
                    av_channels: AVChannels::default(),
                })
                .unwrap(),
            )
            .send()
            .unwrap();
        let end_res = minreq::post(format!("http://127.0.0.1:{port}/end"))
            .with_header("Content-Type", "application/json")
            .with_body(
                serde_json::to_string(&EditRequestData {
                    edit_type: "dissolve".into(),
                    edit_duration_frames: Some(2),
                    wipe_num: None,
                    source_tape: Some("tape1".into()),
                    av_channels: AVChannels::default(),
                })
                .unwrap(),
            )
            .send()
            .unwrap();

        assert_eq!(start_res.status_code, 200);
        assert_eq!(cut_1_res.status_code, 200);
        assert_eq!(cut_2_res.status_code, 200);
        assert_eq!(wipe_1_res.status_code, 200);
        assert_eq!(dis_1_res.status_code, 200);
        assert_eq!(end_res.status_code, 200);

        tx_stop_serv.send(()).unwrap();
    }

    #[test]
    fn test_wait_for_ltc_on_start() {
        let TestServer {
            device,
            port,
            tx_stop_serv,
        } = TestServer::new(6908, "test_wait_for_ltc_on_start".to_string());

        let handle = thread::spawn(move || {
            let start_res = minreq::post(format!("http://127.0.0.1:{port}/start"))
                .with_header("Content-Type", "application/json")
                .with_body(
                    serde_json::to_string(&EditRequestData {
                        edit_type: "cut".into(),
                        edit_duration_frames: None,
                        wipe_num: None,
                        source_tape: Some("tape1".into()),
                        av_channels: AVChannels::default(),
                    })
                    .unwrap(),
                )
                .send()
                .unwrap();
            let cut_1_res = minreq::post(format!("http://127.0.0.1:{port}/log"))
                .with_header("Content-Type", "application/json")
                .with_body(
                    serde_json::to_string(&EditRequestData {
                        edit_type: "cut".into(),
                        edit_duration_frames: None,
                        wipe_num: None,
                        source_tape: Some("tape2".into()),
                        av_channels: AVChannels::default(),
                    })
                    .unwrap(),
                )
                .send()
                .unwrap();

            assert_eq!(start_res.status_code, 200);
            assert_eq!(cut_1_res.status_code, 200);
        });

        thread::sleep(Duration::from_millis(2000));
        device.tx_start_playing.send(()).unwrap();
        handle.join().unwrap();
        tx_stop_serv.send(()).unwrap();
    }

    #[test]
    fn test_edit_events_with_preselected_src() {
        let TestServer {
            device,
            port,
            tx_stop_serv,
        } = TestServer::new(7891, "test_edit_events_with_preselected_src".to_string());

        device.tx_start_playing.send(()).unwrap();

        let src_res = minreq::post(format!("http://127.0.0.1:{port}/select-src"))
            .with_header("Content-Type", "application/json")
            .with_body(
                serde_json::to_string(&SourceTapeRequestData {
                    source_tape: "tape1".into(),
                })
                .unwrap(),
            )
            .send()
            .unwrap();
        let start_res = minreq::post(format!("http://127.0.0.1:{port}/start"))
            .with_header("Content-Type", "application/json")
            .with_body(
                serde_json::to_string(&EditRequestData {
                    edit_type: "cut".into(),
                    edit_duration_frames: None,
                    wipe_num: None,
                    source_tape: None,
                    av_channels: AVChannels::default(),
                })
                .unwrap(),
            )
            .send()
            .unwrap();
        let cut_1_res = minreq::post(format!("http://127.0.0.1:{port}/log"))
            .with_header("Content-Type", "application/json")
            .with_body(
                serde_json::to_string(&EditRequestData {
                    edit_type: "cut".into(),
                    edit_duration_frames: None,
                    wipe_num: None,
                    source_tape: Some("tape2".into()),
                    av_channels: AVChannels::default(),
                })
                .unwrap(),
            )
            .send()
            .unwrap();
        let cut_2_res = minreq::post(format!("http://127.0.0.1:{port}/log"))
            .with_header("Content-Type", "application/json")
            .with_body(
                serde_json::to_string(&EditRequestData {
                    edit_type: "cut".into(),
                    edit_duration_frames: None,
                    wipe_num: None,
                    source_tape: None,
                    av_channels: AVChannels::default(),
                })
                .unwrap(),
            )
            .send()
            .unwrap();

        assert_eq!(src_res.status_code, 200);
        assert_eq!(start_res.status_code, 200);
        assert_eq!(cut_1_res.status_code, 200);
        assert_eq!(cut_2_res.status_code, 200);

        tx_stop_serv.send(()).unwrap();
    }

    #[test]
    fn test_select_src_while_waiting() {
        let TestServer {
            device,
            port,
            tx_stop_serv,
        } = TestServer::new(6928, "test_select_src_while_waiting".to_string());

        let handle_start = thread::spawn(move || {
            let start_res = minreq::post(format!("http://127.0.0.1:{port}/start"))
                .with_header("Content-Type", "application/json")
                .with_body(
                    serde_json::to_string(&EditRequestData {
                        edit_type: "cut".into(),
                        edit_duration_frames: None,
                        wipe_num: None,
                        source_tape: None,
                        av_channels: AVChannels::default(),
                    })
                    .unwrap(),
                )
                .send()
                .unwrap();
            let cut_1_res = minreq::post(format!("http://127.0.0.1:{port}/log"))
                .with_header("Content-Type", "application/json")
                .with_body(
                    serde_json::to_string(&EditRequestData {
                        edit_type: "cut".into(),
                        edit_duration_frames: None,
                        wipe_num: None,
                        source_tape: None,
                        av_channels: AVChannels::default(),
                    })
                    .unwrap(),
                )
                .send()
                .unwrap();
            let end_res = minreq::post(format!("http://127.0.0.1:{port}/end"))
                .with_header("Content-Type", "application/json")
                .with_body(
                    serde_json::to_string(&EditRequestData {
                        edit_type: "cut".into(),
                        edit_duration_frames: None,
                        wipe_num: None,
                        source_tape: None,
                        av_channels: AVChannels::default(),
                    })
                    .unwrap(),
                )
                .send()
                .unwrap();
            assert_eq!(start_res.status_code, 200);
            assert_eq!(cut_1_res.status_code, 200);
            assert_eq!(end_res.status_code, 200);
        });

        thread::sleep(Duration::from_millis(1500));

        let handle_src = thread::spawn(move || {
            let src_res = minreq::post(format!("http://127.0.0.1:{port}/select-src"))
                .with_header("Content-Type", "application/json")
                .with_body(
                    serde_json::to_string(&SourceTapeRequestData {
                        source_tape: "tape1".into(),
                    })
                    .unwrap(),
                )
                .send()
                .unwrap();
            assert_eq!(src_res.status_code, 200);
        });

        thread::sleep(Duration::from_millis(2000));

        handle_src.join().unwrap();
        device.tx_start_playing.send(()).unwrap();
        handle_start.join().unwrap();
        tx_stop_serv.send(()).unwrap();
    }

    #[test]
    fn test_event_failures() {
        let TestServer {
            device,
            port,
            tx_stop_serv,
        } = TestServer::new(7910, "test_event_failures".to_string());

        device.tx_start_playing.send(()).unwrap();

        let cut_first = minreq::post(format!("http://127.0.0.1:{port}/cut"))
            .with_header("Content-Type", "application/json")
            .with_body(
                serde_json::to_string(&EditRequestData {
                    edit_type: "cut".into(),
                    edit_duration_frames: None,
                    wipe_num: None,
                    source_tape: Some("tape1".into()),
                    av_channels: AVChannels::default(),
                })
                .unwrap(),
            )
            .send()
            .unwrap();
        let start_before_src = minreq::post(format!("http://127.0.0.1:{port}/start"))
            .with_header("Content-Type", "application/json")
            .with_body(
                serde_json::to_string(&EditRequestData {
                    edit_type: "cut".into(),
                    edit_duration_frames: None,
                    wipe_num: None,
                    source_tape: None,
                    av_channels: AVChannels::default(),
                })
                .unwrap(),
            )
            .send()
            .unwrap();
        minreq::post(format!("http://127.0.0.1:{port}/select-src"))
            .with_header("Content-Type", "application/json")
            .with_body(
                serde_json::to_string(&SourceTapeRequestData {
                    source_tape: "tape1".into(),
                })
                .unwrap(),
            )
            .send()
            .unwrap();
        let end_before_start = minreq::post(format!("http://127.0.0.1:{port}/end"))
            .with_header("Content-Type", "application/json")
            .with_body(
                serde_json::to_string(&EditRequestData {
                    edit_type: "cut".into(),
                    edit_duration_frames: None,
                    wipe_num: None,
                    source_tape: Some("tape1".into()),
                    av_channels: AVChannels::default(),
                })
                .unwrap(),
            )
            .send()
            .unwrap();
        let cut_before_start = minreq::post(format!("http://127.0.0.1:{port}/cut"))
            .with_header("Content-Type", "application/json")
            .with_body(
                serde_json::to_string(&EditRequestData {
                    edit_type: "cut".into(),
                    edit_duration_frames: None,
                    wipe_num: None,
                    source_tape: Some("tape1".into()),
                    av_channels: AVChannels::default(),
                })
                .unwrap(),
            )
            .send()
            .unwrap();

        assert_eq!(cut_first.status_code, 404);
        assert_eq!(start_before_src.status_code, 500);
        assert_eq!(end_before_start.status_code, 404);
        assert_eq!(cut_before_start.status_code, 404);

        tx_stop_serv.send(()).unwrap();
    }

    #[test]
    fn test_event_repeats() {
        let TestServer {
            device,
            port,
            tx_stop_serv,
        } = TestServer::new(7915, "test_event_repeats".to_string());

        minreq::post(format!("http://127.0.0.1:{port}/select-src"))
            .with_header("Content-Type", "application/json")
            .with_body(
                serde_json::to_string(&SourceTapeRequestData {
                    source_tape: "tape1".into(),
                })
                .unwrap(),
            )
            .send()
            .unwrap();
        let handle_1 = thread::spawn(move || {
            let start_1 = minreq::post(format!("http://127.0.0.1:{port}/start"))
                .with_header("Content-Type", "application/json")
                .with_body(
                    serde_json::to_string(&EditRequestData {
                        edit_type: "cut".into(),
                        edit_duration_frames: None,
                        wipe_num: None,
                        source_tape: None,
                        av_channels: AVChannels::default(),
                    })
                    .unwrap(),
                )
                .send()
                .unwrap();
            assert_eq!(start_1.status_code, 200);
        });
        thread::sleep(Duration::from_millis(150));
        let handle_2 = thread::spawn(move || {
            let start_2 = minreq::post(format!("http://127.0.0.1:{port}/start"))
                .with_header("Content-Type", "application/json")
                .with_body(
                    serde_json::to_string(&EditRequestData {
                        edit_type: "cut".into(),
                        edit_duration_frames: None,
                        wipe_num: None,
                        source_tape: None,
                        av_channels: AVChannels::default(),
                    })
                    .unwrap(),
                )
                .send()
                .unwrap();
            assert_eq!(start_2.status_code, 404);
        });
        thread::sleep(Duration::from_millis(150));
        let handle_3 = thread::spawn(move || {
            let start_3 = minreq::post(format!("http://127.0.0.1:{port}/start"))
                .with_header("Content-Type", "application/json")
                .with_body(
                    serde_json::to_string(&EditRequestData {
                        edit_type: "cut".into(),
                        edit_duration_frames: None,
                        wipe_num: None,
                        source_tape: None,
                        av_channels: AVChannels::default(),
                    })
                    .unwrap(),
                )
                .send()
                .unwrap();
            assert_eq!(start_3.status_code, 404);
            device.tx_start_playing.send(()).unwrap();
        });

        handle_3.join().unwrap();
        handle_2.join().unwrap();
        handle_1.join().unwrap();
        tx_stop_serv.send(()).unwrap();
    }
}
