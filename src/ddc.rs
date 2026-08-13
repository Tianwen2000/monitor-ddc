//! Windows MCCS/DDC-CI access and physical monitor handle ownership.

use std::error::Error;
use std::fmt::{Display, Formatter};

use windows::Win32::Devices::Display::{
    DestroyPhysicalMonitor, GetNumberOfPhysicalMonitorsFromHMONITOR,
    GetPhysicalMonitorsFromHMONITOR, GetVCPFeatureAndVCPFeatureReply, PHYSICAL_MONITOR,
    SetVCPFeature,
};
use windows::Win32::Foundation::{LPARAM, RECT};
use windows::Win32::Graphics::Gdi::{EnumDisplayMonitors, HDC, HMONITOR};
use windows::core::{BOOL, Error as WindowsError};

use crate::display_info::{self, DisplayInfo};

const VCP_BRIGHTNESS: u8 = 0x10;
const VCP_CONTRAST: u8 = 0x12;

#[derive(Debug)]
pub enum DdcError {
    Windows {
        operation: &'static str,
        source: WindowsError,
    },
    Unsupported {
        feature: &'static str,
        details: String,
    },
    NoMonitors,
}

impl DdcError {
    fn windows(operation: &'static str, source: WindowsError) -> Self {
        Self::Windows { operation, source }
    }
}

impl Display for DdcError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Windows { operation, source } => {
                write!(formatter, "{operation}失败：{source}")
            }
            Self::Unsupported { feature, details } => {
                write!(
                    formatter,
                    "显示器不支持通过 DDC/CI 读取{feature}：{details}"
                )
            }
            Self::NoMonitors => write!(formatter, "未检测到物理显示器"),
        }
    }
}

impl Error for DdcError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Windows { source, .. } => Some(source),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Feature {
    Brightness,
    Contrast,
}

impl Feature {
    fn vcp_code(self) -> u8 {
        match self {
            Self::Brightness => VCP_BRIGHTNESS,
            Self::Contrast => VCP_CONTRAST,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Brightness => "亮度",
            Self::Contrast => "对比度",
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct FeatureValue {
    pub current: u32,
    pub maximum: u32,
}

impl FeatureValue {
    pub fn percent(self) -> u8 {
        raw_to_percent(self.current, self.maximum)
    }
}

/// Owns a Windows physical-monitor handle. Dropping it always closes the handle.
pub struct Monitor {
    pub index: usize,
    pub description: String,
    pub info: DisplayInfo,
    handle: windows::Win32::Foundation::HANDLE,
}

impl Monitor {
    pub fn identity_name(&self) -> String {
        self.info.identity_name().unwrap_or_else(|| {
            if self.description.is_empty() {
                format!("显示器 {}", self.index)
            } else {
                self.localized_description().to_owned()
            }
        })
    }

    pub fn localized_description(&self) -> &str {
        match self.description.as_str() {
            "Generic PnP Monitor" => "通用即插即用显示器",
            description => description,
        }
    }

    pub fn read(&self, feature: Feature) -> Result<FeatureValue, DdcError> {
        let mut current = 0_u32;
        let mut maximum = 0_u32;

        // The optional MCCS type is not needed; current and maximum are sufficient.
        let succeeded = unsafe {
            GetVCPFeatureAndVCPFeatureReply(
                self.handle,
                feature.vcp_code(),
                None,
                &mut current,
                Some(&mut maximum),
            )
        };
        check_ddc_result(succeeded, "读取 VCP 参数")?;

        if maximum == 0 {
            return Err(DdcError::Unsupported {
                feature: feature.label(),
                details: "显示器返回的最大值为 0".to_owned(),
            });
        }

        Ok(FeatureValue { current, maximum })
    }

    pub fn write_percent(&self, feature: Feature, percent: u8) -> Result<(), DdcError> {
        // Some monitors accept SetVCPFeature but reject GetVCPFeature. In that case,
        // use the common MCCS 0-100 range so write-only implementations still work.
        let maximum = self.read(feature).map_or(100, |range| range.maximum);
        self.write_percent_with_max(feature, percent, maximum)
    }

    pub fn write_percent_with_max(
        &self,
        feature: Feature,
        percent: u8,
        maximum: u32,
    ) -> Result<(), DdcError> {
        let raw_value = percent_to_raw(percent, maximum.max(1));

        let succeeded = unsafe { SetVCPFeature(self.handle, feature.vcp_code(), raw_value) };
        check_ddc_result(succeeded, "写入 VCP 参数")
    }
}

impl Drop for Monitor {
    fn drop(&mut self) {
        // Failure during cleanup cannot be usefully reported from Drop.
        let _ = unsafe { DestroyPhysicalMonitor(self.handle) };
    }
}

/// Enumerates Windows logical monitors, then expands each into its physical monitors.
pub fn enumerate_monitors() -> Result<Vec<Monitor>, DdcError> {
    let mut logical_monitors = Vec::<HMONITOR>::new();

    unsafe extern "system" fn callback(
        monitor: HMONITOR,
        _dc: HDC,
        _rect: *mut RECT,
        data: LPARAM,
    ) -> BOOL {
        let monitors = unsafe { &mut *(data.0 as *mut Vec<HMONITOR>) };
        monitors.push(monitor);
        BOOL::from(true)
    }

    let data = LPARAM((&mut logical_monitors as *mut Vec<HMONITOR>) as isize);
    unsafe { EnumDisplayMonitors(None, None, Some(callback), data) }
        .ok()
        .map_err(|error| DdcError::windows("枚举逻辑显示器", error))?;

    // DisplayConfig identity data is supplemental. DDC enumeration remains usable even
    // when a display driver does not expose names or active-mode information.
    let display_map = display_info::query_active_displays().unwrap_or_default();
    let mut monitors = Vec::new();
    for logical_monitor in logical_monitors {
        let display_info = display_info::identify_monitor(logical_monitor, &display_map);
        let mut count = 0_u32;
        unsafe { GetNumberOfPhysicalMonitorsFromHMONITOR(logical_monitor, &mut count) }
            .map_err(|error| DdcError::windows("获取物理显示器数量", error))?;

        if count == 0 {
            continue;
        }

        let mut physical = vec![PHYSICAL_MONITOR::default(); count as usize];
        if let Err(error) =
            unsafe { GetPhysicalMonitorsFromHMONITOR(logical_monitor, &mut physical) }
        {
            return Err(DdcError::windows("打开物理显示器", error));
        }

        for native in physical {
            // PHYSICAL_MONITOR is packed; copy the UTF-16 array before borrowing it.
            let description_buffer =
                unsafe { std::ptr::addr_of!(native.szPhysicalMonitorDescription).read_unaligned() };
            let description_length = description_buffer
                .iter()
                .position(|character| *character == 0)
                .unwrap_or(description_buffer.len());
            let description = String::from_utf16_lossy(&description_buffer[..description_length]);
            monitors.push(Monitor {
                index: monitors.len(),
                description,
                info: display_info.clone(),
                handle: native.hPhysicalMonitor,
            });
        }
    }

    if monitors.is_empty() {
        Err(DdcError::NoMonitors)
    } else {
        Ok(monitors)
    }
}

fn check_ddc_result(result: i32, operation: &'static str) -> Result<(), DdcError> {
    if result != 0 {
        Ok(())
    } else {
        Err(DdcError::windows(operation, WindowsError::from_thread()))
    }
}

fn raw_to_percent(current: u32, maximum: u32) -> u8 {
    if maximum == 0 {
        return 0;
    }

    (((current.min(maximum) as u64 * 100) + maximum as u64 / 2) / maximum as u64) as u8
}

fn percent_to_raw(percent: u8, maximum: u32) -> u32 {
    (((percent.min(100) as u64 * maximum as u64) + 50) / 100) as u32
}

#[cfg(test)]
mod tests {
    use super::{percent_to_raw, raw_to_percent};

    #[test]
    fn converts_non_standard_monitor_ranges() {
        assert_eq!(raw_to_percent(127, 255), 50);
        assert_eq!(percent_to_raw(50, 255), 128);
    }

    #[test]
    fn clamps_values_to_valid_percent_range() {
        assert_eq!(raw_to_percent(200, 100), 100);
        assert_eq!(raw_to_percent(1, 0), 0);
        assert_eq!(percent_to_raw(120, 255), 255);
    }
}
