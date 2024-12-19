use anyhow::{anyhow, Error};
use eframe::egui::{self, mutex::Mutex, Ui};
use ltc::LTCFrame;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc, Arc,
};
use std::thread::{self, JoinHandle};

use crate::ltc_decode::LTCConfigs;
use crate::{
    edl,
    ltc_decode::{LTCDevice, LTCListener},
    server::Server,
    single_val_channel, update_version, Logger, Opt, StoredOpts,
};

pub struct App {
    rx_stop_serv: Arc<Mutex<mpsc::Receiver<()>>>,
    tx_stop_serv: mpsc::Sender<()>,
    tx_serv_stopped: mpsc::Sender<()>,
    rx_serv_stopped: mpsc::Receiver<()>,
    tx_ltc_frame: Option<single_val_channel::Sender<LTCFrame>>,
    server_handle: Option<JoinHandle<Result<(), Error>>>,
    is_current_version: Arc<AtomicBool>,
    opt: Opt,
}

impl Default for App {
    fn default() -> Self {
        let (tx_stop_serv, rx_stop_serv) = mpsc::channel::<()>();
        let (tx_serv_stopped, rx_serv_stopped) = mpsc::channel::<()>();
        let is_current_version = Arc::new(AtomicBool::new(true));

        let is_current_version_check = Arc::clone(&is_current_version);
        thread::spawn(move || match update_version::update_available() {
            Ok(is_available) => is_current_version_check.store(!is_available, Ordering::Relaxed),
            Err(e) => eprintln!("{e}"),
        });

        App {
            server_handle: None,
            rx_stop_serv: Arc::new(Mutex::new(rx_stop_serv)),
            tx_ltc_frame: None,
            tx_stop_serv,
            tx_serv_stopped,
            rx_serv_stopped,
            is_current_version,
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
        let mut label = ui.label(self.opt.dir.to_str().unwrap_or(""));
        if ui.button("Storage Directory").clicked() {
            if let Some(path) = rfd::FileDialog::new().pick_folder() {
                self.opt.dir = path;
                label.mark_changed();
            }
        }
        label.write_on_change(&self.opt, StoredOpts::Dir);
    }

    //TODO: we should be trying to match the buffer size and input channels when we change devices
    // if they're available, only reverting to default if they're not
    fn config_input_device(&mut self, ui: &mut Ui) {
        let get_name = |device: Option<&LTCDevice>| {
            device.map_or("No Device Found".to_string(), |d| {
                d.name().unwrap_or_else(|| "Device Has No Name".to_string())
            })
        };
        let current_device_name = get_name(self.opt.ltc_device.as_ref());
        egui::ComboBox::from_label("Audio Device")
            .selected_text(current_device_name.to_string())
            .show_ui(ui, |ui| match &self.opt.ltc_devices {
                Some(devices) => {
                    for ltc_device in devices.iter() {
                        let device_name = get_name(Some(ltc_device));
                        let checked = device_name == current_device_name;
                        let mut label = ui.selectable_label(checked, device_name);
                        if label.clicked() {
                            self.opt.ltc_device = Some(ltc_device.to_owned());
                            self.opt.input_channel = ltc_device.get_default_channel(None);
                            self.opt.buffer_size = ltc_device.get_default_buffer_size(None);
                            label.mark_changed();
                        }
                        label
                            .write_on_change(&self.opt, StoredOpts::LTCDevice)
                            .write_on_change(&self.opt, StoredOpts::BufferSize)
                            .write_on_change(&self.opt, StoredOpts::InputChannel);
                    }
                }
                None => {
                    ui.label("No Audio Device Found");
                }
            });
    }

    fn refresh_input_device(&mut self, ui: &mut Ui) {
        let mut button = ui.button("Refresh Devices");
        if button.clicked() {
            self.opt.ltc_devices = LTCDevice::try_get_devices().ok();
            if self.opt.ltc_device.is_none() {
                let LTCConfigs {
                    ltc_devices: _,
                    ltc_device,
                    input_channel,
                    buffer_size,
                } = LTCConfigs::default_no_device_list();
                self.opt.ltc_device = ltc_device;
                self.opt.input_channel = input_channel;
                self.opt.buffer_size = buffer_size;
                button.mark_changed();
            }
        }
        button
            .write_on_change(&self.opt, StoredOpts::LTCDevice)
            .write_on_change(&self.opt, StoredOpts::BufferSize)
            .write_on_change(&self.opt, StoredOpts::InputChannel);
    }

    fn config_input_channel(&mut self, ui: &mut Ui) {
        let label = self
            .opt
            .input_channel
            .map(|ch| ch.to_string())
            .unwrap_or_else(|| "None Available".to_string())
            .to_string();
        egui::ComboBox::from_label("Input Channel")
            .selected_text(label)
            .show_ui(ui, |ui| match &self.opt.ltc_device {
                Some(ltc_device) => {
                    (1..&ltc_device.config.channels() + 1).for_each(|channel| {
                        let channel = channel as usize;
                        let checked = Some(channel) == self.opt.input_channel;
                        let mut label = ui.selectable_label(checked, channel.to_string());
                        if label.clicked() {
                            self.opt.input_channel = Some(channel);
                            label.mark_changed();
                        }
                        label.write_on_change(&self.opt, StoredOpts::InputChannel);
                    });
                }
                None => {
                    ui.label("No Audio Device Found");
                }
            });
    }

    fn config_buffer_size(&mut self, ui: &mut Ui) {
        let label = self
            .opt
            .buffer_size
            .map(|ch| ch.to_string())
            .unwrap_or_else(|| "None Available".to_string())
            .to_string();
        egui::ComboBox::from_label("Buffer Size")
            .selected_text(label)
            .show_ui(ui, |ui| match &self.opt.ltc_device {
                Some(device) => match device.get_buffer_opts() {
                    Some(opts) => opts.into_iter().for_each(|buffer| {
                        let checked = Some(buffer) == self.opt.buffer_size;
                        let mut label = ui.selectable_label(checked, buffer.to_string());
                        if label.clicked() {
                            self.opt.buffer_size = Some(buffer);
                            label.mark_changed();
                        }
                        label.write_on_change(&self.opt, StoredOpts::BufferSize);
                    }),
                    None => {
                        self.opt.buffer_size = None;
                    }
                },
                None => {
                    ui.label("No Audio Device Found");
                }
            });
    }

    fn config_sample_rate(&mut self, ui: &mut Ui) {
        egui::ComboBox::from_label("LTC Input Sample Rate")
            .selected_text(format!("{:?}hz", self.opt.sample_rate))
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut self.opt.sample_rate, 44_100, "44100hz")
                    .write_on_change(&self.opt, StoredOpts::SampleRate);
                ui.selectable_value(&mut self.opt.sample_rate, 48_000, "48000hz")
                    .write_on_change(&self.opt, StoredOpts::SampleRate);
            });
    }

    fn config_frame_rate(&mut self, ui: &mut Ui) {
        egui::ComboBox::from_label("Frame Rate")
            .selected_text(format!("{}", self.opt.fps))
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut self.opt.fps, 23.976, "23.976")
                    .write_on_change(&self.opt, StoredOpts::Fps);
                ui.selectable_value(&mut self.opt.fps, 24.0, "24.0")
                    .write_on_change(&self.opt, StoredOpts::Fps);
                ui.selectable_value(&mut self.opt.fps, 25.0, "25.0")
                    .write_on_change(&self.opt, StoredOpts::Fps);
                ui.selectable_value(&mut self.opt.fps, 29.97, "29.97")
                    .write_on_change(&self.opt, StoredOpts::Fps);
                ui.selectable_value(&mut self.opt.fps, 30.0, "30.0")
                    .write_on_change(&self.opt, StoredOpts::Fps);
            });
    }

    fn config_ntfs(&mut self, ui: &mut Ui) {
        egui::ComboBox::from_label("NTSC/FCM")
            .selected_text(String::from(self.opt.ntsc))
            .show_ui(ui, |ui| {
                ui.selectable_value(
                    &mut self.opt.ntsc,
                    edl::Ntsc::NonDropFrame,
                    String::from(edl::Ntsc::NonDropFrame),
                )
                .write_on_change(&self.opt, StoredOpts::Ntsc);
                ui.selectable_value(
                    &mut self.opt.ntsc,
                    edl::Ntsc::DropFrame,
                    String::from(edl::Ntsc::DropFrame),
                )
                .write_on_change(&self.opt, StoredOpts::Ntsc);
            });
    }

    fn config_tcp_port(&mut self, ui: &mut Ui) {
        ui.add(egui::Slider::new(&mut self.opt.port, 3000..=9999).text("TCP Port"))
            .write_on_change(&self.opt, StoredOpts::Port);
    }

    fn logger(&mut self, ui: &mut Ui) {
        Logger::try_get_log(|logs| {
            let scroll = egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .max_height(ui.available_height() - 2.0)
                .stick_to_bottom(true)
                .show(ui, |ui| {
                    logs.iter().for_each(|(level, string)| {
                        match level {
                            log::Level::Warn => ui.colored_label(egui::Color32::YELLOW, string),
                            log::Level::Error => ui.colored_label(egui::Color32::RED, string),
                            _ => ui.label(string),
                        };
                    });
                });
            let frame = egui::Frame::none()
                .fill(egui::Color32::from_rgba_premultiplied(18, 18, 18, 50))
                .rounding(egui::Rounding::from(3.0))
                .paint(scroll.inner_rect.expand2(egui::vec2(2.0, 3.0)));
            ui.painter().add(frame);
        });
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| {
            let space = 10.0;
            ui.heading("EDLgen");

            if !self.is_current_version.load(Ordering::Relaxed) {
                ui.add_enabled_ui(self.server_handle.is_none(), |ui| {
                    if ui
                        .link("Update available! Click to install and restart EDLgen.")
                        .clicked()
                    {
                        if let Err(e) = update_version::update() {
                            eprintln!("{}", e);
                        }
                    }
                });
            }

            ui.add_enabled_ui(self.server_handle.is_none(), |ui| {
                ui.add_space(space);
                self.config_project_title(ui);
                ui.add_space(space);
                self.config_storage_dir(ui);
                ui.add_space(space);
                ui.separator();
                ui.add_space(space);
                self.config_input_device(ui);
                self.refresh_input_device(ui);
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
            self.logger(ui)
        });
    }
}

trait WriteChange {
    fn write_on_change(self, opt: &Opt, stored_opt: StoredOpts) -> Self;
}

impl WriteChange for egui::Response {
    fn write_on_change(self, opt: &Opt, stored_opt: StoredOpts) -> Self {
        if self.changed() {
            stored_opt.write(opt);
        }
        self
    }
}
