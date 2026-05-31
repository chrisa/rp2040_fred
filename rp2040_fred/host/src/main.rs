use std::env;
use std::fs::File;
use std::io;
use std::io::BufReader;

use fredctl::capture_file::{CaptureReader, CaptureWriter};
use fredctl::monitor::{FredMonitorClient, MonitorSnapshot};
use fredctl::transport::UsbTransport;
use rp2040_fred_protocol::bridge_proto::Packet;

const MONITOR_STEP_WIDTH: usize = 10;
const MONITOR_AXIS_WIDTH: usize = 12;
const MONITOR_RPM_WIDTH: usize = 6;

fn main() -> io::Result<()> {
    let mut args = env::args().skip(1);
    let cmd = args.next().unwrap_or_else(|| "help".to_string());
    let mode = args.next().unwrap_or_default();

    match (cmd.as_str(), mode.as_str()) {
        ("monitor", "usb") => monitor_usb(),
        ("capture", "usb") => capture_usb(CaptureUsbOptions::parse(args)?),
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
        _ => {
            print_help();
            Ok(())
        }
    }
}

fn print_help() {
    eprintln!("usage:");
    eprintln!("  fredctl monitor usb");
    eprintln!("  fredctl capture usb [--ignore-fcf0-reads]");
    eprintln!("  fredctl capture file <capture.bin>");
    eprintln!("  fredctl raw file <capture.bin>");
}

fn monitor_usb() -> io::Result<()> {
    let mut client = FredMonitorClient::open(0x2E8A, 0x000A)?;
    client.enable_polling(10)?;
    print_monitor_header();

    let mut i = 0usize;
    loop {
        let snapshot = client.next_snapshot()?;
        print_monitor_snapshot(i, snapshot);
        i = i.wrapping_add(1);
    }
}

fn capture_usb(options: CaptureUsbOptions) -> io::Result<()> {
    let mut t = UsbTransport::open(0x2E8A, 0x000A)?;
    let _ = t.transact(Packet::telemetry_set(1, false, 25))?;
    let _ = t.transact(Packet::capture_set(2, true))?;

    let mut printer = RawSamplePrinter::new(options.raw_print_options());
    printer.print_header();
    let mut i = 0u64;
    let mut counters = TraceCaptureCounters::default();

    loop {
        match t.read_packet() {
            Ok(packet) => {
                let Some(trace) = packet.decode_trace_samples() else {
                    eprintln!("failed to decode trace samples");
                    continue;
                };

                if let Some(comment) =
                    counters.update(trace.dropped_samples_total, trace.rx_stall_count_total)
                {
                    println!("{comment}");
                }

                for sample in trace.iter_samples() {
                    printer.print_sample(i, trace.timestamp_us, sample);
                    i = i.wrapping_add(1);
                }
            }
            Err(e) => {
                eprintln!("{}", e);
            }
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

    let mut printer = RawSamplePrinter::new(RawPrintOptions::default());
    printer.print_header();
    while let Some(batch) = reader.read_batch()? {
        if let Some(comment) =
            counters.update(batch.dropped_samples_total, batch.rx_stall_count_total)
        {
            println!("{comment}");
        }

        let batch_timestamp_us = batch.timestamp_us;
        for sample in batch.samples {
            printer.print_sample(sample_index, batch_timestamp_us, sample);
            sample_index = sample_index.wrapping_add(1);
        }
    }

    Ok(())
}

fn print_monitor_header() {
    println!(
        "{:<step_width$}  {:<axis_width$}  {:<axis_width$}  {:<rpm_width$}",
        "step",
        "X_mm",
        "Z_mm",
        "RPM",
        step_width = MONITOR_STEP_WIDTH,
        axis_width = MONITOR_AXIS_WIDTH,
        rpm_width = MONITOR_RPM_WIDTH,
    );
}

fn print_monitor_snapshot(step: usize, snapshot: MonitorSnapshot) {
    let x_mm = format!("{:+.3}", snapshot.x_mm);
    let z_mm = format!("{:+.3}", snapshot.z_mm);
    let rpm = snapshot.spindle_rpm.to_string();

    println!(
        "{step:>step_width$}  {x_mm:>axis_width$}  {z_mm:>axis_width$}  {rpm:>rpm_width$}",
        step_width = MONITOR_STEP_WIDTH,
        axis_width = MONITOR_AXIS_WIDTH,
        rpm_width = MONITOR_RPM_WIDTH,
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
            "# capture dropped_delta={dropped_delta} dropped_total={dropped_samples_total} rxfifo_block_delta={stall_delta} rxfifo_block_total={rx_stall_count_total}"
        ))
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct CaptureUsbOptions {
    ignore_fcf0_reads: bool,
}

impl CaptureUsbOptions {
    fn parse(args: impl Iterator<Item = String>) -> io::Result<Self> {
        let mut options = Self::default();

        for arg in args {
            match arg.as_str() {
                "--ignore-fcf0-reads" => options.ignore_fcf0_reads = true,
                _ => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        format!("unknown capture usb option: {arg}"),
                    ));
                }
            }
        }

        Ok(options)
    }

    fn raw_print_options(self) -> RawPrintOptions {
        RawPrintOptions {
            ignore_fcf0_reads: self.ignore_fcf0_reads,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct RawPrintOptions {
    ignore_fcf0_reads: bool,
}

struct RawSamplePrinter {
    options: RawPrintOptions,
    last_emitted_step: Option<u64>,
}

// temporary! host should not know this stuff
pub const ONE_MHZ_PIN: u8 = 16;
pub const READ_WRITE_PIN: u8 = 17;
pub const FRED_PIN: u8 = 18;

impl RawSamplePrinter {
    fn new(options: RawPrintOptions) -> Self {
        Self {
            options,
            last_emitted_step: None,
        }
    }

    fn print_header(&self) {
        println!("step  delta_us  batch_us          sample      D    A   RnW CLK FREDn");
    }

    fn print_sample(&mut self, step: u64, batch_timestamp_us: Option<u64>, sample: u32) {
        if self.options.ignore_fcf0_reads && sample_is_fcf0_read(sample) {
            return;
        }

        let d = (sample & 0xFF) as u8;
        let a = ((sample >> 8) & 0xFF) as u8;
        let rnw = if ((sample >> READ_WRITE_PIN) & 1) as u8 == 0 {
            "W"
        } else {
            "R"
        };
        let clk = ((sample >> ONE_MHZ_PIN) & 1) as u8;
        let fred_n = ((sample >> FRED_PIN) & 1) as u8;
        let delta_us = self
            .last_emitted_step
            .map(|prev_step| step.wrapping_sub(prev_step).to_string())
            .unwrap_or_else(|| "-".to_string());
        let batch_us = batch_timestamp_us
            .map(|timestamp_us| timestamp_us.to_string())
            .unwrap_or_else(|| "-".to_string());

        println!(
            "{step:04}  {delta_us:>8}  {batch_us:>16}  0x{sample:08X}  {d:02X}  {a:02X}   {rnw}   {clk}    {fred_n}",
        );
        self.last_emitted_step = Some(step);
    }
}

fn sample_is_fcf0_read(sample: u32) -> bool {
    ((sample >> 8) & 0xFF) as u8 == 0xF0 && ((sample >> READ_WRITE_PIN) & 1) != 0
}
