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

impl TestServer {
    fn new(port: usize, file_name: String) -> Self {
        let opt = test_opt(port, file_name);
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

    for _ in 0..10 {
        thread::spawn(move || {
            let try_cut = minreq::post(format!("http://127.0.0.1:{port}/log"))
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
            assert_eq!(try_cut.status_code, 404);
        });
    }

    for _ in 0..3 {
        thread::spawn(move || {
            let try_end = minreq::post(format!("http://127.0.0.1:{port}/end"))
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
            assert_eq!(try_end.status_code, 404);
        });
    }

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

    // thread::sleep(Duration::from_millis(2000));

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
    });
    thread::sleep(Duration::from_millis(10));
    let handle_4 = thread::spawn(move || {
        let start_4 = minreq::post(format!("http://127.0.0.1:{port}/start"))
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
        assert_eq!(start_4.status_code, 404);
    });
    thread::sleep(Duration::from_millis(10));
    let handle_5 = thread::spawn(move || {
        let start_5 = minreq::post(format!("http://127.0.0.1:{port}/start"))
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
        assert_eq!(start_5.status_code, 404);
    });
    thread::sleep(Duration::from_millis(10));
    let handle_6 = thread::spawn(move || {
        let log_1 = minreq::post(format!("http://127.0.0.1:{port}/log"))
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
        assert_eq!(log_1.status_code, 404);
    });
    thread::sleep(Duration::from_millis(10));

    for _ in 0..20 {
        thread::spawn(move || {
            let log = minreq::post(format!("http://127.0.0.1:{port}/log"))
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
            assert_eq!(log.status_code, 404);
        });
    }

    thread::sleep(Duration::from_millis(50));
    let handle_7 = thread::spawn(move || {
        let start_7 = minreq::post(format!("http://127.0.0.1:{port}/start"))
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
        assert_eq!(start_7.status_code, 404);
        device.tx_start_playing.send(()).unwrap();
    });

    handle_7.join().unwrap();
    handle_6.join().unwrap();
    handle_5.join().unwrap();
    handle_4.join().unwrap();
    handle_3.join().unwrap();
    handle_2.join().unwrap();
    handle_1.join().unwrap();
    tx_stop_serv.send(()).unwrap();
}
