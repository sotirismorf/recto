use std::fmt;

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct PageId(u64);

impl PageId {
    pub fn new() -> Self {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(1);
        Self(NEXT.fetch_add(1, Ordering::Relaxed))
    }

    pub fn as_u64(self) -> u64 {
        self.0
    }
}

impl From<PageId> for u64 {
    fn from(id: PageId) -> Self {
        id.0
    }
}

impl fmt::Display for PageId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[allow(dead_code)]
pub struct RequestId(u64);

impl RequestId {
    #[allow(dead_code)]
    pub fn as_u64(self) -> u64 {
        self.0
    }
}

impl std::ops::Add<u64> for RequestId {
    type Output = Self;
    fn add(self, rhs: u64) -> Self {
        Self(self.0.wrapping_add(rhs))
    }
}

#[derive(Debug, thiserror::Error)]
#[allow(dead_code)]
pub enum AppError {
    #[error("Failed to load project")]
    ProjectLoad(#[source] anyhow::Error),

    #[error("Failed to save project")]
    ProjectSave(#[source] anyhow::Error),

    #[error("Failed to export")]
    Export(#[source] anyhow::Error),
}
