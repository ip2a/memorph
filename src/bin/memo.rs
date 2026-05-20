use std::env;
use std::process::{exit, Command};

fn main() {
    let mut memorph_path = match env::current_exe() {
        Ok(path) => path,
        Err(error) => {
            eprintln!("Failed to locate memo executable: {}", error);
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
            eprintln!(
                "Failed to run memorph from memo alias at {}: {}",
                memorph_path.display(),
                error
            );
            exit(1);
        }
    }
}
