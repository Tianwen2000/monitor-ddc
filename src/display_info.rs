//! Read-only Windows display identity and active-mode discovery.

use std::collections::HashMap;
use std::mem::size_of;

use windows::Win32::Devices::Display::{
    DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME, DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME,
    DISPLAYCONFIG_DEVICE_INFO_HEADER, DISPLAYCONFIG_MODE_INFO, DISPLAYCONFIG_PATH_INFO,
    DISPLAYCONFIG_SOURCE_DEVICE_NAME, DISPLAYCONFIG_TARGET_DEVICE_NAME,
    DISPLAYCONFIG_VIDEO_OUTPUT_TECHNOLOGY, DisplayConfigGetDeviceInfo, GetDisplayConfigBufferSizes,
    QDC_ONLY_ACTIVE_PATHS, QueryDisplayConfig,
};
use windows::Win32::Foundation::{ERROR_INSUFFICIENT_BUFFER, ERROR_SUCCESS, WIN32_ERROR};
use windows::Win32::Graphics::Gdi::{
    DEVMODEW, ENUM_CURRENT_SETTINGS, EnumDisplaySettingsW, GetMonitorInfoW, HMONITOR,
    MONITORINFOEXW,
};
use windows::core::PCWSTR;

const QUERY_RETRIES: usize = 3;
const MONITORINFOF_PRIMARY: u32 = 1;

#[derive(Clone, Debug, Default)]
pub struct DisplayInfo {
    pub gdi_device_name: String,
    pub friendly_name: Option<String>,
    pub manufacturer_code: Option<String>,
    pub product_code: Option<u16>,
    pub device_path: Option<String>,
    pub resolution: Option<(u32, u32)>,
    pub refresh_rate_hz: Option<f64>,
    pub bit_depth: Option<u32>,
    pub connector: Option<String>,
    pub is_primary: bool,
}

impl DisplayInfo {
    pub fn manufacturer_name(&self) -> Option<&str> {
        self.manufacturer_code
            .as_deref()
            .map(localized_manufacturer)
    }

    pub fn identity_name(&self) -> Option<String> {
        let manufacturer = self.manufacturer_name();
        match (manufacturer, self.friendly_name.as_deref()) {
            (Some(brand), Some(model)) if !model_mentions_brand(model, brand) => {
                Some(format!("{brand} {model}"))
            }
            (_, Some(model)) => Some(model.to_owned()),
            (Some(brand), None) => Some(brand.to_owned()),
            (None, None) => None,
        }
    }

    pub fn mode_text(&self) -> String {
        match (self.resolution, self.refresh_rate_hz) {
            (Some((width, height)), Some(refresh)) => {
                format!("{width} × {height} @ {} Hz", format_refresh_rate(refresh))
            }
            (Some((width, height)), None) => format!("{width} × {height}"),
            (None, Some(refresh)) => format!("{} Hz", format_refresh_rate(refresh)),
            (None, None) => "未知".to_owned(),
        }
    }

    pub fn product_code_text(&self) -> String {
        self.product_code
            .map(|code| format!("0x{code:04X}"))
            .unwrap_or_else(|| "未知".to_owned())
    }

    pub fn connector_text(&self) -> &str {
        self.connector.as_deref().unwrap_or("未知接口")
    }
}

/// Returns active displays keyed by their normalized GDI name (for example, `\\.\DISPLAY1`).
pub fn query_active_displays() -> Result<HashMap<String, DisplayInfo>, String> {
    let (paths, _modes) = query_display_config()?;
    let mut displays = HashMap::new();

    for path in paths {
        let Some(gdi_name) = query_source_name(&path) else {
            continue;
        };

        let target = query_target_name(&path);
        let current_mode = query_current_mode(&gdi_name);
        let refresh_rate_hz = rational_to_hz(
            path.targetInfo.refreshRate.Numerator,
            path.targetInfo.refreshRate.Denominator,
        )
        .or_else(|| current_mode.map(|mode| mode.2 as f64));

        let mut info = DisplayInfo {
            gdi_device_name: gdi_name.clone(),
            resolution: current_mode.map(|mode| (mode.0, mode.1)),
            refresh_rate_hz,
            bit_depth: current_mode.map(|mode| mode.3),
            connector: Some(connector_name(path.targetInfo.outputTechnology).to_owned()),
            ..Default::default()
        };

        if let Some(target) = target {
            let friendly_name = utf16_string(&target.monitorFriendlyDeviceName);
            let device_path = utf16_string(&target.monitorDevicePath);
            info.manufacturer_code = manufacturer_from_device_path(&device_path)
                .or_else(|| decode_edid_manufacturer(target.edidManufactureId));
            info.friendly_name = (!friendly_name.is_empty()).then_some(friendly_name);
            info.device_path = (!device_path.is_empty()).then_some(device_path);
            info.product_code = (target.edidProductCodeId != 0).then_some(target.edidProductCodeId);
            info.connector = Some(connector_name(target.outputTechnology).to_owned());
        }

        displays
            .entry(normalized_gdi_name(&gdi_name))
            .or_insert(info);
    }

    Ok(displays)
}

/// Matches an HMONITOR to DisplayConfig metadata and supplements it with GDI state.
pub fn identify_monitor(monitor: HMONITOR, displays: &HashMap<String, DisplayInfo>) -> DisplayInfo {
    let Some((gdi_name, is_primary)) = monitor_gdi_name(monitor) else {
        return DisplayInfo::default();
    };

    let mut info = displays
        .get(&normalized_gdi_name(&gdi_name))
        .cloned()
        .unwrap_or_default();
    info.gdi_device_name = gdi_name.clone();
    info.is_primary = is_primary;

    if let Some((width, height, refresh, bit_depth)) = query_current_mode(&gdi_name) {
        info.resolution.get_or_insert((width, height));
        info.refresh_rate_hz.get_or_insert(refresh as f64);
        info.bit_depth.get_or_insert(bit_depth);
    }

    info
}

fn query_display_config()
-> Result<(Vec<DISPLAYCONFIG_PATH_INFO>, Vec<DISPLAYCONFIG_MODE_INFO>), String> {
    for _ in 0..QUERY_RETRIES {
        let mut path_count = 0_u32;
        let mut mode_count = 0_u32;
        let result = unsafe {
            GetDisplayConfigBufferSizes(QDC_ONLY_ACTIVE_PATHS, &mut path_count, &mut mode_count)
        };
        check_win32(result, "获取显示配置缓冲区大小")?;

        let mut paths = vec![DISPLAYCONFIG_PATH_INFO::default(); path_count as usize];
        let mut modes = vec![DISPLAYCONFIG_MODE_INFO::default(); mode_count as usize];
        let result = unsafe {
            QueryDisplayConfig(
                QDC_ONLY_ACTIVE_PATHS,
                &mut path_count,
                paths.as_mut_ptr(),
                &mut mode_count,
                modes.as_mut_ptr(),
                None,
            )
        };

        if result == ERROR_INSUFFICIENT_BUFFER {
            continue;
        }
        check_win32(result, "读取活动显示配置")?;
        paths.truncate(path_count as usize);
        modes.truncate(mode_count as usize);
        return Ok((paths, modes));
    }

    Err("读取活动显示配置失败：显示器连接状态持续变化".to_owned())
}

fn query_source_name(path: &DISPLAYCONFIG_PATH_INFO) -> Option<String> {
    let mut packet = DISPLAYCONFIG_SOURCE_DEVICE_NAME {
        header: DISPLAYCONFIG_DEVICE_INFO_HEADER {
            r#type: DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME,
            size: size_of::<DISPLAYCONFIG_SOURCE_DEVICE_NAME>() as u32,
            adapterId: path.sourceInfo.adapterId,
            id: path.sourceInfo.id,
        },
        ..Default::default()
    };

    let result = unsafe { DisplayConfigGetDeviceInfo(&mut packet.header) };
    (result == 0)
        .then(|| utf16_string(&packet.viewGdiDeviceName))
        .filter(|name| !name.is_empty())
}

fn query_target_name(path: &DISPLAYCONFIG_PATH_INFO) -> Option<DISPLAYCONFIG_TARGET_DEVICE_NAME> {
    let mut packet = DISPLAYCONFIG_TARGET_DEVICE_NAME {
        header: DISPLAYCONFIG_DEVICE_INFO_HEADER {
            r#type: DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME,
            size: size_of::<DISPLAYCONFIG_TARGET_DEVICE_NAME>() as u32,
            adapterId: path.targetInfo.adapterId,
            id: path.targetInfo.id,
        },
        ..Default::default()
    };

    let result = unsafe { DisplayConfigGetDeviceInfo(&mut packet.header) };
    (result == 0).then_some(packet)
}

fn monitor_gdi_name(monitor: HMONITOR) -> Option<(String, bool)> {
    let mut info = MONITORINFOEXW::default();
    info.monitorInfo.cbSize = size_of::<MONITORINFOEXW>() as u32;
    let succeeded = unsafe { GetMonitorInfoW(monitor, &mut info.monitorInfo) };
    if !succeeded.as_bool() {
        return None;
    }

    let name = utf16_string(&info.szDevice);
    (!name.is_empty()).then_some((name, info.monitorInfo.dwFlags & MONITORINFOF_PRIMARY != 0))
}

fn query_current_mode(gdi_name: &str) -> Option<(u32, u32, u32, u32)> {
    let wide_name: Vec<u16> = gdi_name.encode_utf16().chain([0]).collect();
    let mut mode = DEVMODEW {
        dmSize: size_of::<DEVMODEW>() as u16,
        ..Default::default()
    };
    let succeeded = unsafe {
        EnumDisplaySettingsW(PCWSTR(wide_name.as_ptr()), ENUM_CURRENT_SETTINGS, &mut mode)
    };

    succeeded.as_bool().then_some((
        mode.dmPelsWidth,
        mode.dmPelsHeight,
        mode.dmDisplayFrequency,
        mode.dmBitsPerPel,
    ))
}

fn check_win32(result: WIN32_ERROR, operation: &str) -> Result<(), String> {
    if result == ERROR_SUCCESS {
        Ok(())
    } else {
        Err(format!("{operation}失败（Windows 错误 {}）", result.0))
    }
}

fn utf16_string(buffer: &[u16]) -> String {
    let length = buffer
        .iter()
        .position(|character| *character == 0)
        .unwrap_or(buffer.len());
    String::from_utf16_lossy(&buffer[..length])
        .trim()
        .to_owned()
}

fn normalized_gdi_name(name: &str) -> String {
    name.trim().to_ascii_uppercase()
}

fn rational_to_hz(numerator: u32, denominator: u32) -> Option<f64> {
    (numerator != 0 && denominator != 0).then_some(numerator as f64 / denominator as f64)
}

fn manufacturer_from_device_path(path: &str) -> Option<String> {
    let uppercase = path.to_ascii_uppercase();
    let marker = "DISPLAY#";
    let start = uppercase.find(marker)? + marker.len();
    let hardware_id = uppercase[start..].split('#').next()?;
    let code = hardware_id.get(..3)?;
    code.bytes()
        .all(|character| character.is_ascii_uppercase())
        .then(|| code.to_owned())
}

fn decode_edid_manufacturer(raw: u16) -> Option<String> {
    // EDID stores this 15-bit EISA code in big-endian order; Windows exposes the two bytes
    // as a native u16, so swap before extracting the three five-bit letters.
    decode_eisa_id(raw.swap_bytes()).or_else(|| decode_eisa_id(raw))
}

fn decode_eisa_id(value: u16) -> Option<String> {
    let values = [(value >> 10) & 0x1f, (value >> 5) & 0x1f, value & 0x1f];
    if !values.iter().all(|value| (1..=26).contains(value)) {
        return None;
    }

    Some(
        values
            .into_iter()
            .map(|value| char::from(b'A' + value as u8 - 1))
            .collect(),
    )
}

fn localized_manufacturer(code: &str) -> &str {
    match code {
        "HKC" => "惠科（HKC）",
        "DEL" => "戴尔（Dell）",
        "ACI" | "AUS" => "华硕（ASUS）",
        "GSM" => "LG",
        "SAM" => "三星（Samsung）",
        "AOC" => "AOC",
        "BNQ" => "明基（BenQ）",
        "ACR" => "宏碁（Acer）",
        "LEN" => "联想（Lenovo）",
        "HWP" | "HPN" => "惠普（HP）",
        "APP" => "苹果（Apple）",
        "SNY" => "索尼（Sony）",
        "PHL" | "PHI" => "飞利浦（Philips）",
        "MSI" => "微星（MSI）",
        "GBT" => "技嘉（Gigabyte）",
        "VSC" => "优派（ViewSonic）",
        _ => code,
    }
}

fn model_mentions_brand(model: &str, brand: &str) -> bool {
    let model = model.to_ascii_lowercase();
    let brand = brand.to_ascii_lowercase();
    let aliases = [
        "hkc",
        "dell",
        "asus",
        "lg",
        "samsung",
        "aoc",
        "benq",
        "acer",
        "lenovo",
        "hp",
        "apple",
        "sony",
        "philips",
        "msi",
        "gigabyte",
        "viewsonic",
    ];

    model.contains(&brand)
        || aliases
            .iter()
            .any(|alias| brand.contains(alias) && model.contains(alias))
}

fn connector_name(technology: DISPLAYCONFIG_VIDEO_OUTPUT_TECHNOLOGY) -> &'static str {
    match technology.0 {
        0 => "VGA",
        4 => "DVI",
        5 => "HDMI",
        10 => "DisplayPort",
        18 => "USB-C（DisplayPort）",
        6 | 11 | 13 | i32::MIN => "内置显示屏",
        15 => "无线显示",
        16 => "USB 显示设备",
        17 => "虚拟显示器",
        -1 => "其他接口",
        _ => "其他接口",
    }
}

fn format_refresh_rate(refresh: f64) -> String {
    let rounded = refresh.round();
    if (refresh - rounded).abs() < 0.05 {
        format!("{rounded:.0}")
    } else {
        format!("{refresh:.2}")
    }
}

#[cfg(test)]
mod tests {
    use super::{decode_edid_manufacturer, manufacturer_from_device_path};

    #[test]
    fn decodes_big_endian_edid_manufacturer() {
        assert_eq!(decode_edid_manufacturer(0x6321).as_deref(), Some("HKC"));
    }

    #[test]
    fn gets_manufacturer_from_monitor_device_path() {
        let path = r"\\?\DISPLAY#HKC2416#5&123456&0&UID4352";
        assert_eq!(manufacturer_from_device_path(path).as_deref(), Some("HKC"));
    }
}
