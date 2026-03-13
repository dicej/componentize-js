fn main() -> anyhow::Result<()> {
    componentize_js::command::run(std::env::args_os())
}
