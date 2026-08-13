#![cfg_attr(not(target_os = "windows"), allow(dead_code))]
#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

#[cfg(not(target_os = "windows"))]
compile_error!("MonitorDDC only supports Windows.");

mod cli;
mod ddc;
mod display_info;
mod gui;

use clap::Parser;

fn main() {
    let args = cli::Args::parse();

    if args.requests_cli() {
        if let Err(error) = cli::run(&args) {
            eprintln!("错误：{error}");
            std::process::exit(1);
        }
    } else if let Err(error) = gui::run() {
        eprintln!("无法启动图形界面：{error}");
        std::process::exit(1);
    }
}
