pub fn widen(lo: u32, hi: u32) -> u64 {
    u64::from(lo) | (u64::from(hi) << 32)
}

pub fn narrow(v: u64) -> (u32, u32) {
    ((v & 0xffff_ffff) as u32, (v >> 32) as u32)
}

pub const LIMITS: [(u32, u32); 4] =
    [(0, 1), (1, 16), (16, 256), (256, 4096)];
