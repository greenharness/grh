//! grh's exit codes: one table, a public interface. Changing a code is a major
//! version. grh never picks 126, 127 or 128+n itself; a dispatched part's own
//! status passes through untouched.

use std::process::ExitCode;

#[derive(Debug, PartialEq, Eq)]
pub enum Exit {
    /// 1: a step failed; the receipt keeps the failed entries.
    Failed(String),
    /// 2: usage, or a confirmation was needed and not given.
    Usage(String),
    /// 3: a part is not published yet (design today); the others installed.
    Unpublished(String),
    /// 4: the dispatched part is not installed.
    NotInstalled(String),
    /// 5: another grh holds the receipt lock.
    Locked(String),
    /// 130: interrupted.
    Interrupted,
}

impl Exit {
    pub fn code(&self) -> u8 {
        match self {
            Exit::Failed(_) => 1,
            Exit::Usage(_) => 2,
            Exit::Unpublished(_) => 3,
            Exit::NotInstalled(_) => 4,
            Exit::Locked(_) => 5,
            Exit::Interrupted => 130,
        }
    }

    /// When several apply, the lowest non-zero code wins: a failed step plus
    /// an unpublished part exits 1.
    pub fn worst(a: Exit, b: Exit) -> Exit {
        if b.code() < a.code() {
            b
        } else {
            a
        }
    }

    pub fn report(self) -> ExitCode {
        let code = self.code();
        match &self {
            Exit::Failed(m)
            | Exit::Usage(m)
            | Exit::Unpublished(m)
            | Exit::NotInstalled(m)
            | Exit::Locked(m) => eprintln!("grh: {m}"),
            Exit::Interrupted => eprintln!("grh: interrupted"),
        }
        ExitCode::from(code)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_match_the_published_table() {
        assert_eq!(Exit::Failed(String::new()).code(), 1);
        assert_eq!(Exit::Usage(String::new()).code(), 2);
        assert_eq!(Exit::Unpublished(String::new()).code(), 3);
        assert_eq!(Exit::NotInstalled(String::new()).code(), 4);
        assert_eq!(Exit::Locked(String::new()).code(), 5);
        assert_eq!(Exit::Interrupted.code(), 130);
    }

    #[test]
    fn the_lowest_non_zero_code_wins() {
        let w = Exit::worst(Exit::Unpublished("d".into()), Exit::Failed("f".into()));
        assert_eq!(w.code(), 1);
    }
}
