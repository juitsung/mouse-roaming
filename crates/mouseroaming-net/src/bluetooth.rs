use serde::{Deserialize, Serialize};
use std::net::IpAddr;
use tracing::debug;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BluetoothAdapterInfo {
    pub available: bool,
    pub name: String,
    pub mac: Option<String>,
    pub pan_ip: Option<String>,
    pub status: String,
}

impl Default for BluetoothAdapterInfo {
    fn default() -> Self {
        Self {
            available: false,
            name: "Bluetooth Adapter".into(),
            mac: None,
            pan_ip: None,
            status: "未偵測到藍牙介面".into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BluetoothHostDevice {
    pub name: String,
    pub mac: String,
    pub is_computer: bool,
    pub last_seen_ip: Option<String>,
}

/// Detects local Bluetooth network adapter and Bluetooth PAN IP address.
pub fn detect_bluetooth_adapter() -> BluetoothAdapterInfo {
    let mut info = BluetoothAdapterInfo::default();

    if let Ok(interfaces) = if_addrs::get_if_addrs() {
        for iface in interfaces {
            let name_lower = iface.name.to_lowercase();
            let is_bt = name_lower.contains("bluetooth")
                || name_lower.contains("藍牙")
                || name_lower.contains("藍芽")
                || name_lower.contains("bth")
                || name_lower.contains("pan");

            if is_bt {
                info.available = true;
                info.name = iface.name.clone();
                info.status = "藍牙介面已就緒".into();
                if let IpAddr::V4(ipv4) = iface.addr.ip() {
                    if !ipv4.is_loopback() {
                        info.pan_ip = Some(ipv4.to_string());
                        info.status = "藍牙個人區域網絡 (PAN) 已就緒".into();
                        break;
                    }
                }
            }
        }
    }

    #[cfg(target_os = "windows")]
    {
        // If no PAN IP found yet, check if Windows Generic Bluetooth radio exists
        if !info.available {
            let devs = scan_paired_bluetooth_hosts();
            if !devs.is_empty() {
                info.available = true;
                info.status = "藍牙廣播正常 (已配對裝置已就緒)".into();
            }
        }
    }

    info
}

/// Scans for paired Bluetooth host devices (computers / laptops).
pub fn scan_paired_bluetooth_hosts() -> Vec<BluetoothHostDevice> {
    let mut devices = Vec::new();

    #[cfg(target_os = "windows")]
    {
        scan_windows_bluetooth_devices(&mut devices);
    }

    #[cfg(target_os = "macos")]
    {
        scan_macos_bluetooth_devices(&mut devices);
    }

    // Sort: Computers first, then alphabetically by name
    devices.sort_by(|a, b| {
        b.is_computer
            .cmp(&a.is_computer)
            .then_with(|| a.name.cmp(&b.name))
    });

    devices
}

#[cfg(target_os = "windows")]
fn scan_windows_bluetooth_devices(devices: &mut Vec<BluetoothHostDevice>) {
    use std::ptr::null_mut;
    use windows_sys::Win32::Foundation::ERROR_SUCCESS;
    use windows_sys::Win32::System::Registry::{
        RegCloseKey, RegEnumKeyExW, RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_LOCAL_MACHINE,
        KEY_READ, REG_BINARY, REG_DWORD,
    };

    let subkey: Vec<u16> = "SYSTEM\\CurrentControlSet\\Services\\BTHPORT\\Parameters\\Devices\0"
        .encode_utf16()
        .collect();

    let mut hkey: HKEY = null_mut();
    let res = unsafe {
        RegOpenKeyExW(
            HKEY_LOCAL_MACHINE,
            subkey.as_ptr(),
            0,
            KEY_READ,
            &mut hkey,
        )
    };

    if res != ERROR_SUCCESS {
        debug!("BTHPORT\\Parameters\\Devices key could not be opened (res = {res})");
        return;
    }

    let mut index = 0u32;
    loop {
        let mut key_name = [0u16; 256];
        let mut key_name_len = key_name.len() as u32;

        let enum_res = unsafe {
            RegEnumKeyExW(
                hkey,
                index,
                key_name.as_mut_ptr(),
                &mut key_name_len,
                null_mut(),
                null_mut(),
                null_mut(),
                null_mut(),
            )
        };

        if enum_res != ERROR_SUCCESS {
            break;
        }

        index += 1;

        let raw_mac = String::from_utf16_lossy(&key_name[..key_name_len as usize]);
        let formatted_mac = format_mac_address(&raw_mac);

        // Open device subkey to read Name and COD
        let mut dev_hkey: HKEY = null_mut();
        let dev_subkey: Vec<u16> = format!(
            "SYSTEM\\CurrentControlSet\\Services\\BTHPORT\\Parameters\\Devices\\{}\0",
            raw_mac
        )
        .encode_utf16()
        .collect();

        let open_dev = unsafe {
            RegOpenKeyExW(
                HKEY_LOCAL_MACHINE,
                dev_subkey.as_ptr(),
                0,
                KEY_READ,
                &mut dev_hkey,
            )
        };

        if open_dev == ERROR_SUCCESS {
            let mut name_buf = [0u8; 512];
            let mut name_len = name_buf.len() as u32;
            let mut val_type = 0u32;
            let val_name: Vec<u16> = "Name\0".encode_utf16().collect();

            let name_res = unsafe {
                RegQueryValueExW(
                    dev_hkey,
                    val_name.as_ptr(),
                    null_mut(),
                    &mut val_type,
                    name_buf.as_mut_ptr(),
                    &mut name_len,
                )
            };

            let name = if name_res == ERROR_SUCCESS
                && (val_type == REG_BINARY || val_type == 1 /* REG_SZ */)
            {
                let bytes = &name_buf[..name_len as usize];
                // Strip trailing null bytes
                let trimmed = bytes.split(|&b| b == 0).next().unwrap_or(bytes);
                String::from_utf8_lossy(trimmed).trim().to_string()
            } else {
                format!("Bluetooth Device ({})", formatted_mac)
            };

            // Read COD (Class of Device)
            let mut cod: u32 = 0;
            let mut cod_len = std::mem::size_of::<u32>() as u32;
            let cod_val_name: Vec<u16> = "COD\0".encode_utf16().collect();
            let mut cod_type = 0u32;
            let cod_res = unsafe {
                RegQueryValueExW(
                    dev_hkey,
                    cod_val_name.as_ptr(),
                    null_mut(),
                    &mut cod_type,
                    &mut cod as *mut u32 as *mut u8,
                    &mut cod_len,
                )
            };

            let is_computer = if cod_res == ERROR_SUCCESS && cod_type == REG_DWORD {
                // Major device class is bits 8-12. 0x01 = Computer.
                let major_class = (cod >> 8) & 0x1F;
                major_class == 0x01
                    || name.to_lowercase().contains("mac")
                    || name.to_lowercase().contains("pc")
                    || name.to_lowercase().contains("book")
                    || name.to_lowercase().contains("laptop")
                    || name.to_lowercase().contains("desktop")
            } else {
                name.to_lowercase().contains("mac")
                    || name.to_lowercase().contains("pc")
                    || name.to_lowercase().contains("book")
            };

            unsafe { RegCloseKey(dev_hkey) };

            if !name.is_empty() {
                devices.push(BluetoothHostDevice {
                    name,
                    mac: formatted_mac,
                    is_computer,
                    last_seen_ip: None,
                });
            }
        }
    }

    unsafe { RegCloseKey(hkey) };
}

#[cfg(target_os = "macos")]
fn scan_macos_bluetooth_devices(devices: &mut Vec<BluetoothHostDevice>) {
    // Run system_profiler SPBluetoothDataType to retrieve paired devices
    if let Ok(output) = std::process::Command::new("system_profiler")
        .args(["SPBluetoothDataType", "-json"])
        .output()
    {
        if let Ok(text) = String::from_utf8(output.stdout) {
            parse_macos_bluetooth_json(&text, devices);
        }
    }
}

#[cfg(target_os = "macos")]
fn parse_macos_bluetooth_json(json_text: &str, devices: &mut Vec<BluetoothHostDevice>) {
    if let Ok(val) = serde_json::from_str::<serde_json::Value>(json_text) {
        if let Some(arr) = val.get("SPBluetoothDataType").and_then(|v| v.as_array()) {
            for item in arr {
                if let Some(devices_obj) = item
                    .get("device_connected")
                    .or_else(|| item.get("device_not_connected"))
                {
                    if let Some(dev_map) = devices_obj.as_object() {
                        for (dev_name, dev_val) in dev_map {
                            let mac = dev_val
                                .get("device_address")
                                .and_then(|a| a.as_str())
                                .unwrap_or_default()
                                .to_string();
                            let is_computer = dev_val
                                .get("device_minorType")
                                .and_then(|t| t.as_str())
                                .map(|t| t.to_lowercase().contains("computer") || t.to_lowercase().contains("mac"))
                                .unwrap_or(false)
                                || dev_name.to_lowercase().contains("mac")
                                || dev_name.to_lowercase().contains("pc");

                            devices.push(BluetoothHostDevice {
                                name: dev_name.clone(),
                                mac,
                                is_computer,
                                last_seen_ip: None,
                            });
                        }
                    }
                }
            }
        }
    }
}

/// Helper to format raw hex string (e.g. `f8ffc2609d10`) to standard MAC format `F8:FF:C2:60:9D:10`.
pub fn format_mac_address(raw: &str) -> String {
    let clean: String = raw.chars().filter(|c| c.is_ascii_hexdigit()).collect();
    if clean.len() == 12 {
        let chunks: Vec<String> = clean
            .as_bytes()
            .chunks(2)
            .map(|chunk| std::str::from_utf8(chunk).unwrap_or("").to_ascii_uppercase())
            .collect();
        chunks.join(":")
    } else {
        raw.to_ascii_uppercase()
    }
}

/// Opens system Bluetooth settings panel.
pub fn open_system_bluetooth_settings() {
    #[cfg(target_os = "windows")]
    {
        let _ = std::process::Command::new("explorer.exe")
            .arg("ms-settings:bluetooth")
            .spawn();
    }
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open")
            .arg("x-apple.systempreferences:com.apple.Bluetooth")
            .spawn();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_mac_address_handles_hex_and_colons() {
        assert_eq!(format_mac_address("f8ffc2609d10"), "F8:FF:C2:60:9D:10");
        assert_eq!(format_mac_address("04-7F-0E-9D-C2-E5"), "04:7F:0E:9D:C2:E5");
    }

    #[test]
    fn bluetooth_adapter_detection_runs_without_panic() {
        let adapter = detect_bluetooth_adapter();
        // Adapter info should have meaningful values
        assert!(!adapter.name.is_empty());
    }

    #[test]
    fn bluetooth_paired_hosts_scan_runs_without_panic() {
        let hosts = scan_paired_bluetooth_hosts();
        for h in &hosts {
            assert!(!h.name.is_empty());
            assert!(!h.mac.is_empty());
        }
    }
}
