use crate::transport::pio::master::ThisMasterPio;

pub struct Bus<'a> {
    pub pio: ThisMasterPio<'a>
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

    pub async fn poll_until(&mut self, addr: u8, mask: u8) {
        let addr_payload = 0x0001_0000_u32 | (u32::from(addr) << 24);
        self.pio.read.clear_fifos();
        loop {
            self.pio.control.tx().wait_push(addr_payload).await;
            if let Some(r) = self.pio.read.rx().try_pull() {
                if (r as u8) & mask == 0 {
                    break;
                }
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