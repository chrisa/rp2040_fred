use rp2040_fred_protocol::mock_bus::MockBusRunner;

fn main() {
    let count = std::env::args()
        .nth(1)
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(30);

    let mut sim = MockBusRunner::new();

    println!("step cmd_fc80 status_fcf0 response_fcf1");
    for i in 0..count {
        let frame = sim.step();
        println!(
            "{:04}   {:02X}       {:02X}         {:02X}",
            i, frame.cmd_fc80, frame.status_fcf0, frame.response_fcf1
        );
    }
}
