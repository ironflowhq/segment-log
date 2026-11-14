fn main() -> std::process::ExitCode {
    segment_log_server::main_with(std::env::args().skip(1))
}
