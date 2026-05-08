pub fn run() {
    let version = env!("CARGO_PKG_VERSION");
    let git_hash = env!("MAGNARR_GIT_HASH");
    let build_ts: i64 = env!("MAGNARR_BUILD_TIMESTAMP").parse().unwrap_or(0);
    let build_date = chrono::DateTime::from_timestamp(build_ts, 0)
        .map(|dt| dt.format("%Y-%m-%d %H:%M:%S UTC").to_string())
        .unwrap_or_else(|| "unknown".to_owned());

    println!("magnarr {version}");
    println!("Commit:  {git_hash}");
    println!("Built:   {build_date}");
}
