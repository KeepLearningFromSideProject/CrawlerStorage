#[cfg(not(target_os = "macos"))]
const LIBFUSE_NAME: &str = "fuse3";

#[cfg(target_os = "macos")]
const LIBFUSE_NAME: &str = "osxfuse";

fn main() {
    pkg_config::Config::new()
        .atleast_version("3.0.0")
        .probe(LIBFUSE_NAME)
        .map_err(|e| eprintln!("{}", e))
        .unwrap();
}
