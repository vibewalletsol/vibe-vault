mod app;
mod cli;
mod ctx;
mod dialogs;
mod donut;
mod fonts;
mod holdings;
mod pages;
mod paths;
mod qr;
mod task;
mod theme;
mod window;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Some(code) = cli::run(&args) {
        std::process::exit(code);
    }
    std::process::exit(app::run().into());
}
