use magnarr::graphql::schema::build_schema_sdl;
use std::{fs, path::Path};

fn main() {
    let sdl = build_schema_sdl();
    let path = Path::new("../../schema.graphql");
    fs::write(path, sdl).expect("failed to write schema.graphql");
    println!("schema.graphql written to {}", path.display());
}
