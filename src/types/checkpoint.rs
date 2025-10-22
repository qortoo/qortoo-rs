use std::fmt::Display;

#[derive(Debug, Default, PartialEq, Eq, Copy, Clone)]
pub struct CheckPoint {
    pub sseq: u64,
    pub cseq: u64,
}

impl CheckPoint {
    pub fn new(sseq: u64, cseq: u64) -> Self {
        Self { sseq, cseq }
    }
}

impl Display for CheckPoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_fmt(format_args!("(s:{}, c:{})", self.sseq, self.cseq))
    }
}

#[cfg(test)]
mod tests_checkpoint {
    use tracing::info;

    use crate::types::checkpoint::CheckPoint;

    #[test]
    fn can_use_checkpoint() {
        let cp1 = CheckPoint::default();
        let cp2 = CheckPoint::new(100, 101);
        info!("{cp1}, {cp2:?}");
        assert_eq!(cp1.to_string(), "(s:0, c:0)");
        assert_eq!(cp2.to_string(), "(s:100, c:101)");
        assert_eq!(cp1, cp1);
        assert_ne!(cp1, cp2);
    }
}
