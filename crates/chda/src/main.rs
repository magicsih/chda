//! Entry point. `chda` runs the GUI; `chda hook <agent>` is the command
//! coding agents call to report their status.

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("hook") => std::process::exit(chda_agents::hook_main(&args[1..])),
        Some("--version" | "-V") => println!("chda {}", env!("CARGO_PKG_VERSION")),
        Some(other) => {
            eprintln!("unknown argument: {other}");
            std::process::exit(2);
        }
        None => {
            let config = chda_config::load(&chda_config::Paths::default_for_user());
            chda_ui::run(chda_ui::Settings::from_ghostty(&config));
        }
    }
}
