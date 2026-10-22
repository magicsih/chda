//! No GPUI dependency: hooks and the service survive application replacement.
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = match args.first().map(String::as_str) {
        Some("agent-settings") if args.get(1).map(String::as_str) == Some("claude") => {
            let result = std::env::var_os("HOME")
                .map(std::path::PathBuf::from)
                .zip(chda_agents::hook::data_dir())
                .zip(std::env::current_dir().ok())
                .zip(std::env::current_exe().ok())
                .ok_or_else(|| std::io::Error::other("agent settings locations unavailable"))
                .and_then(|(((home, dir), cwd), bin)| {
                    chda_agents::statusline::launch_settings(&home, &cwd, &dir, &bin)
                });
            match result {
                Ok(path) => {
                    println!("{}", path.display());
                    0
                }
                Err(_) => 1,
            }
        }
        Some("serve") => match args
            .get(1)
            .map(std::path::Path::new)
            .map(chda_session::serve)
        {
            Some(Ok(())) => 0,
            Some(Err(e)) => {
                eprintln!("chda-session: {e}");
                1
            }
            None => 2,
        },
        Some("hook") => chda_agents::hook_main(&args[1..]),
        Some("mcp") => chda_agents::mcp_main(&args[1..]),
        Some("statusline") => chda_agents::statusline::main(&args[1..]),
        Some("--protocol") => {
            println!("{}", chda_session::PROTOCOL_VERSION);
            0
        }
        Some("--version") => {
            println!(
                "chda-session {} protocol {}",
                env!("CARGO_PKG_VERSION"),
                chda_session::PROTOCOL_VERSION
            );
            0
        }
        _ => 2,
    };
    std::process::exit(code);
}
