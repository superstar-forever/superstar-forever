//! Desktop / Termux entry point.
//!
//! superstar-server [--data DIR_OR_ZIP]... [--bind ADDR] [--public-url URL] [--save FILE] [--log FILE]

use std::path::PathBuf;

const USAGE: &str = "usage: superstar-server [--data server-data] [--bind 0.0.0.0:18384] \
[--public-url http://<this-machine>:18384] [--save offline.save.json] [--log server.log]";

fn main() {
    let mut data: Vec<PathBuf> = Vec::new(); // --data may be repeated (several zips)
    let mut bind = "0.0.0.0:18384".to_string();
    let mut public_url: Option<String> = None;
    let mut save = PathBuf::from("offline.save.json");
    let mut log = None;

    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        let mut val = || args.next().unwrap_or_else(|| exit_usage(&format!("{a} needs a value")));
        match a.as_str() {
            "--data" => data.push(PathBuf::from(val())),
            "--bind" => bind = val(),
            "--public-url" => public_url = Some(val()),
            "--save" => save = PathBuf::from(val()),
            "--log" => log = Some(PathBuf::from(val())),
            "-h" | "--help" => exit_usage(""),
            other => exit_usage(&format!("unknown argument {other}")),
        }
    }
    let public_url = public_url.unwrap_or_else(|| {
        let port = bind.rsplit(':').next().unwrap_or("18384");
        let host = if bind.starts_with("0.0.0.0") { "127.0.0.1" } else { bind.rsplit_once(':').map_or("127.0.0.1", |b| b.0) };
        format!("http://{host}:{port}")
    });

    if data.is_empty() {
        data.push(PathBuf::from("server-data"));
    }
    if let Err(e) = superstar_server::run(superstar_server::Config { data, bind, public_url, save, log }) {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

fn exit_usage(msg: &str) -> ! {
    if !msg.is_empty() {
        eprintln!("error: {msg}");
    }
    eprintln!("{USAGE}");
    std::process::exit(2);
}
