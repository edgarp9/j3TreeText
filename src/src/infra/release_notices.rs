use std::env;
use std::fs;
use std::path::PathBuf;

const ABOUT_TEXT: &str = include_str!("../../about.txt");

pub fn about_text() -> String {
    release_text("about.txt", ABOUT_TEXT)
}

fn release_text(file_name: &str, embedded_text: &str) -> String {
    release_file_candidates(file_name)
        .into_iter()
        .find_map(|path| fs::read_to_string(path).ok())
        .unwrap_or_else(|| embedded_text.to_owned())
}

fn release_file_candidates(file_name: &str) -> Vec<PathBuf> {
    match env::current_exe().ok().and_then(|executable| {
        executable
            .parent()
            .map(|directory| directory.join(file_name))
    }) {
        Some(path) => vec![path],
        None => Vec::new(),
    }
}
