mod capture_file;
mod trace_decode;
mod transport;

use std::env;
use std::fs::File;
use std::io;
use std::io::BufReader;
use std::thread;
use std::time::Duration;

use capture_file::{CaptureReader, CaptureWriter};
use rp2040_fred_protocol::bridge_proto::{MsgType, Packet};
use rp2040_fred_protocol::dro_decode::{counts_to_mm, Calibration, DroSnapshot};
use trace_decode::{FeedbackDecoder, FeedbackSnapshot};
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
        ("capture", "file") => {
            let path = args.next().ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "usage: fredctl capture file <capture.bin>",
                )
            })?;
            capture_usb_to_file(&path)
        }
        ("raw", "file") => {
            let path = args.next().ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "usage: fredctl raw file <capture.bin>",
                )
            })?;
            raw_capture_file(&path)
        }
        ("decode", "usb") => decode_usb_capture(),
        ("decode", "file") => {
            let path = args.next().ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "usage: fredctl decode file <capture.bin>",
                )
            })?;
            decode_capture_file(&path)
        }
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
    eprintln!("  fredctl capture file <capture.bin>");
    eprintln!("  fredctl raw file <capture.bin>");
    eprintln!("  fredctl decode usb");
    eprintln!("  fredctl decode file <capture.bin>");
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

    print_raw_header();
    let mut i = 0u64;
    let mut counters = TraceCaptureCounters::default();

    loop {
        let pkt = t.read_packet()?;
        let Some(trace) = pkt.decode_trace_samples() else {
            continue;
        };

        if let Some(comment) =
            counters.update(trace.dropped_samples_total, trace.rx_stall_count_total)
        {
            println!("{comment}");
        }

        for sample in trace.iter_samples() {
            print_raw_sample(i, sample);
            i = i.wrapping_add(1);
        }
    }
}

fn decode_usb_capture() -> io::Result<()> {
    let mut t = UsbTransport::open(0x2E8A, 0x000A)?;
    let _ = t.transact(Packet::telemetry_set(1, false, 100))?;
    let _ = t.transact(Packet::capture_set(2, true))?;

    let mut decoder = FeedbackDecoder::new();
    let mut sample_index = 0u64;
    let mut counters = TraceCaptureCounters::default();

    print_decode_header();
    loop {
        let pkt = t.read_packet()?;
        let Some(trace) = pkt.decode_trace_samples() else {
            continue;
        };

        if let Some(comment) =
            counters.update(trace.dropped_samples_total, trace.rx_stall_count_total)
        {
            println!("{comment}");
        }

        for sample in trace.iter_samples() {
            if let Some(snapshot) = decoder.ingest_sample(sample_index, sample) {
                print_decoded_snapshot(snapshot);
            }
            sample_index = sample_index.wrapping_add(1);
        }
    }
}

fn capture_usb_to_file(path: &str) -> io::Result<()> {
    let mut t = UsbTransport::open(0x2E8A, 0x000A)?;
    let _ = t.transact(Packet::telemetry_set(1, false, 100))?;
    let _ = t.transact(Packet::capture_set(2, true))?;

    let file = File::create(path)?;
    let mut writer = CaptureWriter::new(file)?;

    loop {
        let pkt = t.read_packet()?;
        let Some(trace) = pkt.decode_trace_samples() else {
            continue;
        };
        writer.write_trace(trace)?;
    }
}

fn raw_capture_file(path: &str) -> io::Result<()> {
    let file = File::open(path)?;
    let mut reader = CaptureReader::new(BufReader::new(file))?;
    let mut counters = TraceCaptureCounters::default();
    let mut sample_index = 0u64;

    print_raw_header();
    while let Some(batch) = reader.read_batch()? {
        if let Some(comment) =
            counters.update(batch.dropped_samples_total, batch.rx_stall_count_total)
        {
            println!("{comment}");
        }

        for sample in batch.samples {
            print_raw_sample(sample_index, sample);
            sample_index = sample_index.wrapping_add(1);
        }
    }

    Ok(())
}

fn decode_capture_file(path: &str) -> io::Result<()> {
    let file = File::open(path)?;
    let mut reader = CaptureReader::new(BufReader::new(file))?;
    let mut decoder = FeedbackDecoder::new();
    let mut counters = TraceCaptureCounters::default();
    let mut sample_index = 0u64;

    print_decode_header();
    while let Some(batch) = reader.read_batch()? {
        if let Some(comment) =
            counters.update(batch.dropped_samples_total, batch.rx_stall_count_total)
        {
            println!("{comment}");
        }

        for sample in batch.samples {
            if let Some(snapshot) = decoder.ingest_sample(sample_index, sample) {
                print_decoded_snapshot(snapshot);
            }
            sample_index = sample_index.wrapping_add(1);
        }
    }

    Ok(())
}

fn print_raw_header() {
    println!("step  sample      D    A   RnW CLK FREDn");
}

fn print_raw_sample(step: u64, sample: u32) {
    let d = (sample & 0xFF) as u8;
    let a = ((sample >> 8) & 0xFF) as u8;
    let rnw = if ((sample >> 16) & 1) as u8 == 0 {
        "W"
    } else {
        "R"
    };
    let clk = ((sample >> 17) & 1) as u8;
    let fred_n = ((sample >> 20) & 1) as u8;

    println!(
        "{:04}  0x{sample:08X}  {d:02X}  {a:02X}   {rnw}   {clk}    {fred_n}",
        step
    );
}

fn print_decode_header() {
    println!("sample    X_raw    Z_raw    RPMraw RPMdisp");
}

fn print_decoded_snapshot(snapshot: FeedbackSnapshot) {
    println!(
        "{:08}  {}  {}  {:6} {:7}",
        snapshot.sample_index,
        snapshot.x_digits(),
        snapshot.z_digits(),
        snapshot.rpm_raw,
        snapshot.rpm_display
    );
}

#[derive(Default)]
struct TraceCaptureCounters {
    dropped_samples_total: u32,
    rx_stall_count_total: u32,
}

impl TraceCaptureCounters {
    fn update(&mut self, dropped_samples_total: u32, rx_stall_count_total: u32) -> Option<String> {
        if dropped_samples_total == self.dropped_samples_total
            && rx_stall_count_total == self.rx_stall_count_total
        {
            return None;
        }

        let dropped_delta = dropped_samples_total.wrapping_sub(self.dropped_samples_total);
        let stall_delta = rx_stall_count_total.wrapping_sub(self.rx_stall_count_total);
        self.dropped_samples_total = dropped_samples_total;
        self.rx_stall_count_total = rx_stall_count_total;

        Some(format!(
            "# capture dropped_delta={dropped_delta} dropped_total={dropped_samples_total} rxstall_delta={stall_delta} rxstall_total={rx_stall_count_total}"
        ))
    }
}
