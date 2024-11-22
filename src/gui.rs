use anyhow::{anyhow, Error};
use cpal::traits::DeviceTrait;
use eframe::egui::{self, mutex::Mutex, Ui};
use ltc::LTCFrame;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::{mpsc, Arc};
use std::thread::{self, JoinHandle};

use crate::ltc_decode::LTCListener;
use crate::server::Server;
use crate::single_val_channel;
use crate::Logger;
use crate::{edl, Opt};

pub struct App {
    rx_stop_serv: Arc<Mutex<mpsc::Receiver<()>>>,
    tx_stop_serv: mpsc::Sender<()>,
    tx_serv_stopped: mpsc::Sender<()>,
    rx_serv_stopped: mpsc::Receiver<()>,
    tx_ltc_frame: Option<single_val_channel::Sender<LTCFrame>>,
    server_handle: Option<JoinHandle<Result<(), Error>>>,
    opt: Opt,
}

impl Default for App {
    fn default() -> Self {
        let (tx_stop_serv, rx_stop_serv) = mpsc::channel::<()>();
        let (tx_serv_stopped, rx_serv_stopped) = mpsc::channel::<()>();
        App {
            server_handle: None,
            rx_stop_serv: Arc::new(Mutex::new(rx_stop_serv)),
            tx_ltc_frame: None,
            tx_stop_serv,
            tx_serv_stopped,
            rx_serv_stopped,
            opt: Opt::default(),
        }
    }
}

impl App {
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

        self.tx_ltc_frame = Some(decode_handlers.tx_ltc_frame.clone());
        self.server_handle = Some(thread::spawn(move || {
            Server::new(&opt.port).listen(rx_stop_serv, tx_serv_stopped, decode_handlers, opt)
        }));
    }

    fn kill_server(&mut self) -> Result<(), Error> {
        match self.server_handle.take() {
            Some(handle) => {
                self.tx_stop_serv.send(())?;
                if let Some(tx_ltc_frame) = self.tx_ltc_frame.as_ref() {
                    tx_ltc_frame.hangup();
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
        ui.add(egui::Slider::new(&mut self.opt.port, 3000..=9999).text("TCP Port"));
    }

    fn logger(&mut self, ui: &mut Ui, ctx: &egui::Context) {
        let scroll = egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .max_height(ui.available_height() - 2.0)
            .stick_to_bottom(true)
            .show(ui, |ui| {
                Logger::try_get_log(|logs| {
                    logs.iter().for_each(|(level, string)| {
                        // let string_format = format!("[{}]: {}", level, string);
                        let string_format = format!("{}", string);
                        match level {
                            log::Level::Warn => {
                                ui.colored_label(egui::Color32::YELLOW, string_format)
                            }
                            log::Level::Error => {
                                ui.colored_label(egui::Color32::RED, string_format)
                            }
                            // log::Level::Info => {
                            //     ui.colored_label(egui::Color32::LIGHT_YELLOW, string_format)
                            // }
                            _ => ui.label(string_format),
                        };
                    });
                    ctx.request_repaint();
                });
            });
        let frame = egui::Frame::none()
            .fill(egui::Color32::from_rgba_premultiplied(18, 18, 18, 50))
            .rounding(egui::Rounding::from(3.0))
            .paint(scroll.inner_rect.expand2(egui::vec2(2.0, 3.0)));
        ui.painter().add(frame);
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| {
            let space = 10.0;
            ui.heading("EDL-Server v0.1");

            ui.add_enabled_ui(self.server_handle.is_none(), |ui| {
                ui.add_space(space);
                self.config_project_title(ui);
                ui.add_space(space);
                self.config_storage_dir(ui);
                ui.add_space(space);
                ui.separator();
                ui.add_space(space);
                self.config_input_channel(ui);
                ui.add_space(space);
                self.config_buffer_size(ui);
                ui.add_space(space);
                self.config_sample_rate(ui);
                ui.add_space(space);
                self.config_frame_rate(ui);
                ui.add_space(space);
                self.config_ntfs(ui);
                ui.add_space(space);
                self.config_tcp_port(ui);
                ui.add_space(space);
                ui.separator();
                ui.add_space(space);
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

            ui.add_space(space);
            self.logger(ui, ctx)
        });
    }
}
