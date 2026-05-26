use std::{
    env,
    io::{self, IsTerminal},
};

use exgent_core::AppRuntimeHost;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InteractiveUi {
    Tui,
    Plain,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InteractiveMode {
    ui: InteractiveUi,
}

impl InteractiveMode {
    pub fn resolve() -> Self {
        let ui = if io::stdin().is_terminal()
            && io::stdout().is_terminal()
            && env::var_os("EXGENT_LEGACY_TUI").is_none()
        {
            InteractiveUi::Tui
        } else {
            InteractiveUi::Plain
        };
        Self { ui }
    }

    pub fn with_ui(ui: InteractiveUi) -> Self {
        Self { ui }
    }

    pub fn ui(&self) -> InteractiveUi {
        self.ui
    }

    pub fn run(self, runtime: &mut AppRuntimeHost) -> io::Result<()> {
        match self.ui {
            InteractiveUi::Tui => exgent_tui::run_tui(runtime),
            InteractiveUi::Plain => exgent_tui::run_plain(runtime),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stores_selected_ui() {
        let mode = InteractiveMode::with_ui(InteractiveUi::Plain);

        assert_eq!(mode.ui(), InteractiveUi::Plain);
    }
}
