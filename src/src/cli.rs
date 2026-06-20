use std::ffi::OsString;

use crate::app::BootstrappedDocumentRepository;
use crate::error::AppError;

mod args;
mod commands;
mod content;
mod output;
mod parser;

use args::{set_once, CliArgs};

pub fn run(args: impl IntoIterator<Item = OsString>) -> Result<(), AppError> {
    let mut args = CliArgs::new(args);
    let mut db_path = None;

    loop {
        if args.peek_is("--db") {
            let _ = args.pop_string("option")?;
            set_once(&mut db_path, args.pop_path("--db")?, "--db")?;
            continue;
        }

        if args.peek_is("-h") || args.peek_is("--help") {
            output::print_help()?;
            return Ok(());
        }

        break;
    }

    if args.is_empty() {
        output::print_help()?;
        return Ok(());
    }

    let command = args.pop_string("command")?;
    if command == "help" {
        output::print_help()?;
        return Ok(());
    }

    let mut repository =
        BootstrappedDocumentRepository::open(db_path.as_deref())?.into_repository();
    commands::dispatch(command.as_str(), &mut repository, args)
}
