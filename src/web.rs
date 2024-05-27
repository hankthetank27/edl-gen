use std::net::TcpListener;

use crate::Opt;

pub fn listen(opt: &Opt) {
    let port = format!("127.0.0.1:{}", opt.port);
    println!("{}", port);
    let listener = TcpListener::bind(port).unwrap();

    for stream in listener.incoming() {
        let stream = stream.unwrap();

        println!("Connection established!");
    }
}
