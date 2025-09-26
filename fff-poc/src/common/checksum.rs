use xxhash_rust::xxh64::Xxh64;

#[repr(u8)]
#[derive(PartialEq, Debug, Clone, Copy)]
pub enum ChecksumType {
    XxHash,
}

impl From<u8> for ChecksumType {
    fn from(v: u8) -> ChecksumType {
        match v {
            0 => ChecksumType::XxHash,
            _ => panic!("Invalid checksum type"),
        }
    }
}

pub trait Checksum {
    fn update(&mut self, data: &[u8]);
    fn finalize(&self) -> u64;
    fn reset(&mut self);
}

#[derive(Default)]
pub struct XxHash {
    state: Xxh64,
}

impl Checksum for XxHash {
    fn update(&mut self, data: &[u8]) {
        self.state.update(data);
    }

    fn finalize(&self) -> u64 {
        self.state.digest()
    }

    fn reset(&mut self) {
        self.state = Xxh64::default()
    }
}

pub fn create_checksum(checksum_type: &ChecksumType) -> Box<dyn Checksum> {
    match checksum_type {
        ChecksumType::XxHash => Box::new(XxHash::default()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_xxhash() {
        let mut checksum = create_checksum(&ChecksumType::XxHash);
        checksum.update(b"helloworld");
        let c1 = checksum.finalize();

        let mut checksum = create_checksum(&ChecksumType::XxHash);
        checksum.update(b"hello");
        checksum.update(b"world");
        let c2 = checksum.finalize();
        assert_eq!(c1, c2);

        let mut checksum = create_checksum(&ChecksumType::XxHash);
        checksum.update(b"hell");
        checksum.update(b"oworld");
        let c3 = checksum.finalize();

        assert_eq!(c1, c3);

        let mut checksum = create_checksum(&ChecksumType::XxHash);
        checksum.update(b"oworld");
        checksum.update(b"hell");
        let c4 = checksum.finalize();
        assert_ne!(c3, c4);
    }
}
