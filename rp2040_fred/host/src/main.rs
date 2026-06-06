use std::env;
use std::fs::File;
use std::io;
use std::io::BufReader;

use fredctl::canned_cycle::{self, CannedCycleCode, CannedCycleParams};
use fredctl::capture_file::{CaptureReader, CaptureWriter};
use fredctl::monitor::{FredMonitorClient, MonitorSnapshot};
use fredctl::motion::AxisCalibration;
use fredctl::spindle::{self, SpindleDirection};
use fredctl::threading;
use fredctl::tool;
use fredctl::transport::{UsbRole, UsbTransport};
use rp2040_fred_protocol::bridge_proto::{
    CommandBlock, CommandBlockRequest, ControllerAction, ControllerStatus, MsgType, Packet,
    RpmServiceMode, COMMAND_BLOCK_FLAG_CYCLE_START_WAIT,
};
use std::thread;
use std::time::Duration;

const MONITOR_STEP_WIDTH: usize = 10;
const MONITOR_AXIS_WIDTH: usize = 12;
const MONITOR_RPM_WIDTH: usize = 6;

fn main() -> io::Result<()> {
    let mut args = env::args().skip(1);
    let cmd = args.next().unwrap_or_else(|| "help".to_string());
    let mode = args.next().unwrap_or_default();

    match (cmd.as_str(), mode.as_str()) {
        ("monitor", "usb") => monitor_usb(),
        ("jog", "usb") => motion_usb(MotionStart::Immediate, MotionOptions::parse(args)?),
        ("cycle-move", "usb") => motion_usb(MotionStart::CycleStart, MotionOptions::parse(args)?),
        ("cycle-start", "usb") => cycle_start_usb(),
        ("tool", "usb") => tool_usb(ToolOptions::parse(args)?),
        ("spindle", "usb") => spindle_usb(SpindleOptions::parse(args)?),
        ("g33", "usb") => g33_usb(G33Options::parse(args)?),
        ("canned-cycle", "usb") => canned_cycle_usb(CannedCycleOptions::parse(args)?),
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
    eprintln!("  fredctl cycle-move usb --mode rapid --x-counts <diameter_delta> --z-counts <delta> --slew <value>");
    eprintln!("  fredctl cycle-move usb --mode feed --x-counts <diameter_delta> --z-counts <delta> --feed <value> --slew <value>");
    eprintln!("  fredctl jog usb --mode rapid --x-counts <diameter_delta> --z-counts <delta> --slew <value>");
    eprintln!("  fredctl jog usb --mode feed --x-counts <diameter_delta> --z-counts <delta> --feed <value> --slew <value>");
    eprintln!("  fredctl cycle-start usb");
    eprintln!("  fredctl tool usb --current-station <1-8> --target-station <1-8> --slew <value> [--wait-complete]");
    eprintln!("  fredctl spindle usb --start <forward|reverse> (--rpm <rpm>|--ssl <0-127>) [--wait-complete]");
    eprintln!("  fredctl spindle usb --stop [--wait-complete]");
    eprintln!(
        "  fredctl g33 usb --z-mm <delta> --pitch-mm <pitch> --slew <value> [--wait-complete]"
    );
    eprintln!("  fredctl canned-cycle usb --code <G80|G81|G82|G83|G84> [--x-mm <value>] [--z-mm <value>] [--i <value>] [--k <value>] [--f <value>] [--slew <value>] [--wait-complete]");
    eprintln!("  fredctl capture usb [--ignore-fcf0-reads]");
    eprintln!("  fredctl capture file <capture.bin>");
    eprintln!("  fredctl raw file <capture.bin>");
}

fn monitor_usb() -> io::Result<()> {
    let mut client = FredMonitorClient::open(0x2E8A, 0x000A)?;
    client.enable_polling(10, RpmServiceMode::Manual)?;
    print_monitor_header();

    let mut i = 0usize;
    loop {
        let snapshot = client.next_snapshot()?;
        print_monitor_snapshot(i, snapshot);
        i = i.wrapping_add(1);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MotionStart {
    Immediate,
    CycleStart,
}

impl MotionStart {
    fn waits_for_cycle_start(self) -> bool {
        matches!(self, Self::CycleStart)
    }

    fn label(self) -> &'static str {
        match self {
            Self::Immediate => "jog",
            Self::CycleStart => "cycle-start motion",
        }
    }
}

fn motion_usb(start: MotionStart, options: MotionOptions) -> io::Result<()> {
    let mut t = UsbTransport::open(0x2E8A, 0x000A, UsbRole::Master)?;
    t.set_timeout(Duration::from_millis(1000));

    let request = options.command_request(start.waits_for_cycle_start())?;
    let seq = send_command_request(&mut t, 1, request)?;
    println!("sent {} CommandBlock: {:?}", start.label(), request.block);
    wait_controller_idle(&mut t, seq)?;
    println!("controller idle");
    Ok(())
}

fn cycle_start_usb() -> io::Result<()> {
    let mut t = UsbTransport::open(0x2E8A, 0x000A, UsbRole::Master)?;
    t.set_timeout(Duration::from_millis(1000));
    let replies = t.transact(Packet::controller_action(
        1,
        ControllerAction::CycleStartWait,
    ))?;
    ensure_ack(&replies, 1, MsgType::ControllerAction)?;
    let seq = 2;
    wait_controller_idle(&mut t, seq)?;
    println!("cycle-start wait complete");
    Ok(())
}

fn tool_usb(options: ToolOptions) -> io::Result<()> {
    let mut t = UsbTransport::open(0x2E8A, 0x000A, UsbRole::Master)?;
    t.set_timeout(Duration::from_millis(1000));

    let requests = options.command_requests()?;
    let mut seq = 1;
    for request in &requests {
        seq = send_command_request(&mut t, seq, *request)?;
    }

    println!(
        "sent tool change: current_station={} target_station={} steps={}",
        options.current_station,
        options.target_station,
        options.step_count()
    );

    if options.wait_complete {
        wait_controller_idle(&mut t, seq)?;
        println!("controller idle");
    }
    Ok(())
}

fn spindle_usb(options: SpindleOptions) -> io::Result<()> {
    let mut t = UsbTransport::open(0x2E8A, 0x000A, UsbRole::Master)?;
    t.set_timeout(Duration::from_millis(1000));

    let request = options.command_request()?;
    let seq = send_command_request(&mut t, 1, request)?;

    match options.command {
        SpindleCommand::Start(direction) => {
            println!(
                "sent spindle start: direction={} ssl={}",
                direction.label(),
                request.block.m9
            );
        }
        SpindleCommand::Stop => {
            println!("sent spindle stop");
        }
    }

    if options.wait_complete {
        wait_controller_idle(&mut t, seq)?;
        println!("controller idle");
    }
    Ok(())
}

fn g33_usb(options: G33Options) -> io::Result<()> {
    let mut t = UsbTransport::open(0x2E8A, 0x000A, UsbRole::Master)?;
    t.set_timeout(Duration::from_millis(1000));

    let request = threading::thread_sync_command_request_mm(
        options.z_mm,
        options.pitch_mm,
        options.slew,
        default_axis_calibration(),
    )?;
    let seq = send_command_request(&mut t, 1, request)?;

    println!(
        "sent G33 thread-sync CommandBlock: z_mm={} pitch_mm={} block={:?}",
        options.z_mm, options.pitch_mm, request.block
    );
    if options.wait_complete {
        wait_controller_idle(&mut t, seq)?;
        println!("controller idle");
    }
    Ok(())
}

fn canned_cycle_usb(options: CannedCycleOptions) -> io::Result<()> {
    let mut t = UsbTransport::open(0x2E8A, 0x000A, UsbRole::Master)?;
    t.set_timeout(Duration::from_millis(1000));

    let requests = canned_cycle::canned_cycle_command_requests_mm(
        options.code,
        options.params(),
        default_axis_calibration(),
    )?;
    if requests.is_empty() {
        println!("{} canned-cycle cancel/no-op", options.code.label());
        return Ok(());
    }

    let mut seq = 1;
    for request in &requests {
        seq = send_command_request(&mut t, seq, *request)?;
    }
    println!(
        "sent {} canned-cycle CommandBlocks: count={}",
        options.code.label(),
        requests.len()
    );
    if options.wait_complete {
        wait_controller_idle(&mut t, seq)?;
        println!("controller idle");
    }
    Ok(())
}

fn send_command_request(
    t: &mut UsbTransport,
    seq: u16,
    request: CommandBlockRequest,
) -> io::Result<u16> {
    let replies = t.transact(Packet::command_block_request(seq, request))?;
    ensure_ack(&replies, seq, MsgType::CommandBlock)?;
    Ok(seq.wrapping_add(1))
}

fn ensure_ack(replies: &[Packet], seq: u16, acked_type: MsgType) -> io::Result<()> {
    for pkt in replies {
        if pkt.seq != seq {
            continue;
        }

        match pkt.msg_type {
            MsgType::Ack if pkt.payload_len >= 2 && pkt.payload[0] == acked_type as u8 => {
                let status = pkt.payload[1];
                if status == 0 {
                    return Ok(());
                }
                return Err(io::Error::other(format!(
                    "device acked {:?} with nonzero status {status:#04x}",
                    acked_type
                )));
            }
            MsgType::Nack if pkt.payload_len >= 2 && pkt.payload[0] == acked_type as u8 => {
                return Err(io::Error::other(format!(
                    "device rejected {:?} with reason {:#04x}",
                    acked_type, pkt.payload[1]
                )));
            }
            _ => {}
        }
    }

    Err(io::Error::new(
        io::ErrorKind::TimedOut,
        format!("no ACK/NACK for {:?}", acked_type),
    ))
}

fn controller_status(t: &mut UsbTransport, seq: u16) -> io::Result<ControllerStatus> {
    let replies = t.transact(Packet::controller_status_req(seq))?;
    for pkt in replies {
        if pkt.seq != seq {
            continue;
        }
        if let Some(status) = pkt.decode_controller_status_ack() {
            return Ok(status);
        }
        if pkt.msg_type == MsgType::Nack
            && pkt.payload_len >= 2
            && pkt.payload[0] == MsgType::ControllerStatusReq as u8
        {
            return Err(io::Error::other(format!(
                "device rejected ControllerStatusReq with reason {:#04x}",
                pkt.payload[1]
            )));
        }
    }

    Err(io::Error::new(
        io::ErrorKind::TimedOut,
        "no controller status response",
    ))
}

fn wait_controller_idle(t: &mut UsbTransport, first_seq: u16) -> io::Result<()> {
    let mut seq = first_seq;
    loop {
        let status = controller_status(t, seq)?;
        if status.has_error() {
            return Err(io::Error::other("controller work reported an error"));
        }
        if status.is_idle() {
            return Ok(());
        }
        seq = seq.wrapping_add(1);
        thread::sleep(Duration::from_millis(50));
    }
}

fn capture_usb(options: CaptureUsbOptions) -> io::Result<()> {
    let mut t = UsbTransport::open(0x2E8A, 0x000A, UsbRole::Capture)?;
    let _ = t.transact(Packet::capture_set(1, true))?;

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
    let mut t = UsbTransport::open(0x2E8A, 0x000A, UsbRole::Capture)?;
    let _ = t.transact(Packet::capture_set(1, true))?;

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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MoveMode {
    Rapid,
    Feed,
}

impl MoveMode {
    fn parse(value: &str) -> io::Result<Self> {
        match value {
            "rapid" => Ok(Self::Rapid),
            "feed" => Ok(Self::Feed),
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("unknown move mode: {value}"),
            )),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct MotionOptions {
    mode: MoveMode,
    x_counts: i32,
    z_counts: i32,
    feed: Option<u32>,
    slew: u16,
}

impl MotionOptions {
    fn parse(args: impl Iterator<Item = String>) -> io::Result<Self> {
        let mut args = args;
        let mut mode = None;
        let mut x_counts = None;
        let mut z_counts = None;
        let mut feed = None;
        let mut slew = None;

        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--mode" => mode = Some(MoveMode::parse(&next_arg(&mut args, "--mode")?)?),
                "--x-counts" => {
                    x_counts = Some(parse_i32(
                        "--x-counts",
                        &next_arg(&mut args, "--x-counts")?,
                    )?)
                }
                "--z-counts" => {
                    z_counts = Some(parse_i32(
                        "--z-counts",
                        &next_arg(&mut args, "--z-counts")?,
                    )?)
                }
                "--feed" => feed = Some(parse_u32("--feed", &next_arg(&mut args, "--feed")?)?),
                "--slew" => slew = Some(parse_u16("--slew", &next_arg(&mut args, "--slew")?)?),
                _ => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        format!("unknown motion option: {arg}"),
                    ));
                }
            }
        }

        Ok(Self {
            mode: mode.ok_or_else(|| missing_option("--mode"))?,
            x_counts: x_counts.ok_or_else(|| missing_option("--x-counts"))?,
            z_counts: z_counts.ok_or_else(|| missing_option("--z-counts"))?,
            feed,
            slew: slew.ok_or_else(|| missing_option("--slew"))?,
        })
    }

    fn command_request(self, cycle_start_wait: bool) -> io::Result<CommandBlockRequest> {
        let mut flags = 0;
        if cycle_start_wait {
            flags |= COMMAND_BLOCK_FLAG_CYCLE_START_WAIT;
        }

        Ok(CommandBlockRequest {
            block: self.command_block()?,
            flags,
        })
    }

    fn command_block(self) -> io::Result<CommandBlock> {
        let mut block = rapid_command_block(self.x_counts, self.z_counts, self.slew)?;

        match self.mode {
            MoveMode::Rapid => {
                if self.feed.is_some() {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "--feed is only valid with --mode feed",
                    ));
                }
            }
            MoveMode::Feed => {
                block.m1 = 1;
                let feed = self.feed.ok_or_else(|| missing_option("--feed"))?;
                block.m8 = feed_timing(feed)?;
            }
        }

        Ok(block)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ToolOptions {
    current_station: u8,
    target_station: u8,
    slew: u16,
    wait_complete: bool,
}

impl ToolOptions {
    fn parse(args: impl Iterator<Item = String>) -> io::Result<Self> {
        let mut args = args;
        let mut current_station = None;
        let mut target_station = None;
        let mut slew = None;
        let mut wait_complete = false;

        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--current-station" => {
                    current_station = Some(parse_station(
                        "--current-station",
                        &next_arg(&mut args, "--current-station")?,
                    )?)
                }
                "--target-station" => {
                    target_station = Some(parse_station(
                        "--target-station",
                        &next_arg(&mut args, "--target-station")?,
                    )?)
                }
                "--slew" => slew = Some(parse_u16("--slew", &next_arg(&mut args, "--slew")?)?),
                "--wait-complete" => wait_complete = true,
                _ => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        format!("unknown tool option: {arg}"),
                    ));
                }
            }
        }

        Ok(Self {
            current_station: current_station.ok_or_else(|| missing_option("--current-station"))?,
            target_station: target_station.ok_or_else(|| missing_option("--target-station"))?,
            slew: slew.ok_or_else(|| missing_option("--slew"))?,
            wait_complete,
        })
    }

    fn step_count(self) -> u8 {
        tool::turret_step_count(self.current_station, self.target_station)
    }

    fn command_requests(self) -> io::Result<Vec<CommandBlockRequest>> {
        tool::turret_command_requests(self.current_station, self.target_station, self.slew)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum SpindleCommand {
    Start(SpindleDirection),
    Stop,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct SpindleOptions {
    command: SpindleCommand,
    rpm: Option<f32>,
    speed_code: Option<u16>,
    wait_complete: bool,
}

impl SpindleOptions {
    fn parse(args: impl Iterator<Item = String>) -> io::Result<Self> {
        let mut args = args;
        let mut command = None;
        let mut rpm = None;
        let mut speed_code = None;
        let mut wait_complete = false;

        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--start" => {
                    if command.is_some() {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidInput,
                            "only one spindle command may be specified",
                        ));
                    }
                    command = Some(SpindleCommand::Start(parse_spindle_direction(&next_arg(
                        &mut args, "--start",
                    )?)?));
                }
                "--stop" => {
                    if command.is_some() {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidInput,
                            "only one spindle command may be specified",
                        ));
                    }
                    command = Some(SpindleCommand::Stop);
                }
                "--rpm" => rpm = Some(parse_f32("--rpm", &next_arg(&mut args, "--rpm")?)?),
                "--ssl" => speed_code = Some(parse_u16("--ssl", &next_arg(&mut args, "--ssl")?)?),
                "--wait-complete" => wait_complete = true,
                _ => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        format!("unknown spindle option: {arg}"),
                    ));
                }
            }
        }

        let command = command.ok_or_else(|| missing_option("--start or --stop"))?;
        match command {
            SpindleCommand::Start(_) => {
                if rpm.is_none() && speed_code.is_none() {
                    return Err(missing_option("--rpm or --ssl"));
                }
            }
            SpindleCommand::Stop => {
                if rpm.is_some() || speed_code.is_some() {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "--rpm and --ssl are only valid with --start",
                    ));
                }
            }
        }

        Ok(Self {
            command,
            rpm,
            speed_code,
            wait_complete,
        })
    }

    fn command_request(self) -> io::Result<CommandBlockRequest> {
        match self.command {
            SpindleCommand::Start(direction) => {
                let speed_code = match self.speed_code {
                    Some(speed_code) => {
                        spindle::validate_speed_code(speed_code)?;
                        speed_code
                    }
                    None => spindle::speed_code_from_rpm(self.rpm.unwrap_or(0.0))?,
                };
                spindle::spindle_start_request(direction, speed_code)
            }
            SpindleCommand::Stop => Ok(spindle::spindle_stop_request()),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct G33Options {
    z_mm: f32,
    pitch_mm: f32,
    slew: u16,
    wait_complete: bool,
}

impl G33Options {
    fn parse(args: impl Iterator<Item = String>) -> io::Result<Self> {
        let mut args = args;
        let mut z_mm = None;
        let mut pitch_mm = None;
        let mut slew = None;
        let mut wait_complete = false;

        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--z-mm" => z_mm = Some(parse_f32("--z-mm", &next_arg(&mut args, "--z-mm")?)?),
                "--pitch-mm" => {
                    pitch_mm = Some(parse_f32(
                        "--pitch-mm",
                        &next_arg(&mut args, "--pitch-mm")?,
                    )?)
                }
                "--slew" => slew = Some(parse_u16("--slew", &next_arg(&mut args, "--slew")?)?),
                "--wait-complete" => wait_complete = true,
                _ => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        format!("unknown G33 option: {arg}"),
                    ));
                }
            }
        }

        Ok(Self {
            z_mm: z_mm.ok_or_else(|| missing_option("--z-mm"))?,
            pitch_mm: pitch_mm.ok_or_else(|| missing_option("--pitch-mm"))?,
            slew: slew.ok_or_else(|| missing_option("--slew"))?,
            wait_complete,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct CannedCycleOptions {
    code: CannedCycleCode,
    x_mm: Option<f32>,
    z_mm: Option<f32>,
    i: Option<f32>,
    k: Option<f32>,
    f: Option<f32>,
    slew: u16,
    wait_complete: bool,
}

impl CannedCycleOptions {
    fn parse(args: impl Iterator<Item = String>) -> io::Result<Self> {
        let mut args = args;
        let mut code = None;
        let mut x_mm = None;
        let mut z_mm = None;
        let mut i = None;
        let mut k = None;
        let mut f = None;
        let mut slew = 61;
        let mut wait_complete = false;

        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--code" => code = Some(CannedCycleCode::parse(&next_arg(&mut args, "--code")?)?),
                "--x-mm" => x_mm = Some(parse_f32("--x-mm", &next_arg(&mut args, "--x-mm")?)?),
                "--z-mm" => z_mm = Some(parse_f32("--z-mm", &next_arg(&mut args, "--z-mm")?)?),
                "--i" => i = Some(parse_f32("--i", &next_arg(&mut args, "--i")?)?),
                "--k" => k = Some(parse_f32("--k", &next_arg(&mut args, "--k")?)?),
                "--f" => f = Some(parse_f32("--f", &next_arg(&mut args, "--f")?)?),
                "--slew" => slew = parse_u16("--slew", &next_arg(&mut args, "--slew")?)?,
                "--wait-complete" => wait_complete = true,
                _ => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        format!("unknown canned-cycle option: {arg}"),
                    ));
                }
            }
        }

        Ok(Self {
            code: code.ok_or_else(|| missing_option("--code"))?,
            x_mm,
            z_mm,
            i,
            k,
            f,
            slew,
            wait_complete,
        })
    }

    fn params(self) -> CannedCycleParams {
        CannedCycleParams {
            x_mm: self.x_mm,
            z_mm: self.z_mm,
            i: self.i,
            k: self.k,
            f: self.f,
            slew: self.slew,
        }
    }
}

fn default_axis_calibration() -> AxisCalibration {
    AxisCalibration {
        x_counts_per_mm: 100.0,
        z_counts_per_mm: 100.0,
    }
}

fn rapid_command_block(
    x_diameter_counts: i32,
    z_counts: i32,
    slew: u16,
) -> io::Result<CommandBlock> {
    let x_radius_counts = checked_x_radius_counts(x_diameter_counts)?;
    let z_counts = checked_i16("--z-counts", z_counts)?;
    Ok(CommandBlock {
        m1: 0,
        m2: 0,
        m3: raw_word(x_radius_counts),
        m4: raw_word(z_counts),
        m5: 0,
        m6: 0,
        m7: 0,
        m8: 0,
        m9: slew,
        m10: 0,
    })
}

fn next_arg(args: &mut impl Iterator<Item = String>, option: &str) -> io::Result<String> {
    args.next().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("missing value for {option}"),
        )
    })
}

fn missing_option(option: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        format!("missing required option {option}"),
    )
}

fn parse_i32(option: &str, value: &str) -> io::Result<i32> {
    value.parse::<i32>().map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("invalid {option} value: {value}"),
        )
    })
}

fn parse_u32(option: &str, value: &str) -> io::Result<u32> {
    value.parse::<u32>().map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("invalid {option} value: {value}"),
        )
    })
}

fn parse_u16(option: &str, value: &str) -> io::Result<u16> {
    value.parse::<u16>().map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("invalid {option} value: {value}"),
        )
    })
}

fn parse_f32(option: &str, value: &str) -> io::Result<f32> {
    value.parse::<f32>().map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("invalid {option} value: {value}"),
        )
    })
}

fn parse_station(option: &str, value: &str) -> io::Result<u8> {
    let station = value.parse::<u8>().map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("invalid {option} value: {value}"),
        )
    })?;

    tool::validate_station(option, station)?;
    Ok(station)
}

fn parse_spindle_direction(value: &str) -> io::Result<SpindleDirection> {
    match value {
        "forward" | "fwd" => Ok(SpindleDirection::Forward),
        "reverse" | "rev" => Ok(SpindleDirection::Reverse),
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("unknown spindle direction: {value}"),
        )),
    }
}

fn checked_x_radius_counts(x_diameter_counts: i32) -> io::Result<i16> {
    if x_diameter_counts % 2 != 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "--x-counts must be even because the controller uses radius counts",
        ));
    }
    checked_i16("--x-counts / 2", x_diameter_counts / 2)
}

fn checked_i16(option: &str, value: i32) -> io::Result<i16> {
    i16::try_from(value).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{option} is outside signed 16-bit range: {value}"),
        )
    })
}

fn raw_word(value: i16) -> u16 {
    u16::from_le_bytes(value.to_le_bytes())
}

fn feed_timing(feed: u32) -> io::Result<u16> {
    if feed == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "--feed must be greater than zero",
        ));
    }

    let timing = 95_000 / feed;
    if timing == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "--feed is too high for nonzero controller timing",
        ));
    }

    u16::try_from(timing).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("feed timing is outside 16-bit range: {timing}"),
        )
    })
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

#[cfg(test)]
mod tests {
    use super::{
        raw_word, CannedCycleOptions, G33Options, MotionOptions, MoveMode, SpindleCommand,
        SpindleOptions, ToolOptions,
    };
    use fredctl::canned_cycle::CannedCycleCode;
    use fredctl::spindle::{
        SpindleDirection, SPINDLE_START_FORWARD_SUBCODE, SPINDLE_START_REVERSE_SUBCODE,
        SPINDLE_STOP_SUBCODE,
    };
    use fredctl::tool::turret_step_count;

    fn rapid_options() -> MotionOptions {
        MotionOptions {
            mode: MoveMode::Rapid,
            x_counts: -252,
            z_counts: 1500,
            feed: None,
            slew: 61,
        }
    }

    #[test]
    fn rapid_move_options_build_command_block() {
        let block = rapid_options().command_block().expect("rapid block");

        assert_eq!(block.m1, 0);
        assert_eq!(block.m2, 0);
        assert_eq!(block.m3, raw_word(-126));
        assert_eq!(block.m4, raw_word(1500));
        assert_eq!(block.m8, 0);
        assert_eq!(block.m9, 61);
        assert_eq!(block.m10, 0);
    }

    #[test]
    fn feed_move_options_build_command_block() {
        let block = MotionOptions {
            mode: MoveMode::Feed,
            x_counts: 0,
            z_counts: -1500,
            feed: Some(100),
            slew: 61,
        }
        .command_block()
        .expect("feed block");

        assert_eq!(block.m1, 1);
        assert_eq!(block.m2, 0);
        assert_eq!(block.m3, raw_word(0));
        assert_eq!(block.m4, raw_word(-1500));
        assert_eq!(block.m8, 950);
        assert_eq!(block.m9, 61);
    }

    #[test]
    fn feed_move_rejects_missing_feed() {
        let err = MotionOptions {
            mode: MoveMode::Feed,
            x_counts: 0,
            z_counts: 10,
            feed: None,
            slew: 61,
        }
        .command_block()
        .expect_err("missing feed");

        assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
    }

    #[test]
    fn move_options_reject_odd_x_diameter_counts() {
        let err = MotionOptions {
            mode: MoveMode::Rapid,
            x_counts: 1,
            z_counts: 0,
            feed: None,
            slew: 61,
        }
        .command_block()
        .expect_err("odd x");

        assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
    }

    #[test]
    fn cycle_move_request_sets_cycle_start_flag() {
        let request = rapid_options().command_request(true).expect("request");

        assert_ne!(
            request.flags & rp2040_fred_protocol::bridge_proto::COMMAND_BLOCK_FLAG_CYCLE_START_WAIT,
            0
        );
    }

    #[test]
    fn jog_request_has_no_flags() {
        let request = rapid_options().command_request(false).expect("request");

        assert_eq!(request.flags, 0);
    }

    #[test]
    fn turret_step_count_wraps_like_cncmak1() {
        assert_eq!(turret_step_count(1, 1), 0);
        assert_eq!(turret_step_count(1, 4), 3);
        assert_eq!(turret_step_count(6, 2), 4);
        assert_eq!(turret_step_count(8, 1), 1);
    }

    #[test]
    fn tool_options_build_turret_sequence() {
        let requests = ToolOptions {
            current_station: 6,
            target_station: 2,
            slew: 61,
            wait_complete: false,
        }
        .command_requests()
        .expect("tool blocks");

        assert_eq!(requests.len(), 3);
        assert_eq!(requests[0].block.m1, 1);
        assert_eq!(requests[0].block.m2, 0);
        assert_eq!(requests[0].block.m5, raw_word(-832 * 4));
        assert_eq!(requests[0].block.m8, 400);
        assert_eq!(requests[0].block.m9, 61);
        assert_eq!(requests[0].flags, 0);

        assert_eq!(requests[1].block.m5, raw_word(159 + 68 * 4));
        assert_eq!(requests[1].block.m8, 300);
        assert_eq!(requests[2].block.m5, raw_word(10));
        assert_eq!(requests[2].block.m8, 600);
    }

    #[test]
    fn tool_options_same_station_builds_no_blocks() {
        let requests = ToolOptions {
            current_station: 3,
            target_station: 3,
            slew: 61,
            wait_complete: false,
        }
        .command_requests()
        .expect("tool blocks");

        assert!(requests.is_empty());
    }

    #[test]
    fn spindle_reverse_start_options_match_capture() {
        let request = SpindleOptions {
            command: SpindleCommand::Start(SpindleDirection::Reverse),
            rpm: None,
            speed_code: Some(125),
            wait_complete: false,
        }
        .command_request()
        .expect("spindle start");

        assert_eq!(request.block.m1, 0);
        assert_eq!(request.block.m2, SPINDLE_START_REVERSE_SUBCODE);
        assert_eq!(request.block.m9, 125);
        assert_eq!(request.flags, 0);
    }

    #[test]
    fn spindle_forward_start_options_use_inferred_subcode() {
        let request = SpindleOptions {
            command: SpindleCommand::Start(SpindleDirection::Forward),
            rpm: Some(3000.0),
            speed_code: None,
            wait_complete: false,
        }
        .command_request()
        .expect("spindle start");

        assert_eq!(request.block.m1, 0);
        assert_eq!(request.block.m2, SPINDLE_START_FORWARD_SUBCODE);
        assert_eq!(request.block.m9, 125);
    }

    #[test]
    fn spindle_stop_options_match_capture() {
        let request = SpindleOptions {
            command: SpindleCommand::Stop,
            rpm: None,
            speed_code: None,
            wait_complete: false,
        }
        .command_request()
        .expect("spindle stop");

        assert_eq!(request.block.m1, 0);
        assert_eq!(request.block.m2, SPINDLE_STOP_SUBCODE);
        assert_eq!(request.block.m9, 0);
    }

    #[test]
    fn spindle_start_parse_requires_speed_source() {
        let err = SpindleOptions::parse(
            ["--start", "forward"]
                .into_iter()
                .map(std::string::ToString::to_string),
        )
        .expect_err("missing speed source");

        assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
    }

    #[test]
    fn g33_options_parse_required_fields() {
        let options = G33Options::parse(
            [
                "--z-mm",
                "-15",
                "--pitch-mm",
                "1.5",
                "--slew",
                "61",
                "--wait-complete",
            ]
            .into_iter()
            .map(std::string::ToString::to_string),
        )
        .expect("g33 options");

        assert_eq!(options.z_mm, -15.0);
        assert_eq!(options.pitch_mm, 1.5);
        assert_eq!(options.slew, 61);
        assert!(options.wait_complete);
    }

    #[test]
    fn canned_cycle_options_parse_g80_with_default_slew() {
        let options = CannedCycleOptions::parse(
            ["--code", "G80"]
                .into_iter()
                .map(std::string::ToString::to_string),
        )
        .expect("canned cycle options");

        assert_eq!(options.code, CannedCycleCode::G80);
        assert_eq!(options.slew, 61);
        assert!(!options.wait_complete);
    }
}
