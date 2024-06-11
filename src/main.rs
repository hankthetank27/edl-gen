use anyhow::{anyhow, Error};
use cpal::traits::DeviceTrait;
use edl_gen::ltc_decode::LTCListener;
use edl_gen::server::Server;
use edl_gen::single_val_channel;
use edl_gen::Logger;
use edl_gen::{edl, Opt};
use egui::mutex::Mutex;
use egui::Ui;
use ltc::LTCFrame;
use std::io::prelude::*;
use std::net::TcpStream;
use std::sync::{mpsc, Arc};
use std::thread;
use std::thread::JoinHandle;
use std::usize;

fn main() -> Result<(), Error> {
    Logger::init()?;

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([480.0, 500.0]),
        ..Default::default()
    };

    eframe::run_native(
        "EzEDL",
        options,
        Box::new(|_cc| {
            let app = AppGui::new();
            Box::new(app)
        }),
    )
    .map_err(|e| anyhow!("Could not initate UI: {}", e))
}

struct AppGui {
    opt: Opt,
    rx_stop_serv: Arc<Mutex<mpsc::Receiver<()>>>,
    tx_stop_serv: mpsc::Sender<()>,
    tx_serv_stopped: mpsc::Sender<()>,
    rx_serv_stopped: mpsc::Receiver<()>,
    tx_decode_ltc: Option<single_val_channel::Sender<LTCFrame>>,
    server_handle: Option<JoinHandle<Result<(), Error>>>,
}

impl AppGui {
    fn new() -> Self {
        let (tx_stop_serv, rx_stop_serv) = mpsc::channel::<()>();
        let (tx_serv_stopped, rx_serv_stopped) = mpsc::channel::<()>();
        AppGui {
            opt: Opt::default(),
            server_handle: None,
            rx_stop_serv: Arc::new(Mutex::new(rx_stop_serv)),
            tx_decode_ltc: None,
            tx_stop_serv,
            tx_serv_stopped,
            rx_serv_stopped,
        }
    }

    fn spawn_server(&mut self) {
        let decode_handlers = match LTCListener::new(self.opt.clone()) {
            Ok(listener) => listener.listen(),
            Err(e) => {
                log::error!("{}", e);
                return;
            }
        };
        let opt = self.opt.clone();
        let rx_stop_serv = Arc::clone(&self.rx_stop_serv);
        let tx_serv_stopped = self.tx_serv_stopped.clone();
        self.tx_decode_ltc = Some(decode_handlers.frame_sender.clone());
        self.server_handle = Some(thread::spawn(move || {
            Server::new(opt).listen(rx_stop_serv, tx_serv_stopped, decode_handlers)
        }));
    }

    fn kill_server(&mut self) -> Result<(), Error> {
        match self.server_handle.take() {
            Some(handle) => {
                self.tx_stop_serv.send(())?;
                if let Some(decode_ltc) = self.tx_decode_ltc.as_ref() {
                    decode_ltc.hangup();
                };
                // If the thread hasnt received the "shutdown" message, we will attempt to connect
                // to the server to advance to the next incoming stream in case its still waiting.
                // It is possible this process has already begun in which case the request will
                // fail. To handle this we supress the errors from attempting to connect and
                // instead check if we have received a message that the server has been shutdown to
                // indicate if the process has succeeded.
                if !handle.is_finished() {
                    let signal_shutdown = || -> Result<(), Error> {
                        let host = format!("127.0.0.1:{}", self.opt.port);
                        let mut stream = TcpStream::connect(&host)?;
                        let request = format!(
                            "GET /SIGKILL HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n",
                            host
                        );
                        stream.write_all(request.as_bytes())?;
                        let mut response = String::new();
                        stream.read_to_string(&mut response)?;
                        Ok(())
                    };
                    let _ = signal_shutdown();
                    if let Err(e) = self
                        .rx_serv_stopped
                        .recv_timeout(std::time::Duration::from_secs(3))
                    {
                        self.server_handle = Some(handle);
                        return Err(anyhow!("Could not kill server: {}", e));
                    }
                }
                handle
                    .join()
                    .expect("Could not kill server, error waiting for shutdown")?;
                Ok(())
            }
            None => Err(anyhow!("Expected server handle")),
        }
    }

    fn config_project_title(&mut self, ui: &mut Ui) {
        ui.add(egui::TextEdit::singleline(&mut self.opt.title).hint_text("Project Title"));
    }

    fn config_storage_dir(&mut self, ui: &mut Ui) {
        if ui.button("Storage Directory").clicked() {
            if let Some(path) = rfd::FileDialog::new().pick_folder() {
                self.opt.dir = path;
            }
        }
        ui.label(self.opt.dir.to_str().unwrap());
    }

    fn config_input_channel(&mut self, ui: &mut Ui) {
        match LTCListener::get_default_config() {
            Ok((config, device)) => {
                if let Ok(name) = device.name() {
                    ui.label(format!("Audio Device: {}", name));
                }

                egui::ComboBox::from_label("Input Channel")
                    .selected_text(format!("{}", self.opt.input_channel))
                    .show_ui(ui, |ui| {
                        for channel in (1..config.channels() + 1).collect::<Vec<u16>>().iter() {
                            let checked = *channel as usize == self.opt.input_channel;
                            if ui.selectable_label(checked, channel.to_string()).clicked() {
                                self.opt.input_channel = *channel as usize;
                            }
                        }
                    });
            }
            Err(e) => {
                log::error!("Could not configure audio device: {e}");
                ui.label("No Audio Device Found");
                egui::ComboBox::from_label("Input Channel")
                    .selected_text("No Device")
                    .show_ui(ui, |_| {});
            }
        };
    }

    fn config_buffer_size(&mut self, ui: &mut Ui) {
        match LTCListener::get_buffer_opts() {
            Ok(buf) => match buf {
                Some(opts) => {
                    if self.opt.buffer_size.is_none()
                        || !opts.contains(&self.opt.buffer_size.unwrap())
                    {
                        let mid = opts.get(opts.len() / 2);
                        self.opt.buffer_size = mid.copied();
                    };

                    egui::ComboBox::from_label("Buffer Size")
                        .selected_text(format!("{}", self.opt.buffer_size.unwrap_or(0)))
                        .show_ui(ui, |ui| {
                            for buffer in opts.iter() {
                                let checked = Some(*buffer) == self.opt.buffer_size;
                                if ui.selectable_label(checked, buffer.to_string()).clicked() {
                                    self.opt.buffer_size = Some(*buffer);
                                }
                            }
                        });
                }
                None => {
                    self.opt.buffer_size = None;
                }
            },
            Err(e) => {
                log::error!("Could not get audio device config: {e}");
            }
        };
    }

    fn config_sample_rate(&mut self, ui: &mut Ui) {
        egui::ComboBox::from_label("LTC Input Sample Rate")
            .selected_text(format!("{:?}hz", self.opt.sample_rate))
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut self.opt.sample_rate, 44100, "44100hz");
                ui.selectable_value(&mut self.opt.sample_rate, 48000, "48000hz");
            });
    }

    fn config_frame_rate(&mut self, ui: &mut Ui) {
        egui::ComboBox::from_label("Frame Rate")
            .selected_text(format!("{}", self.opt.fps))
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut self.opt.fps, 23.976, "23.976");
                ui.selectable_value(&mut self.opt.fps, 24.0, "24.0");
                ui.selectable_value(&mut self.opt.fps, 25.0, "25.0");
                ui.selectable_value(&mut self.opt.fps, 29.97, "29.97");
                ui.selectable_value(&mut self.opt.fps, 30.0, "30.0");
            });
    }

    fn config_ntfs(&mut self, ui: &mut Ui) {
        egui::ComboBox::from_label("NTSC/FCM")
            .selected_text(String::from(self.opt.ntsc))
            .show_ui(ui, |ui| {
                ui.selectable_value(
                    &mut self.opt.ntsc,
                    edl::Fcm::NonDropFrame,
                    String::from(edl::Fcm::NonDropFrame),
                );
                ui.selectable_value(
                    &mut self.opt.ntsc,
                    edl::Fcm::DropFrame,
                    String::from(edl::Fcm::DropFrame),
                );
            });
    }

    fn config_tcp_port(&mut self, ui: &mut Ui) {
        ui.add(egui::Slider::new(&mut self.opt.port, 3000..=9000).text("TCP Port"));
    }
}

impl eframe::App for AppGui {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("EzEDL v0.1");

            ui.add_enabled_ui(self.server_handle.is_none(), |ui| {
                ui.add_space(10.0);
                self.config_project_title(ui);
                ui.add_space(10.0);
                self.config_storage_dir(ui);
                ui.add_space(10.0);
                ui.separator();
                ui.add_space(10.0);
                self.config_input_channel(ui);
                ui.add_space(10.0);
                self.config_sample_rate(ui);
                ui.add_space(10.0);
                self.config_buffer_size(ui);
                ui.add_space(10.0);
                self.config_frame_rate(ui);
                ui.add_space(10.0);
                self.config_ntfs(ui);
                ui.add_space(10.0);
                self.config_tcp_port(ui);
                ui.add_space(10.0);
                ui.separator();
                ui.add_space(10.0);
            });

            ui.add_enabled_ui(self.server_handle.is_none(), |ui| {
                if ui.button("Launch Server").clicked() {
                    self.spawn_server()
                }
            });

            ui.add_enabled_ui(self.server_handle.is_some(), |ui| {
                if ui.button("Stop Server").clicked() {
                    self.kill_server()
                        .unwrap_or_else(|e| log::error!("Unable to kill server: {e}"))
                }
            });

            ui.add_space(10.0);
            let mut logs_displayed: usize = 0;

            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .max_height(ui.available_height() - 20.0)
                .stick_to_bottom(true)
                .show(ui, |ui| {
                    Logger::try_get_log(|logs| {
                        logs.iter().for_each(|(level, string)| {
                            let string_format = format!("[{}]: {}", level, string);

                            match level {
                                log::Level::Warn => {
                                    ui.colored_label(egui::Color32::YELLOW, string_format)
                                }
                                log::Level::Error => {
                                    ui.colored_label(egui::Color32::RED, string_format)
                                }
                                _ => ui.label(string_format),
                            };

                            logs_displayed += 1;
                        });
                        ctx.request_repaint();
                    });
                });
        });
    }
}
