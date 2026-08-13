//! Command-line parsing and terminal-mode execution.

use clap::{ArgAction, Parser};

use crate::ddc::{self, Feature, Monitor};

#[derive(Debug, Parser)]
#[command(
    name = "MonitorDDC",
    version,
    about = "通过 Windows DDC/CI 调节外接显示器的亮度和对比度",
    after_help = "不带参数时启动图形界面。所有数值均为 0 到 100 的百分比。"
)]
pub struct Args {
    /// 将亮度设置为 0 到 100 的百分比。
    #[arg(long, value_name = "PERCENT", value_parser = clap::value_parser!(u8).range(0..=100), conflicts_with = "gui")]
    pub brightness: Option<u8>,

    /// 将对比度设置为 0 到 100 的百分比。
    #[arg(long, value_name = "PERCENT", value_parser = clap::value_parser!(u8).range(0..=100), conflicts_with = "gui")]
    pub contrast: Option<u8>,

    /// 读取当前亮度。
    #[arg(long, action = ArgAction::SetTrue, conflicts_with = "gui")]
    pub get_brightness: bool,

    /// 读取当前对比度。
    #[arg(long, action = ArgAction::SetTrue, conflicts_with = "gui")]
    pub get_contrast: bool,

    /// 列出所有物理显示器及其 DDC/CI 状态。
    #[arg(long, action = ArgAction::SetTrue, conflicts_with = "gui")]
    pub list: bool,

    /// 仅操作 --list 显示的指定显示器序号（从 0 开始）。
    #[arg(long, value_name = "INDEX", conflicts_with = "gui")]
    pub monitor: Option<usize>,

    /// 明确启动图形界面。
    #[arg(long, action = ArgAction::SetTrue)]
    pub gui: bool,
}

impl Args {
    pub fn requests_cli(&self) -> bool {
        !self.gui
            && (self.brightness.is_some()
                || self.contrast.is_some()
                || self.get_brightness
                || self.get_contrast
                || self.list
                || self.monitor.is_some())
    }
}

pub fn run(args: &Args) -> Result<(), String> {
    let monitors = ddc::enumerate_monitors().map_err(|error| error.to_string())?;
    let selected = select_monitors(&monitors, args.monitor)?;

    let should_read_brightness =
        args.get_brightness || (args.list && args.brightness.is_none() && args.contrast.is_none());
    let should_read_contrast =
        args.get_contrast || (args.list && args.brightness.is_none() && args.contrast.is_none());

    let mut failures = Vec::new();
    for monitor in selected {
        println!("[{}] {}", monitor.index, monitor.identity_name());
        if args.list {
            report_display_info(monitor);
        }

        if let Some(percent) = args.brightness {
            report_write(monitor, Feature::Brightness, percent, &mut failures);
        }
        if let Some(percent) = args.contrast {
            report_write(monitor, Feature::Contrast, percent, &mut failures);
        }
        if should_read_brightness {
            report_read(monitor, Feature::Brightness, &mut failures);
        }
        if should_read_contrast {
            report_read(monitor, Feature::Contrast, &mut failures);
        }
    }

    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures.join("; "))
    }
}

fn report_display_info(monitor: &Monitor) {
    let info = &monitor.info;
    println!("  当前模式: {}", info.mode_text());
    println!("  连接接口: {}", info.connector_text());
    println!(
        "  制造商代码: {}",
        info.manufacturer_code.as_deref().unwrap_or("未知")
    );
    println!("  产品代码: {}", info.product_code_text());
    if let Some(bit_depth) = info.bit_depth {
        println!("  桌面色深: {bit_depth} 位");
    }
    if !info.gdi_device_name.is_empty() {
        println!("  Windows 显示设备: {}", info.gdi_device_name);
    }
    if let Some(path) = &info.device_path {
        println!("  设备路径: {path}");
    }
    if info.is_primary {
        println!("  主显示器: 是");
    }
}

fn select_monitors(monitors: &[Monitor], index: Option<usize>) -> Result<Vec<&Monitor>, String> {
    match index {
        Some(index) => monitors
            .iter()
            .find(|monitor| monitor.index == index)
            .map(|monitor| vec![monitor])
            .ok_or_else(|| format!("未找到序号为 {index} 的显示器，请先运行 --list")),
        None => Ok(monitors.iter().collect()),
    }
}

fn report_read(monitor: &Monitor, feature: Feature, failures: &mut Vec<String>) {
    match monitor.read(feature) {
        Ok(value) => println!(
            "  {}: {}% (raw {}/{})",
            feature.label(),
            value.percent(),
            value.current,
            value.maximum
        ),
        Err(error) => {
            let message = format!(
                "显示器 {} 的{}读取失败：{error}",
                monitor.index,
                feature.label()
            );
            eprintln!("  {message}");
            failures.push(message);
        }
    }
}

fn report_write(monitor: &Monitor, feature: Feature, percent: u8, failures: &mut Vec<String>) {
    match monitor.write_percent(feature, percent) {
        Ok(()) => println!("  {}已设置为 {}%", feature.label(), percent),
        Err(error) => {
            let message = format!(
                "显示器 {} 的{}写入失败：{error}",
                monitor.index,
                feature.label()
            );
            eprintln!("  {message}");
            failures.push(message);
        }
    }
}
