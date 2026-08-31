fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (bytes, code) = surfacecheck_cli::run(&args);
    use std::io::Write;
    let mut stdout = std::io::stdout().lock();
    let _ = stdout.write_all(&bytes);
    let _ = stdout.write_all(b"\n");
    std::process::exit(code);
}
