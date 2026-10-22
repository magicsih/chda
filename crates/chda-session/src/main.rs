//! No GPUI dependency: hooks and the service survive application replacement.
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = match args.first().map(String::as_str) {
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
