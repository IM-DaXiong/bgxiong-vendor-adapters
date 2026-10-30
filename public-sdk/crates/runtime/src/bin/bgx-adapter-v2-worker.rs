fn main() {
    let mut arguments = std::env::args_os().skip(1);
    let root = match (arguments.next(), arguments.next(), arguments.next()) {
        (Some(flag), Some(path), None) if flag == "--package" => std::path::PathBuf::from(path),
        _ => {
            eprintln!("Usage: bgx-adapter-v2-worker --package <explicit-sdk-package-root>");
            std::process::exit(2)
        }
    };
    if let Err(error) = bgx_adapter_v2_runtime::server::run_stdio(&root) {
        eprintln!("SDK worker failed: {error}");
        std::process::exit(1);
    }
}
