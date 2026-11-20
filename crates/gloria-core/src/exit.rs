//! Process exit codes (docs/design.md §11).

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum Exit {
    Ok = 0,
    Error = 1,
    DowngradeRefused = 2,
    FilesLocked = 3,
    ElevationFailed = 4,
    WebView2Failed = 5,
    InvalidArgs = 6,
}

impl Exit {
    pub fn code(self) -> i32 {
        self as i32
    }
}
