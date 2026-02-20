mod transport;

use std::env;
use std::io;
use std::thread;
use std::time::Duration;

use rp2040_fred_protocol::bridge_proto::{MsgType, Packet};
use rp2040_fred_protocol::dro_decode::{counts_to_mm, Calibration, DroSnapshot};
use transport::{HostTransport, MockTransport, UsbTransport};

fn main() -> io::Result<()> {
    let mut args = env::args().skip(1);
    let cmd = args.next().unwrap_or_else(|| "help".to_string());
    let mode = args.next().unwrap_or_else(|| "mock".to_string());

    match (cmd.as_str(), mode.as_str()) {
        ("on", "mock") => set_mock_telemetry(true),
        ("off", "mock") => set_mock_telemetry(false),
        ("monitor", "mock") => {
            let packets = args
                .next()
                .and_then(|s| s.parse::<usize>().ok())
                .unwrap_or(50);
            monitor_mock(packets, Duration::from_millis(25));
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
    println!(
        "mock telemetry {} -> {} reply packet(s)",
        if enable { "ON" } else { "OFF" },
        replies.len()
    );
    Ok(())
}

fn monitor_mock(target_packets: usize, period: Duration) {
    let mut t = MockTransport::new();
    let _ = t.transact(Packet::telemetry_set(1, true, period.as_millis() as u16));

    let cal = Calibration::default();

    println!("step  X_mm        Z_mm        RPM");
    let mut shown = 0usize;
    while shown < target_packets {
        if let Some(pkt) = t.next_packet() {
            if pkt.msg_type != MsgType::Telemetry || pkt.payload_len < 16 {
                continue;
            }
            let p = pkt.payload_used();
            let snapshot = DroSnapshot {
                x_counts: i32::from_le_bytes([p[4], p[5], p[6], p[7]]),
                z_counts: i32::from_le_bytes([p[8], p[9], p[10], p[11]]),
                rpm: u16::from_le_bytes([p[12], p[13]]),
            };
            let (x_mm, z_mm, rpm) = counts_to_mm(snapshot, cal);
            println!("{:04}  {:+9.3}   {:+9.3}   {:5}", shown, x_mm, z_mm, rpm);
            shown += 1;
        }
        thread::sleep(period);
    }
}

fn set_usb_telemetry(enable: bool) -> io::Result<()> {
    let mut t = UsbTransport::open(0x2E8A, 0x000A)?;
    let req = Packet::telemetry_set(1, enable, 100);
    let replies = t.transact(req)?;
    println!(
        "usb telemetry {} -> {} reply packet(s)",
        if enable { "ON" } else { "OFF" },
        replies.len()
    );
    Ok(())
}

fn monitor_usb() -> io::Result<()> {
    let mut t = UsbTransport::open(0x2E8A, 0x000A)?;
    let _ = t.transact(Packet::telemetry_set(1, true, 25))?;

    let cal = Calibration::default();
    println!("step  X_mm        Z_mm        RPM");

    let mut i = 0usize;
    loop {
        let pkt = t.read_packet()?;
        if pkt.msg_type != MsgType::Telemetry || pkt.payload_len < 16 {
            continue;
        }
        let p = pkt.payload_used();
        let snapshot = DroSnapshot {
            x_counts: i32::from_le_bytes([p[4], p[5], p[6], p[7]]),
            z_counts: i32::from_le_bytes([p[8], p[9], p[10], p[11]]),
            rpm: u16::from_le_bytes([p[12], p[13]]),
        };
        let (x_mm, z_mm, rpm) = counts_to_mm(snapshot, cal);
        println!("{:04}  {:+9.3}   {:+9.3}   {:5}", i, x_mm, z_mm, rpm);
        i = i.wrapping_add(1);
    }
}
