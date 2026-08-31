//! Foreground entry point for the optional hardened user service.

fn main() {
    match surfacecheck_service::RuntimeService::from_environment()
        .and_then(surfacecheck_service::run_foreground)
    {
        Ok(()) => {}
        Err(error) => {
            // Keep diagnostics bounded and free of request data.  The service
            // protocol itself never exposes this text to a client.
            eprintln!("surfacecheckd: {error}");
            std::process::exit(1);
        }
    }
}
