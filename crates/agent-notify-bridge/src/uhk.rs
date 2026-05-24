#[cfg(windows)]
use agent_notify_core::uhk_exec_macro_report;
#[cfg(windows)]
use anyhow::Context;

pub struct DisplayAdapter {
    mock: bool,
}

impl DisplayAdapter {
    pub fn new(mock: bool) -> Self {
        Self { mock }
    }

    pub fn keyboard_present(&self) -> bool {
        if self.mock {
            return true;
        }

        platform::keyboard_present()
    }

    pub fn display_macro_command(&self, command: &str) -> anyhow::Result<()> {
        if self.mock {
            tracing::info!(%command, "mock UHK display");
            return Ok(());
        }

        platform::display_macro_command(command)
    }
}

#[cfg(windows)]
mod platform {
    use super::*;
    use hidapi::{DeviceInfo, HidApi};

    const UHK_VENDOR_ID: u16 = 0x37a8;
    const UHK80_RIGHT_PID: u16 = 0x0009;
    const UHK80_REPORT_ID: u8 = 4;

    pub fn keyboard_present() -> bool {
        HidApi::new()
            .map(|api| api.device_list().any(is_uhk80_device))
            .unwrap_or(false)
    }

    pub fn display_macro_command(command: &str) -> anyhow::Result<()> {
        let api = HidApi::new().context("failed to initialize HID API")?;
        let devices = api
            .device_list()
            .filter(|device| is_uhk80_device(device))
            .collect::<Vec<_>>();
        let device_info = devices
            .iter()
            .copied()
            .find(|device| is_uhk80_communication_interface(device))
            .or_else(|| {
                let device = devices.first().copied();
                if let Some(device) = device {
                    tracing::warn!(
                        usage_page = device.usage_page(),
                        usage = device.usage(),
                        "UHK80 communication interface usage was not found; trying visible UHK80 HID interface"
                    );
                }
                device
            })
            .with_context(|| {
                "UHK80 HID device not found; the KVM may not be passing through the UHK vendor HID interface"
            })?;
        let device = device_info
            .open_device(&api)
            .context("failed to open UHK80")?;
        let report = uhk_exec_macro_report(UHK80_REPORT_ID, command)?;

        device
            .write(&report)
            .context("failed to write UHK80 HID report")?;

        let mut response = [0_u8; 64];
        let size = device
            .read_timeout(&mut response, 1000)
            .context("failed to read UHK80 HID response")?;
        if size == 0 {
            anyhow::bail!("UHK80 did not respond before timeout");
        }

        let status = if response[0] == UHK80_REPORT_ID {
            response[1]
        } else {
            response[0]
        };
        if status != 0 {
            anyhow::bail!("UHK80 returned communication status {status}");
        }

        Ok(())
    }

    fn is_uhk80_device(device: &DeviceInfo) -> bool {
        device.vendor_id() == UHK_VENDOR_ID && device.product_id() == UHK80_RIGHT_PID
    }

    fn is_uhk80_communication_interface(device: &DeviceInfo) -> bool {
        is_uhk80_device(device)
            && matches!(
                (device.usage_page(), device.usage()),
                (128, 129) | (65280, 1)
            )
    }
}

#[cfg(not(windows))]
mod platform {
    pub fn keyboard_present() -> bool {
        false
    }

    pub fn display_macro_command(_command: &str) -> anyhow::Result<()> {
        anyhow::bail!("direct UHK HID output is only implemented on Windows; use --mock-display")
    }
}
