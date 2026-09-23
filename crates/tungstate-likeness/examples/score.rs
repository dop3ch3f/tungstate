//! `cargo run -p tungstate-likeness --example score -- a b c`
//!
//! Prints how alike every pair of the given files is. This is how the
//! thresholds in `tungstate_core::dupes` were chosen, and it is how they get
//! re-checked when the arithmetic changes, against real files rather than the
//! drawn ones the tests use.

use std::path::PathBuf;

use tungstate_likeness::{alike, kind_of, look};

fn main() {
    let files: Vec<PathBuf> = std::env::args().skip(1).map(PathBuf::from).collect();
    let mut prints = Vec::new();
    for path in &files {
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        let Some(kind) = kind_of(None, &name) else {
            println!("{name}: not a kind this looks at");
            continue;
        };
        match look(path, kind, false) {
            Ok(shot) => {
                println!(
                    "{name}: {} signature {:016x}",
                    shot.print.algo, shot.print.signature
                );
                prints.push((name.to_string(), shot.print));
            }
            Err(trouble) => println!("{name}: {trouble}"),
        }
    }
    println!();
    for (index, (name, print)) in prints.iter().enumerate() {
        for (other_name, other) in prints.iter().skip(index + 1) {
            let score = alike(print, other)
                .map_or("not comparable".to_string(), |it| format!("{it}% alike"));
            let bits = tungstate_likeness::apart(print.signature, other.signature);
            println!("{name} vs {other_name}: {score}, signatures {bits} bits apart");
        }
    }
}
