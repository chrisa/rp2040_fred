use crate::transport::pio::master::ThisMasterPio;
use embassy_time::Instant;
use rp2040_fred_protocol::bridge_proto::CommandBlock;

const WRITE_READY_MASK: u8 = 0x01;
pub const CYCLE_START_MASK: u8 = 0x10;
pub const PROC_BUSY_MASK: u8 = 0x80;

pub struct Bus<'a> {
    pub pio: ThisMasterPio<'a>,
}

impl<'a> Bus<'a> {
    pub async fn command_cycle(&mut self, cmd: u8) -> u8 {
        // 1. Poll `F0` until bit 0 clears.
        // 2. Write one command byte to `80`.
        // 3. Poll `F0` again until bit 0 clears.
        // 4. Read one response byte from `F1`.
        self.poll_until(0xF0, WRITE_READY_MASK).await;
        self.write_cycle(0x80, cmd).await;
        self.poll_until(0xF0, WRITE_READY_MASK).await;
        return self.read_cycle(0xF1).await;
    }

    pub async fn command_cycle_deadline(&mut self, cmd: u8, deadline: Instant) -> Option<u8> {
        if !self
            .poll_until_deadline(0xF0, WRITE_READY_MASK, deadline)
            .await
        {
            return None;
        }
        self.write_cycle(0x80, cmd).await;
        if !self
            .poll_until_deadline(0xF0, WRITE_READY_MASK, deadline)
            .await
        {
            return None;
        }
        Some(self.read_cycle(0xF1).await)
    }

    pub async fn read_write_zero_pair(&mut self, addr: u8) {
        self.read_cycle(addr).await;
        self.write_cycle(addr, 0x00).await;
    }

    pub async fn write_command_block(&mut self, block: CommandBlock) {
        let payload = block.to_payload();
        for (offset, data) in payload.iter().enumerate() {
            self.write_register_gated(0x92 + offset as u8, *data).await;
        }
    }

    pub async fn clear_command_block(&mut self) {
        self.write_register_gated(0xAF, 0x00).await;
    }

    async fn write_register_gated(&mut self, addr: u8, data: u8) {
        self.poll_until(0xF0, WRITE_READY_MASK).await;
        self.read_cycle(addr).await;
        self.write_cycle(addr, data).await;
    }

    pub async fn read_status(&mut self) -> u8 {
        self.read_cycle(0xF0).await
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
