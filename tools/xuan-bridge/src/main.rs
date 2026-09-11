use std::env;
use std::path::PathBuf;

fn main() {
    let mut args = env::args().skip(1);
    match args.next().as_deref() {
        Some("--http") => {
            let address = args.next().unwrap_or_else(|| "127.0.0.1:57324".into());
            if let Err(error) = xuan_bridge::initialize_storage(&xuan_bridge::config_root()) {
                eprintln!("xuan-bridge storage initialization failed: {error}");
                std::process::exit(1);
            }
            if let Err(error) = xuan_bridge::serve_http(&address) {
                eprintln!("xuan-bridge HTTP server failed: {error}");
                std::process::exit(1);
            }
        }
        Some("init") => {
            let output = args
                .next()
                .map(PathBuf::from)
                .unwrap_or_else(xuan_bridge::config_root);
            match xuan_bridge::initialize_storage(&output) {
                Ok(value) => println!("{value}"),
                Err(error) => {
                    eprintln!("storage initialization failed: {error}");
                    std::process::exit(1);
                }
            }
        }
        Some("migrate") => {
            let Some(input) = args.next() else {
                eprintln!("usage: xuan-bridge migrate <legacy-settings.json> [output-root]");
                std::process::exit(2);
            };
            let output = args
                .next()
                .map(PathBuf::from)
                .unwrap_or_else(xuan_bridge::config_root);
            match xuan_bridge::migrate_legacy_settings(&PathBuf::from(input), &output) {
                Ok(value) => println!("{value}"),
                Err(error) => {
                    eprintln!("migration failed: {error}");
                    std::process::exit(1);
                }
            }
        }
        _ => {
            if let Err(error) = xuan_bridge::initialize_storage(&xuan_bridge::config_root()) {
                eprintln!("xuan-bridge storage initialization failed: {error}");
                std::process::exit(1);
            }
            if let Err(error) = xuan_bridge::serve_json_lines(std::io::stdin(), std::io::stdout()) {
                eprintln!("xuan-bridge failed: {error}");
                std::process::exit(1);
            }
        }
    }
}
