use std::env;
use std::process::{exit, Command};

fn language() -> memorph::config::UiLanguage {
    memorph::config::web_preferences()
        .map(|preferences| preferences.language)
        .unwrap_or_default()
}

fn main() {
    let mut memorph_path = match env::current_exe() {
        Ok(path) => path,
        Err(error) => {
            eprintln!("{}", memorph::i18n::format(language(), "cliMemoLocateFailed", &[("error", &error.to_string())]));
            exit(1);
        }
    };

    memorph_path.set_file_name(if cfg!(windows) {
        "memorph.exe"
    } else {
        "memorph"
    });

    let status = Command::new(&memorph_path)
        .args(env::args_os().skip(1))
        .status();

    match status {
        Ok(status) => exit(status.code().unwrap_or(1)),
        Err(error) => {
            eprintln!("{}", memorph::i18n::format(language(), "cliMemoRunFailed", &[("path", &memorph_path.display().to_string()), ("error", &error.to_string())]));
            exit(1);
        }
    }
}
