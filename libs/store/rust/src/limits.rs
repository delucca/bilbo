use std::fmt;

/// A number outside the range its type allows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutOfRange;

impl fmt::Display for OutOfRange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("out of range")
    }
}

impl std::error::Error for OutOfRange {}

/// A zstd level, 1 to 22.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Compression(i32);

impl Compression {
    pub fn level(self) -> i32 {
        self.0
    }
}

impl TryFrom<i64> for Compression {
    type Error = OutOfRange;

    fn try_from(level: i64) -> Result<Self, OutOfRange> {
        i32::try_from(level)
            .ok()
            .filter(|level| (1..=22).contains(level))
            .map(Compression)
            .ok_or(OutOfRange)
    }
}

/// How many times the length of a file the search after a torn frame may hash, 1 to 64.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TearBudget(usize);

impl TearBudget {
    pub fn times(self) -> usize {
        self.0
    }
}

impl TryFrom<usize> for TearBudget {
    type Error = OutOfRange;

    fn try_from(times: usize) -> Result<Self, OutOfRange> {
        if (1..=64).contains(&times) {
            Ok(TearBudget(times))
        } else {
            Err(OutOfRange)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Compression, OutOfRange, TearBudget};

    #[test]
    fn a_zstd_level_is_one_to_twenty_two() {
        for level in [1, 3, 20, 22] {
            let made = Compression::try_from(level).expect("in range");
            assert_eq!(i64::from(made.level()), level);
        }
        for level in [i64::MIN, -3, 0, 23, 4_294_967_297, i64::MAX] {
            assert_eq!(Compression::try_from(level), Err(OutOfRange), "{level}");
        }
    }

    #[test]
    fn a_tear_budget_is_one_to_sixty_four_times_the_file() {
        for times in [1, 4, 64] {
            let made = TearBudget::try_from(times).expect("in range");
            assert_eq!(made.times(), times);
        }
        for times in [0, 65, usize::MAX] {
            assert_eq!(TearBudget::try_from(times), Err(OutOfRange), "{times}");
        }
    }
}
