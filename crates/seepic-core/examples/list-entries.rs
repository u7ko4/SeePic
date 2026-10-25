//! Print the display order a container produces, for diagnosing sort and
//! archive-listing questions from the command line.
//!
//! `cargo run -p seepic-core --example list-entries -- <file-or-folder>`

fn main() {
    let Some(path) = std::env::args_os().nth(1).map(std::path::PathBuf::from) else {
        eprintln!("usage: list-entries <file-or-folder>");
        return;
    };

    let container = match seepic_core::Container::open(&path) {
        Ok(container) => container,
        Err(error) => {
            eprintln!("open failed: {error}");
            return;
        }
    };

    let mut navigator = seepic_core::Navigator::new();
    navigator.set_entries(container.entries().to_vec());

    println!("{} entries, display order:", navigator.len());
    for position in 0..navigator.len() {
        let Some(entry) = navigator.entries().get(position) else {
            continue;
        };
        println!(
            "{position:>3}  source {:>3}  {}",
            navigator.source_index(position).unwrap_or(usize::MAX),
            entry.name
        );
    }
}
