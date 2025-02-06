use cpal::traits::DeviceTrait;
use eframe::egui::Context;
use parking_lot::Mutex;
use test_support::MockDevice;

use crate::{
    edl_writer::{AVChannels, Clip, Dissolve, Edit, Ntsc, Wipe},
    ltc_decoder::{config::LTCDevice, LTCListener},
    server::{EditRequestData, EdlRecordingState, ReqBody, ResBody, Server, SourceTapeRequestData},
    state::{Logger, Opt},
    utils::dirs::get_or_make_dir,
};
use std::{
    net::TcpListener,
    path::PathBuf,
    sync::{mpsc, Arc},
    thread,
    time::Duration,
};

pub struct MockServer {
    device: MockDevice,
    port: u16,
    tx_stop_serv: mpsc::Sender<()>,
}

impl MockServer {
    fn new(file_name: String) -> Self {
        Logger::init(&Context::default());

        let port = MockServer::get_available_port();
        let opt = MockServer::opt(port, file_name);
        let device = opt.ltc_device.as_ref().unwrap().device.clone();
        let decode_handlers = LTCListener::new(opt.clone()).unwrap().listen().unwrap();
        let (tx_stop_serv, rx_stop_serv) = mpsc::channel::<()>();
        let (tx_serv_stopped, _rx_serv_stopped) = mpsc::channel::<()>();
        let rx_stop_serv = Arc::new(Mutex::new(rx_stop_serv));

        thread::spawn(move || {
            Server::new(opt.port)
                .listen(rx_stop_serv, tx_serv_stopped, decode_handlers, opt)
                .unwrap();
        });

        Self {
            device,
            port,
            tx_stop_serv,
        }
    }

    fn get_available_port() -> u16 {
        TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port()
    }

    fn opt(port: u16, file_name: String) -> Opt {
        let device = MockDevice::default();
        let ltc_device = LTCDevice {
            config: device.default_output_config().unwrap(),
            device: device.clone(),
        };

        Opt {
            title: file_name,
            dir: get_or_make_dir(PathBuf::from("./test-output"))
                .unwrap_or_else(|_| PathBuf::from("./")),
            sample_rate: 44_100,
            fps: 30.0,
            ntsc: Ntsc::DropFrame,
            buffer_size: Some(device.clone().opt_config.buffer_size),
            input_channel: Some(device.clone().opt_config.input_channel),
            ltc_device: Some(ltc_device.clone()),
            ltc_devices: Some(vec![ltc_device.clone()]),
            ltc_host: Arc::new(cpal::default_host()),
            ltc_hosts: Arc::new(cpal::available_hosts()),
            port,
        }
    }

    fn server_ready(self) -> Self {
        let addr = format!("127.0.0.1:{}", self.port);
        for _ in 0..100 {
            if std::net::TcpStream::connect(&addr).is_ok() {
                return self;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        panic!("Server did not start on {}", addr);
    }
}

pub fn wait_rec_state_started(port: u16) {
    let check_rdy = || {
        let res = minreq::get(format!("http://127.0.0.1:{port}/edl-recording-state"))
            .with_header("Content-Type", "application/json")
            .send()
            .unwrap()
            .json::<ResBody>()
            .unwrap();
        res.recording_status == EdlRecordingState::Started
    };
    while !check_rdy() {
        thread::sleep(Duration::from_millis(100));
    }
}

trait JsonData {
    fn rec_state(&self) -> EdlRecordingState;
    fn has_edit_or_final_edits_body(&self) -> bool;
    fn edit(&self) -> Edit;
    fn final_edits(&self) -> Vec<Edit>;
}

impl JsonData for minreq::Response {
    fn rec_state(&self) -> EdlRecordingState {
        self.json::<ResBody>().unwrap().recording_status
    }

    fn has_edit_or_final_edits_body(&self) -> bool {
        let res = self.json::<ResBody>().unwrap();
        res.edit.is_some() || res.final_edits.is_some()
    }

    fn edit(&self) -> Edit {
        self.json::<ResBody>().unwrap().edit.expect("Expected edit")
    }

    fn final_edits(&self) -> Vec<Edit> {
        self.json::<ResBody>()
            .unwrap()
            .final_edits
            .expect("Expected final edits")
    }
}

trait AssessEditType {
    fn cut(&self) -> Clip;
    fn dissolve(&self) -> Dissolve;
    fn wipe(&self) -> Wipe;
}

impl AssessEditType for minreq::Response {
    fn cut(&self) -> Clip {
        match self.edit() {
            Edit::Cut(clip) => clip,
            Edit::Dissolve(_) => panic!("Expected Clip, Dissolve"),
            Edit::Wipe(_) => panic!("Expected Clip, got Wipe"),
        }
    }

    fn dissolve(&self) -> Dissolve {
        match self.edit() {
            Edit::Dissolve(dis) => dis,
            Edit::Wipe(_) => panic!("Expected Clip, got Wipe"),
            Edit::Cut(_) => panic!("Expected Clip, got Cut"),
        }
    }

    fn wipe(&self) -> Wipe {
        match self.edit() {
            Edit::Wipe(wipe) => wipe,
            Edit::Dissolve(_) => panic!("Expected Clip, Dissolve"),
            Edit::Cut(_) => panic!("Expected Clip, got Cut"),
        }
    }
}

impl AssessEditType for Edit {
    fn cut(&self) -> Clip {
        match self {
            Edit::Cut(clip) => clip.clone(),
            Edit::Dissolve(_) => panic!("Expected Clip, Dissolve"),
            Edit::Wipe(_) => panic!("Expected Clip, got Wipe"),
        }
    }

    fn dissolve(&self) -> Dissolve {
        match self {
            Edit::Dissolve(dis) => dis.clone(),
            Edit::Wipe(_) => panic!("Expected Clip, got Wipe"),
            Edit::Cut(_) => panic!("Expected Clip, got Cut"),
        }
    }

    fn wipe(&self) -> Wipe {
        match self {
            Edit::Wipe(wipe) => wipe.clone(),
            Edit::Dissolve(_) => panic!("Expected Clip, Dissolve"),
            Edit::Cut(_) => panic!("Expected Clip, got Cut"),
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

#[test]
fn edit_starts_ends_cut() {
    let MockServer {
        device,
        port,
        tx_stop_serv,
    } = MockServer::new("edit_starts_ends_cut".to_string()).server_ready();

    device.tx_start_playing.send(()).unwrap();

    let start_res = minreq::post(format!("http://127.0.0.1:{port}/start"))
        .with_header("Content-Type", "application/json")
        .with_body(serde_edit(EditRequestData {
            edit_type: "cut".into(),
            edit_duration_frames: None,
            wipe_num: None,
            source_tape: Some("tape1".into()),
            av_channels: Some(AVChannels::new(false, 1)),
        }))
        .send()
        .unwrap();
    assert_eq!(start_res.status_code, 200);
    assert_eq!(start_res.rec_state(), EdlRecordingState::Started);
    assert!(!start_res.has_edit_or_final_edits_body());

    wait_rec_state_started(port);

    let wipe_1_res = minreq::post(format!("http://127.0.0.1:{port}/log"))
        .with_header("Co-Type", "application/json")
        .with_body(serde_edit(EditRequestData {
            edit_type: "wipe".into(),
            edit_duration_frames: Some(15),
            wipe_num: None,
            source_tape: Some("tape2".into()),
            av_channels: Some(AVChannels::default()),
        }))
        .send()
        .unwrap();
    assert_eq!(wipe_1_res.status_code, 200);
    assert_eq!(wipe_1_res.rec_state(), EdlRecordingState::Started);
    assert_eq!(wipe_1_res.cut().source_tape, "tape1".to_string());
    assert_eq!(wipe_1_res.cut().av_channels, AVChannels::new(false, 1));

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
    assert_eq!(cut_2_res.wipe().from.source_tape, "tape1".to_string());
    assert_eq!(cut_2_res.wipe().from.av_channels, AVChannels::new(false, 1));
    assert_eq!(cut_2_res.wipe().to.source_tape, "tape2".to_string());
    assert_eq!(cut_2_res.wipe().to.av_channels, AVChannels::new(true, 2));

    let wipe_2_res = minreq::post(format!("http://127.0.0.1:{port}/log"))
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
    assert_eq!(wipe_2_res.status_code, 200);
    assert_eq!(wipe_2_res.rec_state(), EdlRecordingState::Started);
    assert_eq!(wipe_2_res.cut().source_tape, "tape1".to_string());
    assert_eq!(wipe_2_res.cut().av_channels, AVChannels::new(true, 2));

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
    assert_eq!(dis_1_res.rec_state(), EdlRecordingState::Started);
    assert_eq!(dis_1_res.status_code, 200);
    assert_eq!(dis_1_res.wipe().from.source_tape, "tape1".to_string());
    assert_eq!(dis_1_res.wipe().from.av_channels, AVChannels::new(true, 2));
    assert_eq!(dis_1_res.wipe().to.source_tape, "tape2".to_string());
    assert_eq!(dis_1_res.wipe().to.av_channels, AVChannels::new(false, 3));

    let cut_3_res = minreq::post(format!("http://127.0.0.1:{port}/log"))
        .with_header("Content-Type", "application/json")
        .with_body(serde_edit(EditRequestData {
            edit_type: "cut".into(),
            edit_duration_frames: None,
            wipe_num: None,
            source_tape: Some("tape3".into()),
            av_channels: Some(AVChannels::default()),
        }))
        .send()
        .unwrap();
    assert_eq!(cut_3_res.status_code, 200);
    assert_eq!(cut_3_res.rec_state(), EdlRecordingState::Started);
    assert_eq!(cut_3_res.dissolve().from.source_tape, "tape2".to_string());
    assert_eq!(cut_3_res.dissolve().to.source_tape, "tape1".to_string());
    assert_eq!(
        cut_3_res.dissolve().to.av_channels,
        AVChannels::new(true, 2)
    );

    let cut_4_res = minreq::post(format!("http://127.0.0.1:{port}/log"))
        .with_header("Content-Type", "application/json")
        .with_body(serde_edit(EditRequestData {
            edit_type: "cut".into(),
            edit_duration_frames: None,
            wipe_num: Some(1), // ignored
            source_tape: Some("tape1".into()),
            av_channels: Some(AVChannels::default()),
        }))
        .send()
        .unwrap();
    assert_eq!(cut_4_res.status_code, 200);
    assert_eq!(cut_4_res.rec_state(), EdlRecordingState::Started);
    assert_eq!(cut_4_res.cut().source_tape, "tape3".to_string());
    assert_eq!(cut_4_res.cut().av_channels, AVChannels::new(true, 2));

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
    assert_eq!(
        end_res.final_edits()[0].cut().source_tape,
        "tape1".to_string()
    );
    assert_eq!(
        end_res.final_edits()[0].cut().av_channels,
        AVChannels::new(true, 2)
    );
    assert!(end_res.final_edits().into_iter().nth(1).is_none());

    tx_stop_serv.send(()).unwrap();
}

#[test]
fn edit_starts_cut_ends_diss() {
    let MockServer {
        device,
        port,
        tx_stop_serv,
    } = MockServer::new("edit_starts_cut_ends_diss".to_string()).server_ready();

    device.tx_start_playing.send(()).unwrap();

    let start_res = minreq::post(format!("http://127.0.0.1:{port}/start"))
        .with_header("Content-Type", "application/json")
        .with_body(serde_edit(EditRequestData {
            edit_type: "cut".into(),
            edit_duration_frames: Some(40),
            wipe_num: None,
            source_tape: Some("tape1".into()),
            av_channels: Some(AVChannels::new(false, 1)),
        }))
        .send()
        .unwrap();
    assert_eq!(start_res.status_code, 200);
    assert_eq!(start_res.rec_state(), EdlRecordingState::Started);
    assert!(!start_res.has_edit_or_final_edits_body());

    wait_rec_state_started(port);

    let wipe_1_res = minreq::post(format!("http://127.0.0.1:{port}/log"))
        .with_header("Co-Type", "application/json")
        .with_body(serde_edit(EditRequestData {
            edit_type: "wipe".into(),
            edit_duration_frames: Some(15),
            wipe_num: None,
            source_tape: Some("tape2".into()),
            av_channels: Some(AVChannels::default()),
        }))
        .send()
        .unwrap();
    assert_eq!(wipe_1_res.status_code, 200);
    assert_eq!(wipe_1_res.rec_state(), EdlRecordingState::Started);
    assert_eq!(wipe_1_res.cut().source_tape, "tape1".to_string());
    assert_eq!(wipe_1_res.cut().av_channels, AVChannels::new(false, 1));

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
    assert_eq!(cut_2_res.wipe().from.source_tape, "tape1".to_string());
    assert_eq!(cut_2_res.wipe().from.av_channels, AVChannels::new(false, 1));
    assert_eq!(cut_2_res.wipe().to.source_tape, "tape2".to_string());
    assert_eq!(cut_2_res.wipe().to.av_channels, AVChannels::new(true, 2));

    let wipe_2_res = minreq::post(format!("http://127.0.0.1:{port}/log"))
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
    assert_eq!(wipe_2_res.status_code, 200);
    assert_eq!(wipe_2_res.rec_state(), EdlRecordingState::Started);
    assert_eq!(wipe_2_res.cut().source_tape, "tape1".to_string());
    assert_eq!(wipe_2_res.cut().av_channels, AVChannels::new(true, 2));

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
    assert_eq!(dis_1_res.rec_state(), EdlRecordingState::Started);
    assert_eq!(dis_1_res.status_code, 200);
    assert_eq!(dis_1_res.wipe().from.source_tape, "tape1".to_string());
    assert_eq!(dis_1_res.wipe().from.av_channels, AVChannels::new(true, 2));
    assert_eq!(dis_1_res.wipe().to.source_tape, "tape2".to_string());
    assert_eq!(dis_1_res.wipe().to.av_channels, AVChannels::new(false, 3));

    let cut_3_res = minreq::post(format!("http://127.0.0.1:{port}/log"))
        .with_header("Content-Type", "application/json")
        .with_body(serde_edit(EditRequestData {
            edit_type: "cut".into(),
            edit_duration_frames: Some(1), // ignored
            wipe_num: Some(1),             // ignored
            source_tape: Some("tape3".into()),
            av_channels: Some(AVChannels::default()),
        }))
        .send()
        .unwrap();
    assert_eq!(cut_3_res.status_code, 200);
    assert_eq!(cut_3_res.rec_state(), EdlRecordingState::Started);
    assert_eq!(cut_3_res.dissolve().from.source_tape, "tape2".to_string());
    assert_eq!(cut_3_res.dissolve().to.source_tape, "tape1".to_string());
    assert_eq!(
        cut_3_res.dissolve().to.av_channels,
        AVChannels::new(true, 2)
    );

    let cut_4_res = minreq::post(format!("http://127.0.0.1:{port}/log"))
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
    assert_eq!(cut_4_res.status_code, 200);
    assert_eq!(cut_4_res.rec_state(), EdlRecordingState::Started);
    assert_eq!(cut_4_res.cut().source_tape, "tape3".to_string());
    assert_eq!(cut_4_res.cut().av_channels, AVChannels::new(true, 2));

    let end_res = minreq::post(format!("http://127.0.0.1:{port}/end"))
        .with_header("Content-Type", "application/json")
        .with_body(serde_edit(EditRequestData {
            edit_type: "dissolve".into(),
            edit_duration_frames: Some(42),
            wipe_num: None,
            source_tape: Some("tape2".into()),
            av_channels: Some(AVChannels::new(true, 4)),
        }))
        .send()
        .unwrap();
    assert_eq!(end_res.status_code, 200);
    assert_eq!(end_res.rec_state(), EdlRecordingState::Stopped);
    assert_eq!(
        end_res.final_edits()[0].cut().source_tape,
        "tape1".to_string()
    );
    assert_eq!(
        end_res.final_edits()[0].cut().av_channels,
        AVChannels::new(true, 2)
    );
    assert_eq!(
        end_res.final_edits()[1].dissolve().from.source_tape,
        "tape1".to_string()
    );
    assert_eq!(
        end_res.final_edits()[1].dissolve().to.source_tape,
        "BL".to_string()
    );
    assert_eq!(
        end_res.final_edits()[1].dissolve().to.av_channels,
        AVChannels::new(true, 0)
    );
    assert!(end_res.final_edits().into_iter().nth(2).is_none());

    tx_stop_serv.send(()).unwrap();
}

#[test]
fn edit_starts_diss_ends_cut() {
    let MockServer {
        device,
        port,
        tx_stop_serv,
    } = MockServer::new("edit_starts_diss_ends_cut".to_string()).server_ready();

    device.tx_start_playing.send(()).unwrap();

    let start_res = minreq::post(format!("http://127.0.0.1:{port}/start"))
        .with_header("Content-Type", "application/json")
        .with_body(serde_edit(EditRequestData {
            edit_type: "dissolve".into(),
            edit_duration_frames: Some(40),
            wipe_num: None,
            source_tape: Some("tape1".into()),
            av_channels: Some(AVChannels::new(false, 1)),
        }))
        .send()
        .unwrap();
    assert_eq!(start_res.status_code, 200);
    assert_eq!(start_res.rec_state(), EdlRecordingState::Started);
    assert!(!start_res.has_edit_or_final_edits_body());

    wait_rec_state_started(port);

    let wipe_1_res = minreq::post(format!("http://127.0.0.1:{port}/log"))
        .with_header("Co-Type", "application/json")
        .with_body(serde_edit(EditRequestData {
            edit_type: "wipe".into(),
            edit_duration_frames: Some(15),
            wipe_num: None,
            source_tape: Some("tape2".into()),
            av_channels: Some(AVChannels::default()),
        }))
        .send()
        .unwrap();
    assert_eq!(wipe_1_res.status_code, 200);
    assert_eq!(wipe_1_res.rec_state(), EdlRecordingState::Started);
    assert_eq!(wipe_1_res.dissolve().from.source_tape, "BL".to_string());
    assert_eq!(
        wipe_1_res.dissolve().from.av_channels,
        AVChannels::new(true, 0)
    );
    assert_eq!(wipe_1_res.dissolve().to.source_tape, "tape1".to_string());
    assert_eq!(
        wipe_1_res.dissolve().to.av_channels,
        AVChannels::new(false, 1)
    );

    let cut_1_res = minreq::post(format!("http://127.0.0.1:{port}/log"))
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
    assert_eq!(cut_1_res.status_code, 200);
    assert_eq!(cut_1_res.rec_state(), EdlRecordingState::Started);
    assert_eq!(cut_1_res.wipe().from.source_tape, "tape1".to_string());
    assert_eq!(cut_1_res.wipe().from.av_channels, AVChannels::new(false, 1));
    assert_eq!(cut_1_res.wipe().to.source_tape, "tape2".to_string());
    assert_eq!(cut_1_res.wipe().to.av_channels, AVChannels::new(true, 2));

    let wipe_2_res = minreq::post(format!("http://127.0.0.1:{port}/log"))
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
    assert_eq!(wipe_2_res.status_code, 200);
    assert_eq!(wipe_2_res.rec_state(), EdlRecordingState::Started);
    assert_eq!(wipe_2_res.cut().source_tape, "tape1".to_string());
    assert_eq!(wipe_2_res.cut().av_channels, AVChannels::new(true, 2));

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
    assert_eq!(dis_1_res.rec_state(), EdlRecordingState::Started);
    assert_eq!(dis_1_res.status_code, 200);
    assert_eq!(dis_1_res.wipe().from.source_tape, "tape1".to_string());
    assert_eq!(dis_1_res.wipe().from.av_channels, AVChannels::new(true, 2));
    assert_eq!(dis_1_res.wipe().to.source_tape, "tape2".to_string());
    assert_eq!(dis_1_res.wipe().to.av_channels, AVChannels::new(false, 3));

    let end_res = minreq::post(format!("http://127.0.0.1:{port}/end"))
        .with_header("Content-Type", "application/json")
        .with_body(serde_edit(EditRequestData {
            edit_type: "cut".into(),
            edit_duration_frames: None,
            wipe_num: None,
            source_tape: None,
            av_channels: None,
        }))
        .send()
        .unwrap();
    assert_eq!(end_res.status_code, 200);
    assert_eq!(end_res.rec_state(), EdlRecordingState::Stopped);
    assert_eq!(
        end_res.final_edits()[0].dissolve().from.source_tape,
        "tape2".to_string()
    );
    assert_eq!(
        end_res.final_edits()[0].dissolve().from.av_channels,
        AVChannels::new(false, 3)
    );
    assert_eq!(
        end_res.final_edits()[0].dissolve().to.source_tape,
        "tape1".to_string()
    );
    assert_eq!(
        end_res.final_edits()[0].dissolve().to.av_channels,
        AVChannels::new(true, 2)
    );
    assert!(end_res.final_edits().into_iter().nth(1).is_none());

    tx_stop_serv.send(()).unwrap();
}

#[test]
fn event_failures() {
    let MockServer {
        device,
        port,
        tx_stop_serv,
    } = MockServer::new("event_failures".to_string()).server_ready();

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
        .with_body(serde_edit(EditRequestData {
            edit_type: "swipe".into(), //invalid
            edit_duration_frames: None,
            wipe_num: None,
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
fn wait_for_ltc_on_start() {
    let MockServer {
        device,
        port,
        tx_stop_serv,
    } = MockServer::new("wait_for_ltc_on_start".to_string()).server_ready();

    let start_res = minreq::post(format!("http://127.0.0.1:{port}/start"))
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
    wait_rec_state_started(port);

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
fn edit_events_with_preselected_src() {
    let MockServer {
        device,
        port,
        tx_stop_serv,
    } = MockServer::new("edit_events_with_preselected_src".to_string()).server_ready();

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
        .with_body(serde_edit(EditRequestData {
            edit_type: "cut".into(),
            edit_duration_frames: None,
            wipe_num: None,
            source_tape: None,
            av_channels: Some(AVChannels::default()),
        }))
        .send()
        .unwrap();
    assert_eq!(start_res.status_code, 200);
    assert_eq!(start_res.rec_state(), EdlRecordingState::Started);

    wait_rec_state_started(port);

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
fn edit_events_with_preselected_src_2() {
    let MockServer {
        device,
        port,
        tx_stop_serv,
    } = MockServer::new("edit_events_with_preselected_src_2".to_string()).server_ready();

    device.tx_start_playing.send(()).unwrap();

    let src_res = minreq::post(format!("http://127.0.0.1:{port}/select-src"))
        .with_header("Content-Type", "application/json")
        .with_body(serde_src(SourceTapeRequestData {
            source_tape: Some("tape1".into()),
            av_channels: Some(AVChannels::default()),
        }))
        .send()
        .unwrap();
    assert_eq!(src_res.status_code, 200);

    let start_res = minreq::post(format!("http://127.0.0.1:{port}/start"))
        .with_header("Content-Type", "application/json")
        .with_body(serde_edit(EditRequestData {
            edit_type: "cut".into(),
            edit_duration_frames: None,
            wipe_num: None,
            source_tape: None,
            av_channels: None,
        }))
        .send()
        .unwrap();
    assert_eq!(start_res.status_code, 200);
    assert_eq!(start_res.rec_state(), EdlRecordingState::Started);

    wait_rec_state_started(port);

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

    let cut_2_res = minreq::post(format!("http://127.0.0.1:{port}/end"))
        .with_header("Content-Type", "application/json")
        .with_body(serde_edit(EditRequestData {
            edit_type: "cut".into(),
            edit_duration_frames: None,
            wipe_num: None,
            source_tape: None,
            av_channels: None,
        }))
        .send()
        .unwrap();
    assert_eq!(cut_2_res.status_code, 200);
    assert_eq!(cut_2_res.rec_state(), EdlRecordingState::Stopped);

    tx_stop_serv.send(()).unwrap();
}

#[test]
fn select_src_while_waiting() {
    let name = "select_src_while_waiting";
    let MockServer {
        device,
        port,
        tx_stop_serv,
    } = MockServer::new(name.to_string()).server_ready();

    let start_res = minreq::post(format!("http://127.0.0.1:{port}/start"))
        .with_header("Content-Type", "application/json")
        .with_body(serde_edit(EditRequestData {
            edit_type: "cut".into(),
            edit_duration_frames: None,
            wipe_num: None,
            source_tape: None,
            av_channels: None,
        }))
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
        .with_body(serde_edit(EditRequestData {
            edit_type: "cut".into(),
            edit_duration_frames: None,
            wipe_num: None,
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
    wait_rec_state_started(port);

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
fn event_non_ready() {
    let MockServer {
        device,
        port,
        tx_stop_serv,
    } = MockServer::new("event_non_ready".to_string()).server_ready();

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
        .with_body(serde_edit(EditRequestData {
            edit_type: "cut".into(),
            edit_duration_frames: None,
            wipe_num: None,
            source_tape: None,
            av_channels: None,
        }))
        .send()
        .unwrap();
    assert_eq!(start_before_src.status_code, 200);

    minreq::post(format!("http://127.0.0.1:{port}/select-src"))
        .with_header("Content-Type", "application/json")
        .with_body(serde_src(SourceTapeRequestData {
            source_tape: Some("tape1".into()),
            av_channels: Some(AVChannels::default()),
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
    assert_eq!(end_before_start.status_code, 200);
    assert_eq!(end_before_start.rec_state(), EdlRecordingState::Stopped);
    assert_eq!(
        end_before_start.final_edits()[0].cut().source_tape,
        "BL".to_string()
    );

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
        .with_body(serde_edit(EditRequestData {
            edit_type: "cut".into(),
            edit_duration_frames: None,
            wipe_num: None,
            source_tape: None,
            av_channels: None,
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
fn event_repeats() {
    let name = "event_repeats";
    let MockServer {
        device,
        port,
        tx_stop_serv,
    } = MockServer::new(name.to_string()).server_ready();

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
        .with_body(serde_edit(EditRequestData {
            edit_type: "cut".into(),
            edit_duration_frames: None,
            wipe_num: None,
            source_tape: None,
            av_channels: Some(AVChannels::default()),
        }))
        .send()
        .unwrap();
    assert_eq!(start_1.status_code, 202);
    assert_eq!(start_1.rec_state(), EdlRecordingState::Waiting);

    let start_2 = minreq::post(format!("http://127.0.0.1:{port}/start"))
        .with_header("Content-Type", "application/json")
        .with_body(serde_edit(EditRequestData {
            edit_type: "cut".into(),
            edit_duration_frames: None,
            wipe_num: None,
            source_tape: None,
            av_channels: None,
        }))
        .send()
        .unwrap();
    assert_eq!(start_2.status_code, 202);
    assert_eq!(start_2.rec_state(), EdlRecordingState::Waiting);

    let start_3 = minreq::post(format!("http://127.0.0.1:{port}/start"))
        .with_header("Content-Type", "application/json")
        .with_body(serde_edit(EditRequestData {
            edit_type: "cut".into(),
            edit_duration_frames: None,
            wipe_num: None,
            source_tape: None,
            av_channels: None,
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
    wait_rec_state_started(port);

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
