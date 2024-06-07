///////////////////////////////////////////////////////////////////////
// /´￣￣　　　　　　　　　　　　　　　　 　 　 ／:::::::::::::::::::/
// 　　　　 /　　　　　　　　　　　　　　　　　　　　ヽ::::::::::::/
// 　　　 ./　　　　　　　　　　　　　　　　　　　　　　 :::::::/
// 　　　/　　　　　　　　　　　　　　　　　　　　　 　　 V
// 　　./　　　　ーtッ-､,　　　　　　　　　　　　　　　　i
// 　 / 　 　 　　` ー '´　 　 　 　 　　　.'r‐tッ‐ｧ　 　 |
// 　,′　　　　　　 　 　 　 　 　 　 　 　 `'ー　´　 　∧
// / .＼　　　　　　　　　　　　　　　　　　　　　　　　/ 　',
// 　　　＼　　　　　　　　 、　　　　　　 　 　 　 　 / 　　 ',
// 　　　　 ＞　　　　　　 　 ー 'ヽ __ﾉ　 　 　 　 ／　 　 　',
// 　　　／ 　　　≧ｭ ．.　　　　　　　　　　　 ＜|　 　 　 　 ',
///////////////////////////////////////////////////////////////////////
use anyhow::Error;
use clap::Parser;
use edl_gen::server::Server;
use edl_gen::Opt;
use egui::mutex::Mutex;
use std::io::prelude::*;
use std::net::TcpStream;
use std::sync::{mpsc, Arc};
use std::thread;

fn main() -> Result<(), eframe::Error> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([320.0, 240.0]),
        ..Default::default()
    };

    eframe::run_native(
        "EzEDL",
        options,
        Box::new(|_cc| {
            let app = App::new();
            Box::new(app)
        }),
    )
}

struct App {
    opt: Opt,
    is_listening: bool,
    rx_stop_serv: Arc<Mutex<mpsc::Receiver<()>>>,
    tx_stop_serv: mpsc::Sender<()>,
}

impl App {
    fn new() -> Self {
        let (tx_stop_serv, rx_stop_serv) = mpsc::channel::<()>();
        App {
            opt: Opt::parse(),
            is_listening: false,
            rx_stop_serv: Arc::new(Mutex::new(rx_stop_serv)),
            tx_stop_serv,
        }
    }

    fn spawn_server(&mut self) {
        let opt = self.opt.clone();
        let rx_stop_serv = Arc::clone(&self.rx_stop_serv);
        thread::spawn(move || Server::new(&opt).listen(rx_stop_serv));
        self.is_listening = true;
    }

    fn halt_server(&mut self) -> Result<(), Error> {
        self.tx_stop_serv.send(())?;

        let host = format!("127.0.0.1:{}", self.opt.port);
        let mut stream = TcpStream::connect(&host)?;
        let request = format!(
            "GET /SIGKILL HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n",
            host
        );

        stream.write_all(request.as_bytes())?;

        let mut response = String::new();
        stream.read_to_string(&mut response)?;

        self.is_listening = false;
        Ok(())
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("EzEDL v0.1");

            ui.add(egui::Slider::new(&mut self.opt.port, 3000..=8000).text("port"));

            ui.label(format!(
                "Config:\n Project Title: {}\n Storage Directory: {}\n Port: {}\n",
                self.opt.title, self.opt.dir, self.opt.port,
            ));

            if ui.button("Launch Server").clicked() && !self.is_listening {
                self.spawn_server()
            }

            if ui.button("Stop Server").clicked() && self.is_listening {
                self.halt_server()
                    .unwrap_or_else(|e| eprint!("Unable to halt server: {e}"))
            }
        });
    }
}
