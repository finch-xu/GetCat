//! 纯文件持久化：版本化 JSON、路径布局、原子写、合并写队列、启动读取与损坏隔离（spec §9）。
//! 不存历史、不存响应。

pub mod codec;
pub mod disk;

pub use codec::{FORMAT_VERSION, StoreError};
pub use disk::{DRAFTS_DIR, Layout, LoadError, Loaded, REQUESTS_DIR, WORKSPACE_FILE, load_all};
