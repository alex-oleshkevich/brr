use test_case::test_case;

use super::PyRandom;

// Every literal below comes from CPython 3.14 (python3 -c):
//   r = random.Random(); r.seed(42)
//   [r.random() for _ in range(3)]; [r.randint(1, 6) for _ in range(5)]; r._randbelow(10**12)
//   r.sample(range(100), 5); l = list(range(10)); r.shuffle(l)
//   [r.choices(range(3), cum_weights=[1, 4, 10])[0] for _ in range(5)]
//   r.seed(2**64 + 1); [r.random() for _ in range(2)]; r.getrandbits(64); r.getrandbits(40); r.getrandbits(7)
//   r.seed(42); r.sample(range(1000), 30)[:8]; r.seed(42); r.sample(range(10**6), 3)
//   r.seed(42); r.uniform(2.0, 5.0); r.seed(42); r.choice(["a", "b", "c", "d"]); r.seed(0); r.random()
const SEED: u64 = 42;
const FIRST_RANDOMS: [f64; 3] = [
    0.6394267984578837,
    0.025010755222666936,
    0.27502931836911926,
];
const RANDINTS: [i64; 5] = [2, 2, 6, 1, 6];
const RANDBELOW_TRILLION: u64 = 982_433_687_219;
const SAMPLE_OF_HUNDRED: [u32; 5] = [69, 11, 75, 54, 4];
const SHUFFLED_TEN: [u32; 10] = [6, 7, 2, 9, 5, 4, 8, 3, 1, 0];
const WEIGHTED_PICKS: [usize; 5] = [2, 2, 2, 2, 1];
const CUMULATIVE_WEIGHTS: [f64; 3] = [1.0, 4.0, 10.0];
const WIDE_SEED: u128 = (1 << 64) + 1;
const WIDE_RANDOMS: [f64; 2] = [0.10175875467846374, 0.7248101788369984];
const WIDE_BITS: [(u32, u64); 3] = [
    (64, 16_607_987_232_983_196_857),
    (40, 737_092_916_894),
    (7, 3),
];
const ZERO_SEED_RANDOM: f64 = 0.8444218515250481;
const POOL_SAMPLE_OF_THIRTY_HEAD: [u32; 8] = [654, 114, 25, 759, 281, 250, 228, 142];
const SET_SAMPLE_OF_THREE: [u32; 3] = [670_487, 116_739, 26_225];
const UNIFORM_2_5: f64 = 3.9182803953736514;
const CHOICE_FROM_ABCD: &str = "a";

#[test]
fn sequence_matches_cpython() {
    let mut rng = PyRandom::seed(SEED);

    let randoms: Vec<f64> = (0..3).map(|_| rng.random()).collect();
    assert_eq!(randoms, FIRST_RANDOMS);

    let randints: Vec<i64> = (0..5).map(|_| rng.randint(1, 6)).collect();
    assert_eq!(randints, RANDINTS);

    assert_eq!(rng.randbelow(1_000_000_000_000), RANDBELOW_TRILLION);

    let hundred: Vec<u32> = (0..100).collect();
    assert_eq!(rng.sample(&hundred, 5), SAMPLE_OF_HUNDRED);

    let mut ten: Vec<u32> = (0..10).collect();
    rng.shuffle(&mut ten);
    assert_eq!(ten, SHUFFLED_TEN);

    let picks: Vec<usize> = (0..5)
        .map(|_| rng.choices_weighted(&CUMULATIVE_WEIGHTS))
        .collect();
    assert_eq!(picks, WEIGHTED_PICKS);
}

#[test]
fn seed_wider_than_64_bits_splits_into_words() {
    let mut rng = PyRandom::seed_wide(WIDE_SEED);

    let randoms: Vec<f64> = (0..2).map(|_| rng.random()).collect();
    assert_eq!(randoms, WIDE_RANDOMS);

    // getrandbits spans two words at 64 and 40 bits and shifts the last word at 7
    for (bits, expected) in WIDE_BITS {
        assert_eq!(rng.getrandbits(bits), expected);
    }
}

#[test]
fn seed_zero_has_one_key_word() {
    assert_eq!(PyRandom::seed(0).random(), ZERO_SEED_RANDOM);
}

#[test_case(30, 1000, &POOL_SAMPLE_OF_THIRTY_HEAD; "pool past six items")]
#[test_case(3, 1_000_000, &SET_SAMPLE_OF_THREE; "set when the population is large")]
fn sample_switches_between_pool_and_set(k: usize, n: u32, expected_head: &[u32]) {
    let items: Vec<u32> = (0..n).collect();

    let picked = PyRandom::seed(SEED).sample(&items, k);

    assert_eq!(&picked[..expected_head.len()], expected_head);
}

#[test]
fn uniform_and_choice_follow_random() {
    assert_eq!(PyRandom::seed(SEED).uniform(2.0, 5.0), UNIFORM_2_5);
    assert_eq!(
        *PyRandom::seed(SEED).choice(&["a", "b", "c", "d"]),
        CHOICE_FROM_ABCD
    );
}

#[test]
fn from_os_seeds_independently() {
    assert_ne!(PyRandom::from_os().random(), PyRandom::from_os().random());
}
