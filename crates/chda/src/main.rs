//! Entry point. Runs the GUI; CLI subcommands such as `chda hook` arrive later.

fn main() {
    let config = chda_config::load(&chda_config::Paths::default_for_user());
    chda_ui::run(chda_ui::Settings::from_ghostty(&config));
}
