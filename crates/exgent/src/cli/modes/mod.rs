use std::io;

use exgent_core::AppRuntimeHost;

pub mod interactive;
pub mod print;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AppMode {
    Interactive(interactive::InteractiveMode),
    Print(print::PrintMode),
    Json(print::PrintMode),
}

impl AppMode {
    pub fn interactive() -> Self {
        Self::Interactive(interactive::InteractiveMode::resolve())
    }

    pub fn print(prompt: Option<String>) -> Self {
        Self::Print(print::PrintMode::new(prompt))
    }

    pub fn json(prompt: Option<String>) -> Self {
        Self::Json(print::PrintMode::json(prompt))
    }

    pub fn run(self, runtime: &mut AppRuntimeHost) -> io::Result<()> {
        match self {
            Self::Interactive(mode) => mode.run(runtime),
            Self::Print(mode) | Self::Json(mode) => mode.run(runtime),
        }
    }
}
