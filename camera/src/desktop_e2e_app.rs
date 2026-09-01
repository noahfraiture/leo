fn main() {
    let path = std::env::args_os()
        .nth(1)
        .map(std::path::PathBuf::from)
        .expect("desktop E2E requires one settings path");
    app::launch_desktop_e2e(path);
}
