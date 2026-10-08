use std::ffi::CString;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

/// 卷容量快照。`available_bytes` 是可用字节（对应 `statfs` 的 `f_bavail`）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VolumeCapacity {
    pub total_bytes: u64,
    pub available_bytes: u64,
}

/// 低于此增量视为测量噪声，不作为「释放」上报。
///
/// 清理运行期间其它进程仍在写盘，容量差有抖动；太小会把噪声当成果报给用户。
pub const NOISE_FLOOR_BYTES: u64 = 4 * 1024 * 1024;

impl VolumeCapacity {
    /// 读取 `path` 所在卷的容量。失败返回 `None`（**不**伪装成 0）。
    pub fn read_for(path: &Path) -> Option<VolumeCapacity> {
        let c_path = CString::new(path.as_os_str().as_bytes()).ok()?;
        let mut buf = std::mem::MaybeUninit::<libc::statfs>::uninit();
        // SAFETY: `c_path` 是有效的 NUL 结尾 C 字符串；`buf` 是合法可写指针。
        let rc = unsafe { libc::statfs(c_path.as_ptr(), buf.as_mut_ptr()) };
        if rc != 0 {
            return None;
        }
        let stat = unsafe { buf.assume_init() };
        let block = stat.f_bsize as u64;
        Some(VolumeCapacity {
            total_bytes: stat.f_blocks.saturating_mul(block),
            available_bytes: stat.f_bavail.saturating_mul(block),
        })
    }

    /// 家目录所在卷。与清理路径同源，保证测的是同一个卷。
    pub fn read() -> Option<VolumeCapacity> {
        Self::read_for(&crate::folder_access::scanner_home())
    }
}

/// 两次读数的可用空间增量。任一次缺失、空间反而变小、或增量低于噪声下限，
/// 都返回 `None` —— 表示「测不出可报告的释放量」，与「释放 0」不同。
pub fn reclaimed(before: Option<VolumeCapacity>, after: Option<VolumeCapacity>) -> Option<u64> {
    let before = before?;
    let after = after?;
    let delta = after.available_bytes.checked_sub(before.available_bytes)?;
    (delta >= NOISE_FLOOR_BYTES).then_some(delta)
}

#[cfg(test)]
#[path = "volume_tests.rs"]
mod tests;
