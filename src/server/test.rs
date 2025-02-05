use eframe::egui::Context;
use parking_lot::Mutex;

use crate::{
    edl_writer::{AVChannels, Clip, Dissolve, Edit, Wipe},
    ltc_decoder::LTCListener,
    server::{
        EditRequestData, EdlRecordingState, ReqBody, ResBodyRecStatus, Server,
        SourceTapeRequestData,
    },
    state::Logger,
    test::{cpal_device::MockDevice, state::test_opt},
};
use std::{
    net::TcpListener,
    sync::{mpsc, Arc},
    thread,
    time::Duration,
};

struct TestServer {
    device: MockDevice,
    port: u16,
    tx_stop_serv: mpsc::Sender<()>,
}

fn get_available_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

impl TestServer {
    fn new(file_name: String) -> Self {
        let port = get_available_port();
        let opt = test_opt(port.into(), file_name);
        let device = opt.ltc_device.as_ref().unwrap().device.clone();

        Logger::init(&Context::default());

        let decode_handlers = LTCListener::new(opt.clone()).unwrap().listen().unwrap();
        let (tx_stop_serv, rx_stop_serv) = mpsc::channel::<()>();
        let (tx_serv_stopped, _rx_serv_stopped) = mpsc::channel::<()>();
        let rx_stop_serv = Arc::new(Mutex::new(rx_stop_serv));

        thread::spawn(move || {
            Server::new(&opt.port)
                .listen(rx_stop_serv, tx_serv_stopped, decode_handlers, opt)
                .unwrap();
        });
        wait_for_server(port);

        Self {
            device,
            port,
            tx_stop_serv,
        }
    }
}

fn wait_for_server(port: u16) {
    let addr = format!("127.0.0.1:{}", port);
    for _ in 0..100 {
        if std::net::TcpStream::connect(&addr).is_ok() {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    panic!("Server did not start on {}", addr);
}

fn wait_for_rdy(port: u16) {
    let check_rdy = || {
        let res = minreq::get(format!("http://127.0.0.1:{port}/edl-recording-state"))
            .with_header("Content-Type", "application/json")
            .send()
            .unwrap()
            .json::<ResBodyRecStatus>()
            .unwrap();
        res.recording_status == EdlRecordingState::Started
    };
    while !check_rdy() {
        thread::sleep(Duration::from_millis(100));
    }
}

trait JsonData {
    fn rec_state(&self) -> EdlRecordingState;
    fn edit(&self) -> Option<Edit>;
    fn cut(&self) -> Clip;
    fn dissolve(&self) -> Dissolve;
    fn wipe(&self) -> Wipe;
}

impl JsonData for minreq::Response {
    fn rec_state(&self) -> EdlRecordingState {
        self.json::<ResBodyRecStatus>().unwrap().recording_status
    }

    fn edit(&self) -> Option<Edit> {
        self.json::<ResBodyRecStatus>().unwrap().edit
    }

    fn cut(&self) -> Clip {
        match self.edit().expect("Expected edit") {
            Edit::Cut(clip) => clip,
            t @ _ => panic!("Expected Clip, got {:?}", t),
        }
    }

    fn dissolve(&self) -> Dissolve {
        match self.edit().expect("Expected edit") {
            Edit::Dissolve(dis) => dis,
            t @ _ => panic!("Expected Dissolve, got {:?}", t),
        }
    }

    fn wipe(&self) -> Wipe {
        match self.edit().expect("Expected edit") {
            Edit::Wipe(wipe) => wipe,
            t @ _ => panic!("Expected Wipe, got {:?}", t),
        }
    }
}

fn serde_edit(edit: EditRequestData) -> String {
    serde_json::to_value(&ReqBody::Edit(edit))
        .unwrap()
        .to_string()
}

fn serde_src(src: SourceTapeRequestData) -> String {
    serde_json::to_value(&ReqBody::Source(src))
        .unwrap()
        .to_string()
}

// Responses from edits events will always be from the previous call, as and edit is built from an in and out point
#[test]
fn test_basic_edit_events() {
    let TestServer {
        device,
        port,
        tx_stop_serv,
    } = TestServer::new("test_basic_edit_events".to_string());

    device.tx_start_playing.send(()).unwrap();

    let start_res = minreq::post(format!("http://127.0.0.1:{port}/start"))
        .with_header("Content-Type", "application/json")
        .with_body(serde_src(SourceTapeRequestData {
            source_tape: Some("tape1".into()),
            av_channels: Some(AVChannels::new(false, 0)),
        }))
        .send()
        .unwrap();
    assert_eq!(start_res.status_code, 200);
    assert_eq!(start_res.rec_state(), EdlRecordingState::Started);
    assert!(start_res.edit().is_none());

    wait_for_rdy(port);

    let cut_1_res = minreq::post(format!("http://127.0.0.1:{port}/log"))
        .with_header("Content-Type", "application/json")
        .with_body(serde_edit(EditRequestData {
            edit_type: "cut".into(),
            edit_duration_frames: None,
            wipe_num: None,
            source_tape: Some("tape2".into()),
            av_channels: Some(AVChannels::default()),
        }))
        .send()
        .unwrap();
    assert_eq!(cut_1_res.status_code, 200);
    assert_eq!(cut_1_res.rec_state(), EdlRecordingState::Started);
    assert_eq!(cut_1_res.cut().source_tape, "tape1".to_string());
    assert_eq!(cut_1_res.cut().av_channels, AVChannels::new(true, 2));

    let cut_2_res = minreq::post(format!("http://127.0.0.1:{port}/log"))
        .with_header("Content-Type", "application/json")
        .with_body(serde_edit(EditRequestData {
            edit_type: "cut".into(),
            edit_duration_frames: Some(1), // ignored
            wipe_num: Some(1),             // ignored
            source_tape: Some("tape1".into()),
            av_channels: Some(AVChannels::default()),
        }))
        .send()
        .unwrap();
    assert_eq!(cut_2_res.status_code, 200);
    assert_eq!(cut_2_res.rec_state(), EdlRecordingState::Started);
    assert_eq!(cut_2_res.cut().source_tape, "tape2".to_string());
    assert_eq!(cut_2_res.cut().av_channels, AVChannels::new(true, 2));

    let wipe_1_res = minreq::post(format!("http://127.0.0.1:{port}/log"))
        .with_header("Content-Type", "application/json")
        .with_body(serde_edit(EditRequestData {
            edit_type: "wipe".into(),
            edit_duration_frames: Some(20),
            wipe_num: None,
            source_tape: Some("tape2".into()),
            av_channels: Some(AVChannels::new(false, 3)),
        }))
        .send()
        .unwrap();
    assert_eq!(wipe_1_res.status_code, 200);
    assert_eq!(wipe_1_res.rec_state(), EdlRecordingState::Started);
    assert_eq!(wipe_1_res.wipe().to.source_tape, "tape1".to_string());
    assert_eq!(wipe_1_res.wipe().to.av_channels, AVChannels::new(false, 3));

    let dis_1_res = minreq::post(format!("http://127.0.0.1:{port}/log"))
        .with_header("Content-Type", "application/json")
        .with_body(serde_edit(EditRequestData {
            edit_type: "dissolve".into(),
            edit_duration_frames: Some(10),
            wipe_num: None,
            source_tape: Some("tape1".into()),
            av_channels: Some(AVChannels::default()),
        }))
        .send()
        .unwrap();
    assert_eq!(dis_1_res.status_code, 200);
    assert_eq!(dis_1_res.rec_state(), EdlRecordingState::Started);
    assert_eq!(dis_1_res.dissolve().from.source_tape, "tape1".to_string());
    assert_eq!(dis_1_res.dissolve().to.source_tape, "tape2".to_string());
    assert_eq!(
        dis_1_res.dissolve().to.av_channels,
        AVChannels::new(true, 2)
    );

    let end_res = minreq::post(format!("http://127.0.0.1:{port}/end"))
        .with_header("Content-Type", "application/json")
        .with_body(serde_edit(EditRequestData {
            edit_type: "cut".into(),
            edit_duration_frames: None,
            wipe_num: None,
            source_tape: Some("tape2".into()),
            av_channels: Some(AVChannels::new(true, 4)),
        }))
        .send()
        .unwrap();
    assert_eq!(end_res.status_code, 200);
    assert_eq!(end_res.rec_state(), EdlRecordingState::Stopped);
    assert_eq!(end_res.cut().source_tape, "tape1".to_string());
    assert_eq!(end_res.cut().av_channels, AVChannels::new(true, 4));

    tx_stop_serv.send(()).unwrap();
}

#[test]
fn test_event_failures() {
    let TestServer {
        device,
        port,
        tx_stop_serv,
    } = TestServer::new("test_event_failures".to_string());

    device.tx_start_playing.send(()).unwrap();

    let invalid_post_url = minreq::post(format!("http://127.0.0.1:{port}/log-edit")) //invalid url
        .with_header("Content-Type", "application/json")
        .with_body(serde_edit(EditRequestData {
            edit_type: "cut".into(),
            edit_duration_frames: None,
            wipe_num: None,
            source_tape: Some("tape1".into()),
            av_channels: Some(AVChannels::default()),
        }))
        .send()
        .unwrap();
    assert_eq!(invalid_post_url.status_code, 404);

    let invalid_get_url = minreq::get(format!("http://127.0.0.1:{port}/select-src-tape")) //invalud url
        .with_header("Content-Type", "application/json")
        .send()
        .unwrap();
    assert_eq!(invalid_get_url.status_code, 404);

    let invalid_edit_type = minreq::post(format!("http://127.0.0.1:{port}/start"))
        .with_header("Content-Type", "application/json")
        .with_body(serde_src(SourceTapeRequestData {
            source_tape: None,
            av_channels: Some(AVChannels::default()),
        }))
        .send()
        .unwrap();
    assert_eq!(invalid_edit_type.status_code, 500);

    let no_body = minreq::post(format!("http://127.0.0.1:{port}/start"))
        .with_header("Content-Type", "application/json")
        .send()
        .unwrap();
    assert_eq!(no_body.status_code, 500);

    tx_stop_serv.send(()).unwrap();
}

#[test]
fn test_wait_for_ltc_on_start() {
    let TestServer {
        device,
        port,
        tx_stop_serv,
    } = TestServer::new("test_wait_for_ltc_on_start".to_string());

    let start_res = minreq::post(format!("http://127.0.0.1:{port}/start"))
        .with_header("Content-Type", "application/json")
        .with_body(serde_src(SourceTapeRequestData {
            source_tape: Some("tape1".into()),
            av_channels: Some(AVChannels::default()),
        }))
        .send()
        .unwrap();
    assert_eq!(start_res.status_code, 202);
    assert_eq!(start_res.rec_state(), EdlRecordingState::Waiting);

    let cut_1_res = minreq::post(format!("http://127.0.0.1:{port}/log"))
        .with_header("Content-Type", "application/json")
        .with_body(serde_edit(EditRequestData {
            edit_type: "cut".into(),
            edit_duration_frames: None,
            wipe_num: None,
            source_tape: Some("tape2".into()),
            av_channels: Some(AVChannels::default()),
        }))
        .send()
        .unwrap();
    assert_eq!(cut_1_res.status_code, 202);
    assert_eq!(cut_1_res.rec_state(), EdlRecordingState::Waiting);

    device.tx_start_playing.send(()).unwrap();
    wait_for_rdy(port);

    let cut_2_res = minreq::post(format!("http://127.0.0.1:{port}/log"))
        .with_header("Content-Type", "application/json")
        .with_body(serde_edit(EditRequestData {
            edit_type: "cut".into(),
            edit_duration_frames: None,
            wipe_num: None,
            source_tape: Some("tape2".into()),
            av_channels: Some(AVChannels::default()),
        }))
        .send()
        .unwrap();
    assert_eq!(cut_2_res.status_code, 200);
    assert_eq!(cut_2_res.rec_state(), EdlRecordingState::Started);

    tx_stop_serv.send(()).unwrap();
}

#[test]
fn test_edit_events_with_preselected_src() {
    let TestServer {
        device,
        port,
        tx_stop_serv,
    } = TestServer::new("test_edit_events_with_preselected_src".to_string());

    device.tx_start_playing.send(()).unwrap();

    let src_res = minreq::post(format!("http://127.0.0.1:{port}/select-src"))
        .with_header("Content-Type", "application/json")
        .with_body(serde_src(SourceTapeRequestData {
            source_tape: Some("tape1".into()),
            av_channels: None,
        }))
        .send()
        .unwrap();
    assert_eq!(src_res.status_code, 200);

    let start_res = minreq::post(format!("http://127.0.0.1:{port}/start"))
        .with_header("Content-Type", "application/json")
        .with_body(serde_src(SourceTapeRequestData {
            source_tape: None,
            av_channels: Some(AVChannels::default()),
        }))
        .send()
        .unwrap();
    assert_eq!(start_res.status_code, 200);
    assert_eq!(start_res.rec_state(), EdlRecordingState::Started);

    wait_for_rdy(port);

    let cut_1_res = minreq::post(format!("http://127.0.0.1:{port}/log"))
        .with_header("Content-Type", "application/json")
        .with_body(serde_edit(EditRequestData {
            edit_type: "cut".into(),
            edit_duration_frames: None,
            wipe_num: None,
            source_tape: Some("tape2".into()),
            av_channels: Some(AVChannels::default()),
        }))
        .send()
        .unwrap();
    assert_eq!(cut_1_res.status_code, 200);
    assert_eq!(cut_1_res.rec_state(), EdlRecordingState::Started);

    let cut_2_res = minreq::post(format!("http://127.0.0.1:{port}/log"))
        .with_header("Content-Type", "application/json")
        .with_body(serde_edit(EditRequestData {
            edit_type: "cut".into(),
            edit_duration_frames: None,
            wipe_num: None,
            source_tape: None,
            av_channels: Some(AVChannels::default()),
        }))
        .send()
        .unwrap();
    assert_eq!(cut_2_res.status_code, 200);
    assert_eq!(cut_2_res.rec_state(), EdlRecordingState::Started);

    tx_stop_serv.send(()).unwrap();
}

#[test]
fn test_select_src_while_waiting() {
    let name = "test_select_src_while_waiting";
    let TestServer {
        device,
        port,
        tx_stop_serv,
    } = TestServer::new(name.to_string());

    let start_res = minreq::post(format!("http://127.0.0.1:{port}/start"))
        .with_header("Content-Type", "application/json")
        .send()
        .unwrap();
    assert_eq!(start_res.status_code, 202);
    assert_eq!(start_res.rec_state(), EdlRecordingState::Waiting);

    let cut_w_tape_res = minreq::post(format!("http://127.0.0.1:{port}/log"))
        .with_header("Content-Type", "application/json")
        .with_body(serde_edit(EditRequestData {
            edit_type: "cut".into(),
            edit_duration_frames: None,
            wipe_num: None,
            source_tape: Some("tape2".into()),
            av_channels: Some(AVChannels::default()),
        }))
        .send()
        .unwrap();
    assert_eq!(cut_w_tape_res.status_code, 202);
    assert_eq!(cut_w_tape_res.rec_state(), EdlRecordingState::Waiting);

    // end while waiting and then restart
    let end_res = minreq::post(format!("http://127.0.0.1:{port}/end"))
        .with_header("Content-Type", "application/json")
        .with_body(serde_edit(EditRequestData {
            edit_type: "cut".into(),
            edit_duration_frames: None,
            wipe_num: None,
            source_tape: None,
            av_channels: Some(AVChannels::default()),
        }))
        .send()
        .unwrap();
    assert_eq!(end_res.status_code, 200);
    assert_eq!(end_res.rec_state(), EdlRecordingState::Stopped);

    let start_res = minreq::post(format!("http://127.0.0.1:{port}/start"))
        .with_header("Content-Type", "application/json")
        .with_body(serde_src(SourceTapeRequestData {
            source_tape: None,
            av_channels: Some(AVChannels::default()),
        }))
        .send()
        .unwrap();
    assert_eq!(start_res.status_code, 202);
    assert_eq!(start_res.rec_state(), EdlRecordingState::Waiting);

    for _ in 0..5 {
        let try_cut = minreq::post(format!("http://127.0.0.1:{port}/log"))
            .with_header("Content-Type", "application/json")
            .with_body(serde_edit(EditRequestData {
                edit_type: "cut".into(),
                edit_duration_frames: None,
                wipe_num: None,
                source_tape: None,
                av_channels: Some(AVChannels::default()),
            }))
            .send()
            .unwrap();
        assert_eq!(try_cut.status_code, 202);
        assert_eq!(try_cut.rec_state(), EdlRecordingState::Waiting);
    }

    let src_res = minreq::post(format!("http://127.0.0.1:{port}/select-src"))
        .with_header("Content-Type", "application/json")
        .with_body(serde_src(SourceTapeRequestData {
            source_tape: Some("tape1".into()),
            av_channels: Some(AVChannels::default()),
        }))
        .send()
        .unwrap();
    assert_eq!(src_res.status_code, 200);

    device.tx_start_playing.send(()).unwrap();
    wait_for_rdy(port);

    for i in 0..5 {
        println!("{name} - try_cut:{} start", i);
        let try_cut = minreq::post(format!("http://127.0.0.1:{port}/log"))
            .with_header("Content-Type", "application/json")
            .with_body(serde_edit(EditRequestData {
                edit_type: "cut".into(),
                edit_duration_frames: None,
                wipe_num: None,
                source_tape: None,
                av_channels: Some(AVChannels::default()),
            }))
            .send()
            .unwrap();
        assert_eq!(try_cut.status_code, 200);
        assert_eq!(try_cut.rec_state(), EdlRecordingState::Started);
    }

    let src_res = minreq::post(format!("http://127.0.0.1:{port}/select-src"))
        .with_header("Content-Type", "application/json")
        .with_body(serde_src(SourceTapeRequestData {
            source_tape: Some("tape2".into()),
            av_channels: None,
        }))
        .send()
        .unwrap();
    assert_eq!(src_res.status_code, 200);

    for i in 0..5 {
        println!("{name} - try_cut:{} start", i);
        let try_cut = minreq::post(format!("http://127.0.0.1:{port}/log"))
            .with_header("Content-Type", "application/json")
            .with_body(serde_edit(EditRequestData {
                edit_type: "cut".into(),
                edit_duration_frames: None,
                wipe_num: None,
                source_tape: None,
                av_channels: Some(AVChannels::default()),
            }))
            .send()
            .unwrap();
        assert_eq!(try_cut.status_code, 200);
        assert_eq!(try_cut.rec_state(), EdlRecordingState::Started);
    }

    let try_end = minreq::post(format!("http://127.0.0.1:{port}/end"))
        .with_header("Content-Type", "application/json")
        .with_body(serde_edit(EditRequestData {
            edit_type: "cut".into(),
            edit_duration_frames: None,
            wipe_num: None,
            source_tape: None,
            av_channels: Some(AVChannels::default()),
        }))
        .send()
        .unwrap();
    assert_eq!(try_end.status_code, 200);
    assert_eq!(try_end.rec_state(), EdlRecordingState::Stopped);

    tx_stop_serv.send(()).unwrap();
}

#[test]
fn test_event_non_ready() {
    let TestServer {
        device,
        port,
        tx_stop_serv,
    } = TestServer::new("test_event_non_ready".to_string());

    device.tx_start_playing.send(()).unwrap();

    let cut_first = minreq::post(format!("http://127.0.0.1:{port}/log"))
        .with_header("Content-Type", "application/json")
        .with_body(serde_edit(EditRequestData {
            edit_type: "cut".into(),
            edit_duration_frames: None,
            wipe_num: None,
            source_tape: Some("tape1".into()),
            av_channels: Some(AVChannels::default()),
        }))
        .send()
        .unwrap();
    assert_eq!(cut_first.status_code, 202);
    assert_eq!(cut_first.rec_state(), EdlRecordingState::Stopped);

    let start_before_src = minreq::post(format!("http://127.0.0.1:{port}/start"))
        .with_header("Content-Type", "application/json")
        .with_body(serde_src(SourceTapeRequestData {
            source_tape: None,
            av_channels: Some(AVChannels::default()),
        }))
        .send()
        .unwrap();
    assert_eq!(start_before_src.status_code, 500); // No source set

    minreq::post(format!("http://127.0.0.1:{port}/select-src"))
        .with_header("Content-Type", "application/json")
        .with_body(serde_src(SourceTapeRequestData {
            source_tape: Some("tape1".into()),
            av_channels: None,
        }))
        .send()
        .unwrap();

    let end_before_start = minreq::post(format!("http://127.0.0.1:{port}/end"))
        .with_header("Content-Type", "application/json")
        .with_body(serde_edit(EditRequestData {
            edit_type: "cut".into(),
            edit_duration_frames: None,
            wipe_num: None,
            source_tape: Some("tape1".into()),
            av_channels: Some(AVChannels::default()),
        }))
        .send()
        .unwrap();
    assert_eq!(end_before_start.status_code, 202);
    assert_eq!(end_before_start.rec_state(), EdlRecordingState::Stopped);

    let cut_before_start = minreq::post(format!("http://127.0.0.1:{port}/log"))
        .with_header("Content-Type", "application/json")
        .with_body(serde_edit(EditRequestData {
            edit_type: "cut".into(),
            edit_duration_frames: None,
            wipe_num: None,
            source_tape: Some("tape1".into()),
            av_channels: Some(AVChannels::default()),
        }))
        .send()
        .unwrap();
    assert_eq!(cut_before_start.status_code, 202);
    assert_eq!(cut_before_start.rec_state(), EdlRecordingState::Stopped);

    let start_after_src = minreq::post(format!("http://127.0.0.1:{port}/start"))
        .with_header("Content-Type", "application/json")
        .with_body(serde_src(SourceTapeRequestData {
            source_tape: None,
            av_channels: Some(AVChannels::default()),
        }))
        .send()
        .unwrap();
    assert_eq!(start_after_src.status_code, 200);
    assert_eq!(start_after_src.rec_state(), EdlRecordingState::Started);

    let cut_after_start = minreq::post(format!("http://127.0.0.1:{port}/log"))
        .with_header("Content-Type", "application/json")
        .with_body(serde_edit(EditRequestData {
            edit_type: "cut".into(),
            edit_duration_frames: None,
            wipe_num: None,
            source_tape: Some("tape1".into()),
            av_channels: Some(AVChannels::default()),
        }))
        .send()
        .unwrap();
    assert_eq!(cut_after_start.status_code, 200);
    assert_eq!(cut_after_start.rec_state(), EdlRecordingState::Started);

    tx_stop_serv.send(()).unwrap();
}

#[test]
fn test_event_repeats() {
    let name = "test_event_repeats";
    let TestServer {
        device,
        port,
        tx_stop_serv,
    } = TestServer::new(name.to_string());

    minreq::post(format!("http://127.0.0.1:{port}/select-src"))
        .with_header("Content-Type", "application/json")
        .with_body(serde_src(SourceTapeRequestData {
            source_tape: Some("tape1".into()),
            av_channels: None,
        }))
        .send()
        .unwrap();

    let start_1 = minreq::post(format!("http://127.0.0.1:{port}/start"))
        .with_header("Content-Type", "application/json")
        .with_body(serde_src(SourceTapeRequestData {
            source_tape: None,
            av_channels: Some(AVChannels::default()),
        }))
        .send()
        .unwrap();
    assert_eq!(start_1.status_code, 202);
    assert_eq!(start_1.rec_state(), EdlRecordingState::Waiting);

    let start_2 = minreq::post(format!("http://127.0.0.1:{port}/start"))
        .with_header("Content-Type", "application/json")
        .with_body(serde_src(SourceTapeRequestData {
            source_tape: None,
            av_channels: Some(AVChannels::default()),
        }))
        .send()
        .unwrap();
    assert_eq!(start_2.status_code, 202);
    assert_eq!(start_2.rec_state(), EdlRecordingState::Waiting);

    let start_3 = minreq::post(format!("http://127.0.0.1:{port}/start"))
        .with_header("Content-Type", "application/json")
        .with_body(serde_src(SourceTapeRequestData {
            source_tape: None,
            av_channels: Some(AVChannels::default()),
        }))
        .send()
        .unwrap();
    assert_eq!(start_3.status_code, 202);
    assert_eq!(start_3.rec_state(), EdlRecordingState::Waiting);

    for i in 0..20 {
        let log = minreq::post(format!("http://127.0.0.1:{port}/log"))
            .with_header("Content-Type", "application/json")
            .with_body(serde_edit(EditRequestData {
                edit_type: "cut".into(),
                edit_duration_frames: None,
                wipe_num: None,
                source_tape: Some(i.to_string()),
                av_channels: Some(AVChannels::default()),
            }))
            .send()
            .unwrap();
        assert_eq!(log.status_code, 202);
        assert_eq!(log.rec_state(), EdlRecordingState::Waiting);
    }

    device.tx_start_playing.send(()).unwrap();
    wait_for_rdy(port);

    for i in 0..5 {
        let log = minreq::post(format!("http://127.0.0.1:{port}/log"))
            .with_header("Content-Type", "application/json")
            .with_body(serde_edit(EditRequestData {
                edit_type: "cut".into(),
                edit_duration_frames: None,
                wipe_num: None,
                source_tape: Some(i.to_string()),
                av_channels: Some(AVChannels::default()),
            }))
            .send()
            .unwrap();
        assert_eq!(log.status_code, 200);
        assert_eq!(log.rec_state(), EdlRecordingState::Started);
    }

    tx_stop_serv.send(()).unwrap();
}
