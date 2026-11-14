const STATE_LEN: usize = 624;
const SHIFT_LEN: usize = 397;
const MATRIX_A: u32 = 0x9908_b0df;
const UPPER_MASK: u32 = 0x8000_0000;
const LOWER_MASK: u32 = 0x7fff_ffff;
const INIT_SEED: u32 = 19_650_218;
const FLOAT_HIGH_SHIFT: u32 = 5;
const FLOAT_LOW_SHIFT: u32 = 6;
const FLOAT_HIGH_SCALE: f64 = 67_108_864.0;
const FLOAT_RANGE: f64 = 9_007_199_254_740_992.0;
const OS_SEED_BYTES: usize = 32;
// random.sample switches from a pool copy to a set of picked indices past this size
const SAMPLE_POOL_BASE: usize = 21;
const SAMPLE_SET_GROWTH_BASE: f64 = 4.0;
const SAMPLE_SET_GROWTH_MIN_K: usize = 5;

pub struct PyRandom {
    state: [u32; STATE_LEN],
    index: usize,
}

impl PyRandom {
    pub fn seed(seed: u64) -> PyRandom {
        PyRandom::seed_wide(u128::from(seed))
    }

    // random.seed(n) keys init_by_array with |n| as little-endian 32-bit words, at least one
    pub fn seed_wide(seed: u128) -> PyRandom {
        let mut key = Vec::with_capacity(4);
        let mut rest = seed;
        loop {
            key.push(rest as u32);
            rest >>= 32;
            if rest == 0 {
                break;
            }
        }
        PyRandom::from_key(&key)
    }

    pub fn from_os() -> PyRandom {
        let mut bytes = [0u8; OS_SEED_BYTES];
        rand::fill(&mut bytes);
        let key: Vec<u32> = bytes
            .chunks_exact(4)
            .map(|word| u32::from_le_bytes([word[0], word[1], word[2], word[3]]))
            .collect();
        PyRandom::from_key(&key)
    }

    fn from_key(key: &[u32]) -> PyRandom {
        let mut rng = PyRandom {
            state: [0; STATE_LEN],
            index: STATE_LEN,
        };
        rng.state[0] = INIT_SEED;
        for i in 1..STATE_LEN {
            let prev = rng.state[i - 1];
            rng.state[i] = 1_812_433_253u32
                .wrapping_mul(prev ^ (prev >> 30))
                .wrapping_add(i as u32);
        }

        let mut i = 1;
        let mut j = 0;
        for _ in 0..STATE_LEN.max(key.len()) {
            let prev = rng.state[i - 1];
            rng.state[i] = (rng.state[i] ^ (prev ^ (prev >> 30)).wrapping_mul(1_664_525))
                .wrapping_add(key[j])
                .wrapping_add(j as u32);
            i += 1;
            j += 1;
            if i >= STATE_LEN {
                rng.state[0] = rng.state[STATE_LEN - 1];
                i = 1;
            }
            if j >= key.len() {
                j = 0;
            }
        }
        for _ in 0..STATE_LEN - 1 {
            let prev = rng.state[i - 1];
            rng.state[i] = (rng.state[i] ^ (prev ^ (prev >> 30)).wrapping_mul(1_566_083_941))
                .wrapping_sub(i as u32);
            i += 1;
            if i >= STATE_LEN {
                rng.state[0] = rng.state[STATE_LEN - 1];
                i = 1;
            }
        }
        rng.state[0] = UPPER_MASK;
        rng
    }

    fn next_u32(&mut self) -> u32 {
        if self.index >= STATE_LEN {
            self.twist();
        }

        let mut y = self.state[self.index];
        self.index += 1;
        y ^= y >> 11;
        y ^= (y << 7) & 0x9d2c_5680;
        y ^= (y << 15) & 0xefc6_0000;
        y ^ (y >> 18)
    }

    fn twist(&mut self) {
        for i in 0..STATE_LEN {
            let y = (self.state[i] & UPPER_MASK) | (self.state[(i + 1) % STATE_LEN] & LOWER_MASK);
            let mag = if y & 1 == 1 { MATRIX_A } else { 0 };
            self.state[i] = self.state[(i + SHIFT_LEN) % STATE_LEN] ^ (y >> 1) ^ mag;
        }
        self.index = 0;
    }

    pub fn random(&mut self) -> f64 {
        let high = f64::from(self.next_u32() >> FLOAT_HIGH_SHIFT);
        let low = f64::from(self.next_u32() >> FLOAT_LOW_SHIFT);
        (high * FLOAT_HIGH_SCALE + low) / FLOAT_RANGE
    }

    // k up to 64: the stack never asks for more
    pub fn getrandbits(&mut self, k: u32) -> u64 {
        let mut bits = 0u64;
        let mut remaining = k;
        let mut shift = 0;
        while remaining > 0 {
            let mut word = self.next_u32();
            if remaining < 32 {
                word >>= 32 - remaining;
            }
            bits |= u64::from(word) << shift;
            shift += 32;
            remaining = remaining.saturating_sub(32);
        }
        bits
    }

    pub fn randbelow(&mut self, n: u64) -> u64 {
        if n == 0 {
            return 0;
        }

        let bits = u64::BITS - n.leading_zeros();
        loop {
            let candidate = self.getrandbits(bits);
            if candidate < n {
                return candidate;
            }
        }
    }

    pub fn randint(&mut self, a: i64, b: i64) -> i64 {
        a + self.randbelow(b.abs_diff(a) + 1) as i64
    }

    pub fn uniform(&mut self, a: f64, b: f64) -> f64 {
        a + (b - a) * self.random()
    }

    pub fn choice<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.randbelow(items.len() as u64) as usize]
    }

    // random.choices(..., cum_weights=..., k=1)[0] as an index: bisect_right(cum, x, 0, len - 1)
    pub fn choices_weighted(&mut self, cum_weights: &[f64]) -> usize {
        let total = cum_weights[cum_weights.len() - 1];
        let target = self.random() * total;
        cum_weights[..cum_weights.len() - 1].partition_point(|&weight| weight <= target)
    }

    pub fn sample<T: Clone>(&mut self, items: &[T], k: usize) -> Vec<T> {
        let n = items.len();
        let mut set_size = SAMPLE_POOL_BASE;
        if k > SAMPLE_SET_GROWTH_MIN_K {
            let exponent = ((k * 3) as f64).ln() / SAMPLE_SET_GROWTH_BASE.ln();
            set_size += SAMPLE_SET_GROWTH_BASE.powi(exponent.ceil() as i32) as usize;
        }

        let mut picked = Vec::with_capacity(k);
        if n <= set_size {
            let mut pool = items.to_vec();
            for i in 0..k {
                let j = self.randbelow((n - i) as u64) as usize;
                picked.push(pool[j].clone());
                pool[j] = pool[n - i - 1].clone();
            }
        } else {
            let mut selected = Vec::with_capacity(k);
            for _ in 0..k {
                let mut j = self.randbelow(n as u64) as usize;
                while selected.contains(&j) {
                    j = self.randbelow(n as u64) as usize;
                }
                selected.push(j);
                picked.push(items[j].clone());
            }
        }
        picked
    }

    pub fn shuffle<T>(&mut self, items: &mut [T]) {
        for i in (1..items.len()).rev() {
            let j = self.randbelow(i as u64 + 1) as usize;
            items.swap(i, j);
        }
    }
}

#[cfg(test)]
mod tests;
