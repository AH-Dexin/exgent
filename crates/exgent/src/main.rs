use std::io::IsTerminal;

use exgent::{cli, cli::modes::AppMode};
use exgent_core::AppRuntimeHost;

fn main() {
    match cli::parse(std::env::args().skip(1)) {
        Ok(cli::Command::Help(options)) => {
            print!("{}", cli::localized_help_text(&options));
        }
        Ok(cli::Command::Version) => {
            println!("{}", cli::version_text());
        }
        Ok(cli::Command::Run(options)) => {
            let mode = if std::io::stdin().is_terminal() {
                AppMode::interactive()
            } else {
                AppMode::print(None)
            };
            run_app(options, mode);
        }
        Ok(cli::Command::Print { options, prompt }) => {
            run_app(options, AppMode::print(prompt));
        }
        Ok(cli::Command::Json { options, prompt }) => {
            run_app(options, AppMode::json(prompt));
        }
        Err(error) => {
            eprintln!("exgent: {error}");
            eprintln!("Run `exgent --help` for usage.");
            std::process::exit(2);
        }
    }
}

fn run_app(options: cli::CliOptions, mode: AppMode) {
    let mut runtime = match AppRuntimeHost::new(options.into()) {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("exgent: {error}");
            std::process::exit(1);
        }
    };

    if let Err(error) = mode.run(&mut runtime) {
        eprintln!("exgent: {error}");
        std::process::exit(1);
    }
}
