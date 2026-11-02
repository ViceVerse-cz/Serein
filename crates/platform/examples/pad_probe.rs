//! Throwaway: do DualSense HID reports arrive while this window is unfocused? Deleted in Task 4.
fn main() {
	#[cfg(target_os = "windows")]
	{
		let api = hidapi::HidApi::new().expect("hidapi init");
		let Some(info) = api.device_list().find(|d| {
			d.vendor_id() == 0x054C && matches!(d.product_id(), 0x0CE6 | 0x0DF2) && d.usage_page() == 1 && d.usage() == 5
		}) else {
			println!("no DualSense gamepad interface found");
			return;
		};
		println!("pad: {:?} {:04x} {:?}", info.product_string(), info.product_id(), info.bus_type());
		let device = info.open_device(&api).expect("open");
		println!("Focus another window now; press Cross, L2, D-pad Up for 30 s.");
		let end = std::time::Instant::now() + std::time::Duration::from_secs(30);
		let mut buf = [0u8; 78];
		let mut last = Vec::new();
		while std::time::Instant::now() < end {
			let Ok(n) = device.read_timeout(&mut buf, 100) else { break };
			if n == 0 {
				continue;
			}
			// USB 0x01: buttons at [8..=10], L2/R2 at [5],[6]. BT 0x31: +1. BT 0x01 (short): [5..=7], [8],[9].
			let key = match buf[0] {
				0x31 => buf[9..=11].to_vec(),
				0x01 if n >= 11 => buf[8..=10].to_vec(),
				_ => buf[5..=7].to_vec(),
			};
			if key != last {
				println!("report {:#04x} len {n}: buttons {:02x?}", buf[0], key);
				last = key;
			}
		}
	}
}
