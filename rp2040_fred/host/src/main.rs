mod dro_decode;
mod transport;

use std::env;
use std::io;
use std::thread;
use std::time::Duration;

use dro_decode::{counts_to_mm, Calibration, DroAssembler};
use rp2040_fred_firmware::bridge_proto::Packet;
use transport::{HostTransport, MockTransport, UsbTransport};

fn main() -> io::Result<()> {
    let mut args = env::args().skip(1);
    let cmd = args.next().unwrap_or_else(|| "help".to_string());
    let mode = args.next().unwrap_or_else(|| "mock".to_string());

    match (cmd.as_str(), mode.as_str()) {
        ("on", "mock") => set_mock_telemetry(true),
        ("off", "mock") => set_mock_telemetry(false),
        ("monitor", "mock") => {
            let steps = args
                .next()
                .and_then(|s| s.parse::<usize>().ok())
                .unwrap_or(200);
            monitor_mock(steps, Duration::from_millis(100));
            Ok(())
        }
        ("on", "usb") => set_usb_telemetry(true),
        ("off", "usb") => set_usb_telemetry(false),
        ("monitor", "usb") => monitor_usb(),
        _ => {
            print_help();
            Ok(())
        }
    }
}

fn print_help() {
    eprintln!("usage:");
    eprintln!("  fredctl on mock");
    eprintln!("  fredctl off mock");
    eprintln!("  fredctl monitor mock [steps]");
    eprintln!("  fredctl on usb");
    eprintln!("  fredctl off usb");
    eprintln!("  fredctl monitor usb");
}

fn set_mock_telemetry(enable: bool) -> io::Result<()> {
    let mut t = MockTransport::new();
    let req = Packet::telemetry_set(1, enable, 100);
    let replies = t.transact(req)?;
    println!("mock telemetry {} -> {} reply packet(s)", if enable { "ON" } else { "OFF" }, replies.len());
    Ok(())
}

fn monitor_mock(steps: usize, period: Duration) {
    let mut t = MockTransport::new();
    let _ = t.transact(Packet::telemetry_set(1, true, period.as_millis() as u16));

    let mut assembler = DroAssembler::new();
    let cal = Calibration::default();

    println!("step  X_mm        Z_mm        RPM");
    for i in 0..steps {
        if let Some(frame) = t.next_frame() {
            assembler.on_fc80_fcf1(frame.cmd_fc80, frame.response_fcf1);
            let snapshot = assembler.snapshot();
            let (x_mm, z_mm, rpm) = counts_to_mm(snapshot, cal);
            println!("{:04}  {:+9.3}   {:+9.3}   {:5}", i, x_mm, z_mm, rpm);
        }
        thread::sleep(period);
    }
}

fn set_usb_telemetry(enable: bool) -> io::Result<()> {
    let mut t = UsbTransport::open(0x2E8A, 0x000A, 0)?;
    let req = Packet::telemetry_set(1, enable, 100);
    let replies = t.transact(req)?;
    println!("usb telemetry {} -> {} reply packet(s)", if enable { "ON" } else { "OFF" }, replies.len());
    Ok(())
}

fn monitor_usb() -> io::Result<()> {
    let _t = UsbTransport::open(0x2E8A, 0x000A, 0)?;
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "USB monitor path not yet implemented",
    ))
}
