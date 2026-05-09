use magnarr::graphql::schema::build_schema_sdl;
use std::{fs, path::PathBuf};

fn main() {
    let path: PathBuf = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "schema.graphql".to_string())
        .into();

    let sdl = build_schema_sdl();
    fs::write(&path, sdl).expect("failed to write schema");
    println!("schema written to {}", path.display());
}
