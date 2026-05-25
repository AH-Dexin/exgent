use exgent::{app::AppRuntime, cli, tui};

fn main() {
    match cli::parse(std::env::args().skip(1)) {
        Ok(cli::Command::Help(options)) => {
            print!("{}", cli::localized_help_text(&options));
        }
        Ok(cli::Command::Version) => {
            println!("{}", cli::version_text());
        }
        Ok(cli::Command::Run(options)) => {
            let mut runtime = match AppRuntime::new(options) {
                Ok(runtime) => runtime,
                Err(error) => {
                    eprintln!("exgent: {error}");
                    std::process::exit(1);
                }
            };
            if let Err(error) = tui::run(&mut runtime) {
                eprintln!("exgent: {error}");
                std::process::exit(1);
            }
        }
        Err(error) => {
            eprintln!("exgent: {error}");
            eprintln!("Run `exgent --help` for usage.");
            std::process::exit(2);
        }
    }
}
