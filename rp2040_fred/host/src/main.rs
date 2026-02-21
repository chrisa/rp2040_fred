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
        ("capture-on", "usb") => set_usb_capture(true),
        ("capture-off", "usb") => set_usb_capture(false),
        ("capture", "usb") => monitor_usb_capture(),
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
    eprintln!("  fredctl capture-on usb");
    eprintln!("  fredctl capture-off usb");
    eprintln!("  fredctl capture usb");
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
    let _ = t.transact(Packet::capture_set(1, false))?;
    let req = Packet::telemetry_set(2, enable, 100);
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
    let _ = t.transact(Packet::capture_set(1, false))?;
    let _ = t.transact(Packet::telemetry_set(2, true, 25))?;

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

fn set_usb_capture(enable: bool) -> io::Result<()> {
    let mut t = UsbTransport::open(0x2E8A, 0x000A)?;
    let req = Packet::capture_set(1, enable);
    let replies = t.transact(req)?;
    println!(
        "usb passive capture {} -> {} reply packet(s)",
        if enable { "ON" } else { "OFF" },
        replies.len()
    );
    Ok(())
}

fn monitor_usb_capture() -> io::Result<()> {
    let mut t = UsbTransport::open(0x2E8A, 0x000A)?;
    let _ = t.transact(Packet::capture_set(1, true))?;

    println!("step  tick        sample      D    A   RnW CLK C18 C19 FREDn");
    let mut i = 0usize;
    loop {
        let pkt = t.read_packet()?;
        if pkt.msg_type != MsgType::TraceSample || pkt.payload_len < 8 {
            continue;
        }
        let p = pkt.payload_used();
        let tick = u32::from_le_bytes([p[0], p[1], p[2], p[3]]);
        let sample = u32::from_le_bytes([p[4], p[5], p[6], p[7]]);

        let d = (sample & 0xFF) as u8;
        let a = ((sample >> 8) & 0xFF) as u8;
        let rnw = ((sample >> 16) & 1) as u8;
        let clk = ((sample >> 17) & 1) as u8;
        let c18 = ((sample >> 18) & 1) as u8;
        let c19 = ((sample >> 19) & 1) as u8;
        let fred_n = ((sample >> 20) & 1) as u8;

        println!(
            "{:04}  {:10}  0x{sample:08X}  {d:02X}  {a:02X}   {rnw}   {clk}   {c18}   {c19}    {fred_n}",
            i, tick
        );
        i = i.wrapping_add(1);
    }
}
