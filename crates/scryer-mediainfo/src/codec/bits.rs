//! Bit readers over byte slices, plus a writer for building test bitstreams.

pub(crate) struct AudioBitReader<'a> {
    data: &'a [u8],
    pub(crate) bit_pos: usize,
}

impl<'a> AudioBitReader<'a> {
    pub(crate) fn new(data: &'a [u8]) -> Self {
        Self { data, bit_pos: 0 }
    }

    pub(crate) fn read_bit(&mut self) -> Option<u8> {
        Some(self.read_bits(1)? as u8)
    }

    pub(crate) fn read_bits(&mut self, count: usize) -> Option<u32> {
        if count == 0 || count > 32 || self.bit_pos + count > self.data.len() * 8 {
            return None;
        }

        let mut value = 0_u32;
        for _ in 0..count {
            let byte_index = self.bit_pos / 8;
            let bit_index = 7 - (self.bit_pos % 8);
            value = (value << 1) | u32::from((self.data[byte_index] >> bit_index) & 0x01);
            self.bit_pos += 1;
        }
        Some(value)
    }

    pub(crate) fn peek_bits(&self, count: usize) -> Option<u32> {
        if count == 0 || count > 32 || self.bit_pos + count > self.data.len() * 8 {
            return None;
        }

        let mut value = 0_u32;
        for bit_pos in self.bit_pos..(self.bit_pos + count) {
            let byte_index = bit_pos / 8;
            let bit_index = 7 - (bit_pos % 8);
            value = (value << 1) | u32::from((self.data[byte_index] >> bit_index) & 0x01);
        }
        Some(value)
    }

    pub(crate) fn skip_bits(&mut self, count: usize) -> Option<()> {
        if self.bit_pos + count > self.data.len() * 8 {
            return None;
        }
        self.bit_pos += count;
        Some(())
    }

    pub(crate) fn bits_left(&self) -> usize {
        self.data.len() * 8 - self.bit_pos
    }

    pub(crate) fn read_aac_audio_object_type(&mut self) -> Option<u8> {
        let object_type = self.read_bits(5)? as u8;
        if object_type == 31 {
            Some(32 + self.read_bits(6)? as u8)
        } else {
            Some(object_type)
        }
    }

    pub(crate) fn read_aac_sample_rate(&mut self) -> Option<u32> {
        const AAC_SAMPLE_RATES: [u32; 16] = [
            96_000, 88_200, 64_000, 48_000, 44_100, 32_000, 24_000, 22_050, 16_000, 12_000, 11_025,
            8_000, 7_350, 0, 0, 0,
        ];

        let sample_rate_index = self.read_bits(4)? as usize;
        if sample_rate_index == 0xF {
            self.read_bits(24)
        } else {
            AAC_SAMPLE_RATES.get(sample_rate_index).copied()
        }
    }
}

pub(crate) struct BitReader<'a> {
    data: &'a [u8],
    bit_pos: usize,
}

impl<'a> BitReader<'a> {
    pub(crate) fn new(data: &'a [u8]) -> Self {
        Self { data, bit_pos: 0 }
    }

    pub(crate) fn read_bit(&mut self) -> Option<u8> {
        Some(self.read_bits(1)? as u8)
    }

    pub(crate) fn read_bits(&mut self, count: usize) -> Option<u32> {
        if count == 0 || count > 32 || self.bit_pos + count > self.data.len() * 8 {
            return None;
        }

        let mut value = 0_u32;
        for _ in 0..count {
            let byte_index = self.bit_pos / 8;
            let bit_index = 7 - (self.bit_pos % 8);
            value = (value << 1) | u32::from((self.data[byte_index] >> bit_index) & 0x01);
            self.bit_pos += 1;
        }

        Some(value)
    }

    pub(crate) fn skip_bits(&mut self, count: usize) -> Option<()> {
        if self.bit_pos + count > self.data.len() * 8 {
            return None;
        }
        self.bit_pos += count;
        Some(())
    }
}

#[cfg(test)]
pub(crate) struct TestBitWriter {
    pub(crate) bytes: Vec<u8>,
    bit_pos: usize,
}

#[cfg(test)]
impl TestBitWriter {
    pub(crate) fn new() -> Self {
        Self {
            bytes: Vec::new(),
            bit_pos: 0,
        }
    }

    pub(crate) fn write_bits(&mut self, value: u64, count: usize) {
        for shift in (0..count).rev() {
            let bit = ((value >> shift) & 1) as u8;
            if self.bit_pos.is_multiple_of(8) {
                self.bytes.push(0);
            }
            let byte_index = self.bit_pos / 8;
            self.bytes[byte_index] |= bit << (7 - (self.bit_pos % 8));
            self.bit_pos += 1;
        }
    }

    pub(crate) fn finish(self) -> Vec<u8> {
        self.bytes
    }
}
