//! Offline configuration entry point; native serving is not yet implemented.
use std::io::Read;

fn run() -> Result<(), String> {
    let arguments: Vec<_> = std::env::args().skip(1).collect();
    match arguments.as_slice() {
        [arg] if arg == "--help" || arg == "-h" => println!(
            "Usage: provlita check --config PATH\n       provlita --version\nNative serving is not implemented yet."
        ),
        [arg] if arg == "--version" => println!("provlita {}", env!("CARGO_PKG_VERSION")),
        [command, option, path] if command == "check" && option == "--config" => {
            let file = std::fs::File::open(path).map_err(|e| format!("open configuration: {e}"))?;
            let mut text = String::new();
            file.take(provlita::config::MAX_CONFIG_BYTES as u64 + 1)
                .read_to_string(&mut text)
                .map_err(|e| format!("read configuration: {e}"))?;
            let config = provlita::config::Config::parse(&text)?;
            let (width, height) = config.logical_size();
            println!(
                "provlita_config schema=1 status=valid pins={} width={} height={} catalog_resolved=false native=false",
                config.pins.len(),
                width,
                height
            );
        }
        _ => {
            return Err(
                "use provlita check --config PATH; native serving is not implemented yet".into(),
            );
        }
    }
    Ok(())
}
fn main() {
    if let Err(error) = run() {
        eprintln!("provlita: {error}");
        std::process::exit(2);
    }
}
