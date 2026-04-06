use std::io;
use std::io::{ErrorKind, Read, Write};

use rp2040_fred_protocol::bridge_proto::TraceSamples;

const CAPTURE_MAGIC: [u8; 8] = *b"FREDCAP\0";
const CAPTURE_VERSION: u32 = 1;
const RESERVED: u32 = 0;
const MAX_BATCH_SAMPLES: usize = 4096;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CaptureBatch {
    pub dropped_samples_total: u32,
    pub rx_stall_count_total: u32,
    pub samples: Vec<u32>,
}

pub struct CaptureWriter<W> {
    inner: W,
}

impl<W: Write> CaptureWriter<W> {
    pub fn new(mut inner: W) -> io::Result<Self> {
        inner.write_all(&CAPTURE_MAGIC)?;
        inner.write_all(&CAPTURE_VERSION.to_le_bytes())?;
        inner.write_all(&RESERVED.to_le_bytes())?;
        Ok(Self { inner })
    }

    pub fn write_trace(&mut self, trace: TraceSamples<'_>) -> io::Result<()> {
        let sample_count = u32::try_from(trace.sample_count()).map_err(|_| {
            io::Error::new(ErrorKind::InvalidInput, "too many samples in capture batch")
        })?;

        self.inner
            .write_all(&trace.dropped_samples_total.to_le_bytes())?;
        self.inner
            .write_all(&trace.rx_stall_count_total.to_le_bytes())?;
        self.inner.write_all(&sample_count.to_le_bytes())?;
        for sample in trace.iter_samples() {
            self.inner.write_all(&sample.to_le_bytes())?;
        }
        Ok(())
    }

    pub fn write_batch(
        &mut self,
        dropped_samples_total: u32,
        rx_stall_count_total: u32,
        samples: &[u32],
    ) -> io::Result<()> {
        let sample_count = u32::try_from(samples.len()).map_err(|_| {
            io::Error::new(ErrorKind::InvalidInput, "too many samples in capture batch")
        })?;

        self.inner.write_all(&dropped_samples_total.to_le_bytes())?;
        self.inner.write_all(&rx_stall_count_total.to_le_bytes())?;
        self.inner.write_all(&sample_count.to_le_bytes())?;
        for sample in samples {
            self.inner.write_all(&sample.to_le_bytes())?;
        }
        Ok(())
    }
}

pub struct CaptureReader<R> {
    inner: R,
}

impl<R: Read> CaptureReader<R> {
    pub fn new(mut inner: R) -> io::Result<Self> {
        let mut magic = [0u8; 8];
        inner.read_exact(&mut magic)?;
        if magic != CAPTURE_MAGIC {
            return Err(io::Error::new(
                ErrorKind::InvalidData,
                format!("bad capture magic: {:?}", String::from_utf8_lossy(&magic)),
            ));
        }

        let version = read_u32(&mut inner)?;
        if version != CAPTURE_VERSION {
            return Err(io::Error::new(
                ErrorKind::InvalidData,
                format!("unsupported capture version: {version}"),
            ));
        }

        let _reserved = read_u32(&mut inner)?;
        Ok(Self { inner })
    }

    pub fn read_batch(&mut self) -> io::Result<Option<CaptureBatch>> {
        let Some(dropped_samples_total) = read_u32_or_eof(&mut self.inner)? else {
            return Ok(None);
        };
        let rx_stall_count_total = read_u32(&mut self.inner)?;
        let sample_count = read_u32(&mut self.inner)? as usize;
        if sample_count > MAX_BATCH_SAMPLES {
            return Err(io::Error::new(
                ErrorKind::InvalidData,
                format!("capture batch sample_count too large: {sample_count}"),
            ));
        }

        let mut samples = Vec::with_capacity(sample_count);
        for _ in 0..sample_count {
            samples.push(read_u32(&mut self.inner)?);
        }

        Ok(Some(CaptureBatch {
            dropped_samples_total,
            rx_stall_count_total,
            samples,
        }))
    }
}

fn read_u32<R: Read>(inner: &mut R) -> io::Result<u32> {
    let mut buf = [0u8; 4];
    inner.read_exact(&mut buf)?;
    Ok(u32::from_le_bytes(buf))
}

fn read_u32_or_eof<R: Read>(inner: &mut R) -> io::Result<Option<u32>> {
    let mut buf = [0u8; 4];
    let mut filled = 0usize;

    while filled < buf.len() {
        match inner.read(&mut buf[filled..])? {
            0 if filled == 0 => return Ok(None),
            0 => {
                return Err(io::Error::new(
                    ErrorKind::UnexpectedEof,
                    "truncated capture batch header",
                ));
            }
            n => filled += n,
        }
    }

    Ok(Some(u32::from_le_bytes(buf)))
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::{CaptureReader, CaptureWriter};

    #[test]
    fn roundtrip_capture_batches() {
        let mut bytes = Vec::new();
        {
            let mut writer = CaptureWriter::new(&mut bytes).expect("writer");
            writer
                .write_batch(12, 3, &[0x0003_8003, 0x0003_F132, 0x0003_8002])
                .expect("batch 1");
            writer.write_batch(19, 4, &[0x0003_F107]).expect("batch 2");
        }

        let mut reader = CaptureReader::new(Cursor::new(bytes)).expect("reader");
        let batch1 = reader.read_batch().expect("read 1").expect("batch 1");
        assert_eq!(batch1.dropped_samples_total, 12);
        assert_eq!(batch1.rx_stall_count_total, 3);
        assert_eq!(batch1.samples.len(), 3);

        let batch2 = reader.read_batch().expect("read 2").expect("batch 2");
        assert_eq!(batch2.dropped_samples_total, 19);
        assert_eq!(batch2.rx_stall_count_total, 4);
        assert_eq!(batch2.samples, vec![0x0003_F107]);

        assert!(reader.read_batch().expect("read eof").is_none());
    }
}
