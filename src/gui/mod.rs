mod update_version;

use anyhow::{anyhow, Error};
use eframe::egui::{self, Ui};
use ltc::LTCFrame;
use parking_lot::Mutex;

use std::{
    io::{Read, Write},
    net::TcpStream,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use crate::{
    edl_writer,
    ltc_decoder::{
        config::{DevicesFromHost, LTCDevice, LTCHostId},
        LTCListener,
    },
    server::Server,
    state::{Logger, Opt, StoredOpts},
    utils::single_val_channel,
};

pub struct App {
    // Arc because we need more than one owner, and Mutex to implement Sync
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
        let _ = thread::Builder::new()
            .name("edlgen-updater".into())
            .spawn(move || match update_version::update_available() {
                Ok(is_available) => {
                    is_current_version_check.store(!is_available, Ordering::Relaxed)
                }
                Err(e) => eprintln!("Error fetching update status: {e}"),
            })
            .map_err(|e| {
                eprintln!("Error spawning update thread: {e}");
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
    fn spawn_server(&mut self) -> Result<(), Error> {
        let decode_handlers = LTCListener::new(self.opt.clone())
            .map_err(|e| e.context("Unable to initate LTC listener"))
            .and_then(|listener| listener.listen())
            .map_err(|e| e.context("Error spawning LTC listener thread"))?;

        let opt = self.opt.clone();
        let rx_stop_serv = Arc::clone(&self.rx_stop_serv);
        let tx_serv_stopped = self.tx_serv_stopped.clone();

        self.tx_ltc_frame = Some(decode_handlers.tx_ltc_frame.clone());
        self.server_handle = Some(
            thread::Builder::new()
                .name("edlgen-server".into())
                .spawn(move || {
                    Server::new(opt.port).listen(
                        rx_stop_serv,
                        tx_serv_stopped,
                        decode_handlers,
                        opt,
                    )
                })
                .map_err(|e| anyhow!("Error spawning server thread: {e}"))?,
        );
        Ok(())
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

                    signal_shutdown().ok();

                    if let Err(e) = self.rx_serv_stopped.recv_timeout(Duration::from_secs(3)) {
                        self.server_handle = Some(handle);
                        return Err(anyhow!("Could not kill server: {}", e));
                    }
                }

                handle.join().map_err(|e| {
                    anyhow!("Could not kill server, error waiting for shutdown: {:?}", e)
                })?
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

    fn config_driver_type(&mut self, ui: &mut Ui) {
        let current_host_name = self.opt.ltc_host.id().get_name();
        egui::ComboBox::from_label("Audio Driver")
            .selected_text(current_host_name.trim_with_ellipsis())
            .show_ui(ui, |ui| {
                for host_id in self.opt.ltc_hosts.iter() {
                    let host: cpal::Host = LTCHostId::new(*host_id).into();
                    let host_name = host.id().get_name();
                    let checked = host_name == current_host_name;
                    let mut label = ui.selectable_label(checked, host_name);
                    if label.clicked() {
                        self.opt.ltc_host = Arc::new(host);
                        self.opt.ltc_device = LTCDevice::try_get_default(&self.opt.ltc_host).ok();
                        self.opt.ltc_devices = LTCDevice::try_get_devices(&self.opt.ltc_host).ok();
                        self.opt.input_channel = self.opt.ltc_device.as_ref().and_then(|device| {
                            device.match_input_or_default(self.opt.input_channel)
                        });
                        self.opt.buffer_size = self.opt.ltc_device.as_ref().and_then(|buff_size| {
                            buff_size.match_buffer_or_default(self.opt.buffer_size)
                        });
                        label.mark_changed();
                    }
                    label
                        .write_on_change(&self.opt, StoredOpts::LTCHostId)
                        .write_on_change(&self.opt, StoredOpts::LTCDevice)
                        .write_on_change(&self.opt, StoredOpts::BufferSize)
                        .write_on_change(&self.opt, StoredOpts::InputChannel);
                }
            });
    }

    fn config_input_device(&mut self, ui: &mut Ui) {
        let current_device_name = self.opt.ltc_device.as_ref().get_name();
        egui::ComboBox::from_label("Audio Device")
            .selected_text(current_device_name.trim_with_ellipsis())
            .show_ui(ui, |ui| match &self.opt.ltc_devices {
                Some(devices) => {
                    for new_device in devices.iter() {
                        let device_name = Some(new_device).get_name();
                        let checked = device_name == current_device_name;
                        let mut label = ui.selectable_label(checked, device_name);
                        if label.clicked() {
                            self.opt.input_channel =
                                new_device.match_input_or_default(self.opt.input_channel);
                            self.opt.buffer_size =
                                new_device.match_buffer_or_default(self.opt.buffer_size);
                            self.opt.ltc_device = Some(new_device.to_owned());
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

    fn refresh_input_devices(&mut self, ui: &mut Ui) {
        let mut button = ui.button("Refresh Devices");
        if button.clicked() {
            self.opt.ltc_devices = LTCDevice::try_get_devices(&self.opt.ltc_host).ok();
            if self.opt.ltc_device.is_none() {
                self.opt.ltc_device = LTCDevice::try_get_default(&self.opt.ltc_host).ok();
                self.opt.input_channel = self
                    .opt
                    .ltc_device
                    .as_ref()
                    .and_then(|device| device.match_input_or_default(self.opt.input_channel));
                self.opt.buffer_size =
                    self.opt.ltc_device.as_ref().and_then(|buff_size| {
                        buff_size.match_buffer_or_default(self.opt.buffer_size)
                    });
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
                    edl_writer::Ntsc::NonDropFrame,
                    String::from(edl_writer::Ntsc::NonDropFrame),
                )
                .write_on_change(&self.opt, StoredOpts::Ntsc);
                ui.selectable_value(
                    &mut self.opt.ntsc,
                    edl_writer::Ntsc::DropFrame,
                    String::from(edl_writer::Ntsc::DropFrame),
                )
                .write_on_change(&self.opt, StoredOpts::Ntsc);
            });
    }

    fn config_tcp_port(&mut self, ui: &mut Ui) {
        ui.add(egui::Slider::new(&mut self.opt.port, 3000..=9999).text("TCP Port"))
            .write_on_change(&self.opt, StoredOpts::Port);
    }

    fn logger(&mut self, ui: &mut Ui) {
        Logger::get_log(|logs| {
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
                            eprintln!("Error updating: {}", e);
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
                self.config_driver_type(ui);
                ui.add_space(space);
                self.config_input_device(ui);
                self.refresh_input_devices(ui);
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
                        .unwrap_or_else(|e| log::error!("Unable to spawn server: {e}"))
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

trait Name {
    fn get_name(&self) -> String;
}

impl Name for cpal::HostId {
    fn get_name(&self) -> String {
        <&str>::from(LTCHostId::new(*self)).to_string()
    }
}

impl Name for Option<&LTCDevice> {
    fn get_name(&self) -> String {
        self.map_or("No Device Found".to_string(), |d| {
            d.name().unwrap_or_else(|| "Device Has No Name".to_string())
        })
    }
}

trait Trim {
    fn trim_with_ellipsis(&self) -> String;
}

impl Trim for str {
    fn trim_with_ellipsis(&self) -> String {
        let max = 30;
        if self.len() >= max {
            format!("{}...", self.get(0..(max - 3)).unwrap())
        } else {
            self.to_string()
        }
    }
}

// #[cfg(test)]
// mod test {
//     use super::*;
//     use egui_kittest::Harness;

//     fn make_app() {
//         let ctx = egui::Context::default();
//         // let ui = egui::UiBuilder::new();
//         //
//         // let mut harness = Harness::builder().build_eframe(|cc| App::default());

//         // let mut harness = HarnessBuilder::default().build_ui(|ui| {
//         //     let app = App::default();
//         // });
//     }
// }
