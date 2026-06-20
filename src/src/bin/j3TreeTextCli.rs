use std::io::{self, Write};
use std::process::ExitCode;

fn main() -> io::Result<ExitCode> {
    let args = std::env::args_os().skip(1);

    if let Err(error) = j3treetext::cli::run(args) {
        let mut stderr = io::stderr().lock();
        writeln!(stderr, "{}", error.user_message())?;
        writeln!(stderr, "detail: {error}")?;
        return Ok(ExitCode::FAILURE);
    }

    Ok(ExitCode::SUCCESS)
}
