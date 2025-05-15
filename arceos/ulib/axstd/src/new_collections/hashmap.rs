use core::borrow::Borrow;
use core::hash::{BuildHasher, Hash, Hasher};
use core::mem;

#[cfg(feature = "alloc")]
use alloc::vec::Vec;

// 假设的 API 路径，如果你的项目中不同，请修改
// 例如，如果 axstd 可以直接访问 axhal: use axhal::random::random_u64 as ax_rand_u64;
// *** 请务必确认此路径或替换为正确的随机数函数路径 ***
// 默认情况下，ArceOS 的 API 通常通过 arceos_api 模块暴露
use arceos_api::sys::ax_rand_u64;

// 默认初始容量，最好是2的幂
const INITIAL_CAPACITY: usize = 8;
// 默认负载因子阈值
const LOAD_FACTOR_THRESHOLD: f32 = 0.75;

// --- Hasher 和 BuildHasher 实现 ---

/// 自定义的简单哈希状态构建器，使用 axhal 的随机数
#[derive(Clone, Default)]
pub struct AxRandomState;

impl AxRandomState {
    pub fn new() -> Self {
        AxRandomState
    }
}

/// 一个非常基础的哈希器实现
pub struct SimpleHasher {
    state: u64,
}

impl SimpleHasher {
    fn new(seed: u64) -> Self {
        // 使用种子初始化状态，这里用一个简单的方式
        // FNV-1a offset basis，加上种子扰动
        // 这种简单的哈希对于生产环境不够安全，但对于实验足够
        let mut state = 0xcbf29ce484222325_u64.wrapping_add(seed);
        state = state.wrapping_mul(0x100000001b3_u64); // FNV prime
        SimpleHasher { state }
    }
}

impl Hasher for SimpleHasher {
    fn finish(&self) -> u64 {
        // 可以添加一个最终的混淆步骤
        let mut x = self.state;
        x ^= x >> 30;
        x = x.wrapping_mul(0xbf58476d1ce4e5b9_u64);
        x ^= x >> 27;
        x = x.wrapping_mul(0x94d049bb133111eb_u64);
        x ^= x >> 31;
        x
    }

    fn write(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            self.state = self.state.wrapping_mul(0x100000001b3_u64); // FNV prime
            self.state ^= byte as u64;
        }
    }
}

impl BuildHasher for AxRandomState {
    type Hasher = SimpleHasher;

    fn build_hasher(&self) -> Self::Hasher {
        // 确认这里调用的是你新导出的函数
        SimpleHasher::new(ax_rand_u64()) // ax_rand_u64() 现在应该能被解析
    }
}

// --- Bucket 和 HashMap 实现 ---

struct Bucket<K, V> {
    items: Vec<(K, V)>, // 使用 Vec 模拟链表
}

impl<K, V> Bucket<K, V> {
    fn new() -> Self {
        Bucket { items: Vec::new() }
    }
}

pub struct HashMap<K, V, S = AxRandomState> {
    buckets: Vec<Bucket<K, V>>,
    len: usize,
    hasher_builder: S,
}

impl<K, V> HashMap<K, V, AxRandomState>
where
    K: Hash + Eq,
{
    /// 创建一个新的、空的 HashMap。
    /// 它将使用 AxRandomState 从 axhal 获取随机性。
    #[cfg(feature = "alloc")]
    pub fn new() -> Self {
        Self::with_capacity_and_hasher(INITIAL_CAPACITY, AxRandomState::new())
    }
}

impl<K, V, S> HashMap<K, V, S>
where
    K: Hash + Eq,
    S: BuildHasher,
{
    #[cfg(feature = "alloc")]
    fn with_capacity_and_hasher(capacity: usize, hasher_builder: S) -> Self {
        let cap = usize::max(INITIAL_CAPACITY, capacity.next_power_of_two());
        let mut buckets = Vec::with_capacity(cap);
        for _ in 0..cap {
            buckets.push(Bucket::new());
        }
        HashMap {
            buckets,
            len: 0,
            hasher_builder,
        }
    }

    fn make_hash<Q: ?Sized>(&self, key: &Q) -> u64
    where
        K: Borrow<Q>,
        Q: Hash,
    {
        let mut hasher = self.hasher_builder.build_hasher();
        key.hash(&mut hasher);
        hasher.finish()
    }

    fn bucket_index(&self, hash: u64) -> usize {
        if self.buckets.is_empty() { // 防止除以零或对空桶取模
            return 0;
        }
        // 确保桶的数量是2的幂，这样可以用位运算代替取模
        (hash & (self.buckets.len() as u64 - 1)) as usize
    }

    fn resize_if_needed(&mut self) {
        if self.buckets.is_empty() {
            // 初始化情况
            let mut new_buckets_vec = Vec::with_capacity(INITIAL_CAPACITY);
            for _ in 0..INITIAL_CAPACITY {
                new_buckets_vec.push(Bucket::new());
            }
            self.buckets = new_buckets_vec;
            return;
        }

        let load_factor = self.len as f32 / self.buckets.len() as f32;
        if load_factor > LOAD_FACTOR_THRESHOLD && self.buckets.len() > 0 {
            self.resize();
        }
    }

    fn resize(&mut self) {
        let current_capacity = self.buckets.len();
        let new_capacity = if current_capacity == 0 {
            INITIAL_CAPACITY
        } else {
            current_capacity.saturating_mul(2)
        };

        if new_capacity == current_capacity { // 如果容量没有变化 (例如已经达到最大或溢出)
            return;
        }

        let mut new_buckets_vec = Vec::with_capacity(new_capacity);
        for _ in 0..new_capacity {
            new_buckets_vec.push(Bucket::new());
        }
        
        let old_buckets = mem::replace(&mut self.buckets, new_buckets_vec);
        self.len = 0; // 长度将在重新插入时更新

        for bucket_node in old_buckets {
            for (key, value) in bucket_node.items { // items 是 Vec，可以直接迭代消耗
                // 直接调用内部的插入逻辑，避免再次触发 resize 检查
                // 注意：这里的 `make_hash` 和 `bucket_index` 都是在 `self` (即新表) 上操作的
                let hash = self.make_hash(&key);
                let index = self.bucket_index(hash);
                self.buckets[index].items.push((key, value));
                self.len += 1;
            }
        }
    }
    
    /// 插入一个键值对到 HashMap 中。
    /// 如果键已存在，则更新其值，并返回旧值。否则，返回 `None`。
    pub fn insert(&mut self, key: K, value: V) -> Option<V> {
        self.resize_if_needed();
        
        let hash = self.make_hash(&key);
        let index = self.bucket_index(hash);

        // 确保 resize_if_needed 之后 buckets 不会为空
        if self.buckets.is_empty() {
             // 这是一个理论上的防护，resize_if_needed 应该已经处理了空桶的情况
            self.resize_if_needed(); // 再次尝试初始化
             if self.buckets.is_empty() { // 如果还是空，则无法继续
                 // 在 no_std 环境下，panic 可能不是最好的选择，但这里为了简单
                 // 或者可以返回一个错误类型，但这会改变函数签名
                 panic!("Failed to initialize buckets for HashMap");
             }
        }


        let bucket = &mut self.buckets[index];
        for item in bucket.items.iter_mut() {
            if item.0 == key { // K 必须实现 Eq
                return Some(mem::replace(&mut item.1, value));
            }
        }

        bucket.items.push((key, value));
        self.len += 1;
        None
    }

    /// 返回一个迭代器，用于遍历 HashMap 中的所有键值对。
    pub fn iter(&self) -> Iter<'_, K, V, S> {
        Iter::new(self)
    }

    // 为完整性添加 get, len, is_empty (实验可能不直接测试这些，但好的 HashMap 应该有)
    pub fn get<Q: ?Sized>(&self, key: &Q) -> Option<&V>
    where
        K: Borrow<Q>,
        Q: Hash + Eq,
    {
        if self.is_empty() || self.buckets.is_empty() { return None; }
        let hash = self.make_hash(key);
        let index = self.bucket_index(hash);

        for (k_ref, v_ref) in self.buckets[index].items.iter() {
            if key.eq(k_ref.borrow()) { // K: Borrow<Q>, Q: Eq
                return Some(v_ref);
            }
        }
        None
    }
    
    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

// --- Iter 实现 ---
pub struct Iter<'a, K: 'a, V: 'a, S: BuildHasher + 'a> {
    map_buckets: &'a Vec<Bucket<K, V>>,
    current_bucket_idx: usize,
    current_item_idx_in_bucket: usize,
    _hasher_builder_marker: core::marker::PhantomData<&'a S>,
}

impl<'a, K, V, S: BuildHasher> Iter<'a, K, V, S> {
    fn new(map: &'a HashMap<K, V, S>) -> Self {
        Iter {
            map_buckets: &map.buckets,
            current_bucket_idx: 0,
            current_item_idx_in_bucket: 0,
            _hasher_builder_marker: core::marker::PhantomData,
        }
    }
}

impl<'a, K, V, S: BuildHasher> Iterator for Iter<'a, K, V, S>
where
    K: 'a,
    V: 'a,
    S: BuildHasher + 'a,
{
    type Item = (&'a K, &'a V);

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if self.current_bucket_idx >= self.map_buckets.len() {
                return None;
            }

            let current_bucket_items = &self.map_buckets[self.current_bucket_idx].items;
            
            if self.current_item_idx_in_bucket < current_bucket_items.len() {
                let (key, value) = &current_bucket_items[self.current_item_idx_in_bucket];
                self.current_item_idx_in_bucket += 1;
                return Some((key, value));
            } else {
                self.current_bucket_idx += 1;
                self.current_item_idx_in_bucket = 0; 
            }
        }
    }
}
