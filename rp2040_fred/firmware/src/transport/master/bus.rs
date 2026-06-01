use crate::transport::pio::master::ThisMasterPio;
use embassy_time::{Duration, Instant};
use rp2040_fred_protocol::bridge_proto::CommandBlock;

const PROC_CONT_ARMED_MASK: u8 = 0x04;
const PROC_CONT_STARTED_MASK: u8 = 0x08;

pub struct Bus<'a> {
    pub pio: ThisMasterPio<'a>,
}

impl<'a> Bus<'a> {
    pub async fn command_cycle(&mut self, cmd: u8) -> u8 {
        // 1. Poll `F0` until bit 0 clears.
        // 2. Write one command byte to `80`.
        // 3. Poll `F0` again until bit 0 clears.
        // 4. Read one response byte from `F1`.
        self.poll_until(0xF0, 0x01).await;
        self.write_cycle(0x80, cmd).await;
        self.poll_until(0xF0, 0x01).await;
        return self.read_cycle(0xF1).await;
    }

    pub async fn command_cycle_timeout(&mut self, cmd: u8, timeout: Duration) -> Option<u8> {
        let deadline = Instant::now() + timeout;
        if !self.poll_until_deadline(0xF0, 0x01, deadline).await {
            return None;
        }
        self.write_cycle(0x80, cmd).await;
        if !self.poll_until_deadline(0xF0, 0x01, deadline).await {
            return None;
        }
        Some(self.read_cycle(0xF1).await)
    }

    pub async fn read_write_zero_pair(&mut self, addr: u8) {
        self.read_cycle(addr).await;
        self.write_cycle(addr, 0x00).await;
    }

    pub async fn write_command_block(&mut self, block: CommandBlock) {
        self.poll_until(0xF0, 0x01).await;

        let payload = block.to_payload();
        for (offset, data) in payload.iter().enumerate() {
            self.write_cycle(0x92 + offset as u8, *data).await;
        }
    }

    pub async fn clear_command_block(&mut self) {
        self.write_cycle(0xAF, 0x00).await;
    }

    pub async fn wait_proc_cont(&mut self, timeout: Duration) -> bool {
        // BBC BASIC R(&F0) masks &200000 and &100000 map to data bits 2 and 3
        // in the direct byte stream. PROCcont waits for those bits to clear.
        let deadline = Instant::now() + timeout;
        self.poll_until_deadline(0xF0, PROC_CONT_ARMED_MASK, deadline)
            .await
            && self
                .poll_until_deadline(0xF0, PROC_CONT_STARTED_MASK, deadline)
                .await
    }

    pub async fn poll_until(&mut self, addr: u8, mask: u8) {
        loop {
            let r = self.read_cycle(addr).await;
            if r & mask == 0 {
                break;
            }
        }
    }

    async fn poll_until_deadline(&mut self, addr: u8, mask: u8, deadline: Instant) -> bool {
        loop {
            let r = self.read_cycle(addr).await;
            if r & mask == 0 {
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
        }
    }

    pub async fn write_cycle(&mut self, addr: u8, data: u8) {
        let data_payload = 0xFF00_0000_u32 | (u32::from(data) << 16);
        let addr_payload = u32::from(addr) << 24;
        self.pio.write.tx().wait_push(data_payload).await;
        self.pio.control.tx().wait_push(addr_payload).await;
    }

    pub async fn read_cycle(&mut self, addr: u8) -> u8 {
        let addr_payload = 0x0001_0000_u32 | (u32::from(addr) << 24);
        self.pio.read.clear_fifos();
        self.pio.control.tx().wait_push(addr_payload).await;
        return self.pio.read.rx().wait_pull().await as u8;
    }
}
