#![allow(dead_code)]

use core::ffi::{c_void, c_char, c_int};
use axhal::arch::TrapFrame;
use axhal::trap::{register_trap_handler, SYSCALL};
use axerrno::LinuxError;
use axtask::current;
use axtask::TaskExtRef;
use axhal::paging::MappingFlags;
use arceos_posix_api as api;

// 内存管理相关
use alloc::vec;
use alloc::vec::Vec;
use axhal::mem::{PAGE_SIZE_4K, phys_to_virt};
use alloc::sync::Arc;
use arceos_posix_api::imp::fd_ops::{get_file_like, FileLike};


// 文件操作相关（根据你的项目实际情况调整）
use axstd::fs::File; // 如果你用 axstd 文件系统
// use crate::your_file_table::get_file_by_fd; // 如果有自定义文件表


const SYS_IOCTL: usize = 29;
const SYS_OPENAT: usize = 56;
const SYS_CLOSE: usize = 57;
const SYS_READ: usize = 63;
const SYS_WRITE: usize = 64;
const SYS_WRITEV: usize = 66;
const SYS_EXIT: usize = 93;
const SYS_EXIT_GROUP: usize = 94;
const SYS_SET_TID_ADDRESS: usize = 96;
const SYS_MMAP: usize = 222;

const AT_FDCWD: i32 = -100;

/// Macro to generate syscall body
///
/// It will receive a function which return Result<_, LinuxError> and convert it to
/// the type which is specified by the caller.
#[macro_export]
macro_rules! syscall_body {
    ($fn: ident, $($stmt: tt)*) => {{
        #[allow(clippy::redundant_closure_call)]
        let res = (|| -> axerrno::LinuxResult<_> { $($stmt)* })();
        match res {
            Ok(_) | Err(axerrno::LinuxError::EAGAIN) => debug!(concat!(stringify!($fn), " => {:?}"),  res),
            Err(_) => info!(concat!(stringify!($fn), " => {:?}"), res),
        }
        match res {
            Ok(v) => v as _,
            Err(e) => {
                -e.code() as _
            }
        }
    }};
}

bitflags::bitflags! {
    #[derive(Debug)]
    /// permissions for sys_mmap
    ///
    /// See <https://github.com/bminor/glibc/blob/master/bits/mman.h>
    struct MmapProt: i32 {
        /// Page can be read.
        const PROT_READ = 1 << 0;
        /// Page can be written.
        const PROT_WRITE = 1 << 1;
        /// Page can be executed.
        const PROT_EXEC = 1 << 2;
    }
}

impl From<MmapProt> for MappingFlags {
    fn from(value: MmapProt) -> Self {
        let mut flags = MappingFlags::USER;
        if value.contains(MmapProt::PROT_READ) {
            flags |= MappingFlags::READ;
        }
        if value.contains(MmapProt::PROT_WRITE) {
            flags |= MappingFlags::WRITE;
        }
        if value.contains(MmapProt::PROT_EXEC) {
            flags |= MappingFlags::EXECUTE;
        }
        flags
    }
}

bitflags::bitflags! {
    #[derive(Debug)]
    /// flags for sys_mmap
    ///
    /// See <https://github.com/bminor/glibc/blob/master/bits/mman.h>
    struct MmapFlags: i32 {
        /// Share changes
        const MAP_SHARED = 1 << 0;
        /// Changes private; copy pages on write.
        const MAP_PRIVATE = 1 << 1;
        /// Map address must be exactly as requested, no matter whether it is available.
        const MAP_FIXED = 1 << 4;
        /// Don't use a file.
        const MAP_ANONYMOUS = 1 << 5;
        /// Don't check for reservations.
        const MAP_NORESERVE = 1 << 14;
        /// Allocation is for a stack.
        const MAP_STACK = 0x20000;
    }
}

#[register_trap_handler(SYSCALL)]
fn handle_syscall(tf: &TrapFrame, syscall_num: usize) -> isize {
    ax_println!("handle_syscall [{}] ...", syscall_num);
    let ret = match syscall_num {
         SYS_IOCTL => sys_ioctl(tf.arg0() as _, tf.arg1() as _, tf.arg2() as _) as _,
        SYS_SET_TID_ADDRESS => sys_set_tid_address(tf.arg0() as _),
        SYS_OPENAT => sys_openat(tf.arg0() as _, tf.arg1() as _, tf.arg2() as _, tf.arg3() as _),
        SYS_CLOSE => sys_close(tf.arg0() as _),
        SYS_READ => sys_read(tf.arg0() as _, tf.arg1() as _, tf.arg2() as _),
        SYS_WRITE => sys_write(tf.arg0() as _, tf.arg1() as _, tf.arg2() as _),
        SYS_WRITEV => sys_writev(tf.arg0() as _, tf.arg1() as _, tf.arg2() as _),
        SYS_EXIT_GROUP => {
            ax_println!("[SYS_EXIT_GROUP]: system is exiting ..");
            axtask::exit(tf.arg0() as _)
        },
        SYS_EXIT => {
            ax_println!("[SYS_EXIT]: system is exiting ..");
            axtask::exit(tf.arg0() as _)
        },
        SYS_MMAP => sys_mmap(
            tf.arg0() as _,
            tf.arg1() as _,
            tf.arg2() as _,
            tf.arg3() as _,
            tf.arg4() as _,
            tf.arg5() as _,
        ),
        _ => {
            ax_println!("Unimplemented syscall: {}", syscall_num);
            -LinuxError::ENOSYS.code() as _
        }
    };
    ret
}

fn load_file(file: &Arc<dyn FileLike>, buf: &mut [u8], _offset: isize) -> Result<usize, i32> {
    file.read(buf).map_err(|_| -1)
}

fn sys_mmap(
    addr: *mut usize,
    length: usize,
    prot: i32,
    flags: i32,
    fd: i32,
    offset: isize,
) -> isize {
    // 1. 计算映射的虚拟地址
    let vaddr = if addr.is_null() || addr as usize == 0 {
        alloc_user_vaddr(length)
    } else {
        addr as usize
    };

    // 2. 通过 fd 获取文件对象
    let file_like = match arceos_posix_api::imp::fd_ops::get_file_like(fd) {
        Ok(f) => f,
        Err(_) => return -1,
    };
    // 3. 读取文件内容到 buf
    let mut buf = vec![0u8; length];
    if load_file(&file_like, &mut buf, offset).is_err() {
        return -1;
    }

    // 4. 分页映射并拷贝数据
    let page_count = (length + PAGE_SIZE_4K - 1) / PAGE_SIZE_4K;
    let curr = current();
    let mut uspace = curr.task_ext().aspace.lock();
    for i in 0..page_count {
        let page_vaddr = vaddr + i * PAGE_SIZE_4K;
        uspace.map_alloc(
            page_vaddr.into(),
            PAGE_SIZE_4K,
            MappingFlags::from(MmapProt::from_bits_truncate(prot)) | MappingFlags::USER,
            true
        ).unwrap();
        let (paddr, _, _) = uspace.page_table().query(page_vaddr.into()).unwrap();
        let start = i * PAGE_SIZE_4K;
        let end = ((i + 1) * PAGE_SIZE_4K).min(length);
        unsafe {
            core::ptr::copy_nonoverlapping(
                buf[start..end].as_ptr(),
                phys_to_virt(paddr).as_mut_ptr(),
                end - start,
            );
        }
    }
    vaddr as isize
}

fn sys_openat(dfd: c_int, fname: *const c_char, flags: c_int, mode: api::ctypes::mode_t) -> isize {
    assert_eq!(dfd, AT_FDCWD);
    api::sys_open(fname, flags, mode) as isize
}

fn sys_close(fd: i32) -> isize {
    api::sys_close(fd) as isize
}

fn sys_read(fd: i32, buf: *mut c_void, count: usize) -> isize {
    api::sys_read(fd, buf, count)
}

fn sys_write(fd: i32, buf: *const c_void, count: usize) -> isize {
    api::sys_write(fd, buf, count)
}

fn sys_writev(fd: i32, iov: *const api::ctypes::iovec, iocnt: i32) -> isize {
    unsafe { api::sys_writev(fd, iov, iocnt) }
}

fn sys_set_tid_address(tid_ptd: *const i32) -> isize {
    let curr = current();
    curr.task_ext().set_clear_child_tid(tid_ptd as _);
    curr.id().as_u64() as isize
}

fn sys_ioctl(_fd: i32, _op: usize, _argp: *mut c_void) -> i32 {
    ax_println!("Ignore SYS_IOCTL");
    0
}

// 占位：你需要实现一个用户空间虚拟地址分配器
fn alloc_user_vaddr(_length: usize) -> usize {
    // TODO: 实现真正的分配逻辑
    0x8000_0000 // 示例返回一个固定地址
}
