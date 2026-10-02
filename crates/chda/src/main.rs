//! Entry point. `chda` runs the GUI; `chda hook <agent>` is the command
//! coding agents call to report their status; `chda note [<text>]` prints or
//! sets the note of the current worktree's branch; `chda mcp` is an MCP
//! server agents can start to create worktrees, open tabs and report status.

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("hook") => std::process::exit(chda_agents::hook_main(&args[1..])),
        Some("note") => std::process::exit(chda_core::note_main(&args[1..])),
        Some("mcp") => std::process::exit(chda_agents::mcp_main(&args[1..])),
        Some("--version" | "-V") => println!("chda {}", env!("CARGO_PKG_VERSION")),
        Some(other) => {
            eprintln!("unknown argument: {other}");
            std::process::exit(2);
        }
        None => {
            let theme = chda_config::ChdaConfig::default_path()
                .and_then(|p| chda_config::ChdaConfig::load(&p).ok())
                .and_then(|c| c.theme);
            chda_ui::run(chda_config::load(
                &chda_config::Paths::default_for_user(),
                theme.as_deref(),
            ));
        }
    }
}
