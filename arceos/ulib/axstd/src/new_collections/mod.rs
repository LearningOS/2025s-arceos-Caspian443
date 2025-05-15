//! axstd 的集合类型。

// 声明 hash_map 子模块。Rust 会在同级目录下查找 hash_map.rs 或 hash_map/mod.rs。
#[cfg(feature = "alloc")] // 仅当 alloc feature 启用时编译此模块
pub mod hashmap;

// 从子模块中重新导出 HashMap，这样用户可以使用 axstd::collections::HashMap。
#[cfg(feature = "alloc")]
pub use self::hashmap::HashMap;

// 如果你的 AxRandomState 需要在外部被直接使用（例如，如果用户想用 HashMap::with_hasher(AxRandomState)），
// 你也可以在这里导出它。对于本实验，可能不需要。
// #[cfg(feature = "alloc")]
// pub use self::hash_map::AxRandomState;