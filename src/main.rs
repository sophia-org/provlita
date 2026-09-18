//! Scaffold entry point; native protocol and GPU rendering are not implemented.

fn main() {
    match std::env::args().skip(1).collect::<Vec<_>>().as_slice() {
        [] => println!("Provlita: native Sophia GPU dock scaffold. See ARCHITECTURE.md."),
        [arg] if arg == "--help" || arg == "-h" => {
            println!(
                "Usage: provlita [--help | --version]\nNative dock execution is not implemented."
            );
        }
        [arg] if arg == "--version" => println!("provlita {}", env!("CARGO_PKG_VERSION")),
        _ => {
            eprintln!("provlita: unsupported arguments; native dock execution is not implemented");
            std::process::exit(2);
        }
    }
}
