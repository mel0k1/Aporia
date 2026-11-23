use core::ops::Range;

use ostd::{
    arch::{
        cpu::context::{
            FpuContext, FsBase, GeneralRegs, GsBase, UserContext, X87_INITIAL_CONTROL_WORD,
            XStateFeatures,
        },
        trap::{USER_CS_VALUE, USER_SS_VALUE},
    },
    mm::MAX_USERSPACE_VADDR,
};
use spin::Once;
use x86_64::registers::rflags::RFlags;

use crate::{prelude::*, process::posix_thread::SuppUserContext};

static MXCSR_MASK: Once<u32> = Once::new();

pub(super) fn init() {
    MXCSR_MASK.call_once(|| {
        let mut context = FpuContext::new();
        context.save();
        let regs = CUserFpregsStruct::from_bytes(context.fxsave_area().as_bytes());

        if regs.mxcsr_mask == 0 {
            0x0000_ffbf
        } else {
            regs.mxcsr_mask
        }
    });
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod)]
pub(crate) struct CUserRegsStruct {
    pub r15: usize,
    pub r14: usize,
    pub r13: usize,
    pub r12: usize,
    pub rbp: usize,
    pub rbx: usize,
    pub r11: usize,
    pub r10: usize,
    pub r9: usize,
    pub r8: usize,
    pub rax: usize,
    pub rcx: usize,
    pub rdx: usize,
    pub rsi: usize,
    pub rdi: usize,
    pub orig_rax: usize,
    pub rip: usize,
    pub cs: usize,
    pub rflags: usize,
    pub rsp: usize,
    pub ss: usize,
    pub fsbase: usize,
    pub gsbase: usize,
    pub ds: usize,
    pub es: usize,
    pub fs: usize,
    pub gs: usize,
}

impl CUserRegsStruct {
    fn from_regs(user_context: &UserContext, fs_base: FsBase, gs_base: GsBase) -> Self {
        let mut out = Self::default();
        let bytes = out.as_mut_bytes();
        for rule in REG_RULES {
            let value = match rule.policy {
                Policy::Fixed(val) => val,
                _ => (rule.get.unwrap())(user_context.general_regs()),
            };
            write_word(bytes, rule.offset, value);
        }
        write_word(
            bytes,
            core::mem::offset_of!(CUserRegsStruct, rflags),
            user_context.rflags(),
        );
        write_word(
            bytes,
            core::mem::offset_of!(CUserRegsStruct, fsbase),
            fs_base.addr(),
        );
        write_word(
            bytes,
            core::mem::offset_of!(CUserRegsStruct, gsbase),
            gs_base.addr(),
        );
        out
    }

    fn apply_prefix_to(
        &self,
        user_context: &mut UserContext,
        fs_base: &mut FsBase,
        gs_base: &mut GsBase,
        len: usize,
    ) -> Result<()> {
        const ORIG_RAX_OFFSET: usize = core::mem::offset_of!(CUserRegsStruct, orig_rax);
        const RFLAGS_OFFSET: usize = core::mem::offset_of!(CUserRegsStruct, rflags);
        const FSBASE_OFFSET: usize = core::mem::offset_of!(CUserRegsStruct, fsbase);
        const GSBASE_OFFSET: usize = core::mem::offset_of!(CUserRegsStruct, gsbase);

        let bytes = self.as_bytes();
        debug_assert!(len <= bytes.len() && len.is_multiple_of(size_of::<usize>()));

        let mut new_user_context = user_context.clone();
        let mut new_fs_base = *fs_base;
        let mut new_gs_base = *gs_base;
        for offset in (0..len).step_by(size_of::<usize>()) {
            let value = read_word(bytes, offset);
            match offset {
                ORIG_RAX_OFFSET => {}
                RFLAGS_OFFSET => new_user_context.set_rflags(value),
                FSBASE_OFFSET | GSBASE_OFFSET => {
                    if !is_user_addr(value) {
                        return Err(errno!(EIO, "invalid register value"));
                    }
                    if offset == FSBASE_OFFSET {
                        new_fs_base = FsBase::new(value);
                    } else {
                        new_gs_base = GsBase::new(value);
                    }
                }
                _ => {
                    let rule = RegRule::for_offset(offset)
                        .expect("every other register has a rule in `REG_RULES`");
                    rule.apply(new_user_context.general_regs_mut(), value)?;
                }
            }
        }

        *user_context = new_user_context;
        *fs_base = new_fs_base;
        *gs_base = new_gs_base;
        Ok(())
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
struct CUserFpregsStruct {
    cwd: u16,
    swd: u16,
    ftw: u16,
    fop: u16,
    rip: u64,
    rdp: u64,
    mxcsr: u32,
    mxcsr_mask: u32,
    st_space: [u32; 32],
    xmm_space: [u32; 64],
    padding: [u32; 24],
}

impl CUserFpregsStruct {
    fn from_context(context: &FpuContext) -> Self {
        let mut out = Self::from_bytes(context.fxsave_area().as_bytes());

        let Some(features) = context.features() else {
            return out;
        };

        if !features.contains(XStateFeatures::X87) {
            out.cwd = X87_INITIAL_CONTROL_WORD;
            out.swd = 0;
            out.ftw &= 0xff00;
            out.fop = 0;
            out.rip = 0;
            out.rdp = 0;
            out.st_space.fill(0);
        }
        if !features.contains(XStateFeatures::SSE) {
            out.xmm_space.fill(0);
        }
        out.padding.fill(0);

        out
    }

    fn apply_to(&self, context: &mut FpuContext) -> Result<()> {
        if self.mxcsr & !*MXCSR_MASK.get().unwrap() != 0 {
            return Err(errno!(EINVAL, "invalid MXCSR value"));
        }

        context
            .fxsave_area_mut()
            .as_mut_bytes()
            .copy_from_slice(self.as_bytes());

        if let Some(features) = context.features_mut() {
            features.insert(XStateFeatures::X87 | XStateFeatures::SSE);
        }
        Ok(())
    }
}

#[repr(u32)]
#[derive(Clone, Copy, Debug, TryFromInt)]
pub(crate) enum Regset {
    General = 1,
    Fp = 2,
}

impl Regset {
    pub(crate) fn validate_and_truncate_len(self, len: usize) -> Result<usize> {
        let size = match self {
            Self::General => size_of::<CUserRegsStruct>(),
            Self::Fp => size_of::<CUserFpregsStruct>(),
        };

        if !len.is_multiple_of(size_of::<usize>()) {
            return Err(errno!(EINVAL, "invalid register set length"));
        }
        Ok(len.min(size))
    }
}

pub(crate) struct PtraceState {
    user_context: UserContext,
    fs_base: FsBase,
    gs_base: GsBase,
    orig_syscall_ret: usize,
    fpu_context: FpuContext,
    is_fpu_dirty: bool,
}

impl PtraceState {
    const NOT_A_SYSCALL: usize = usize::MAX;

    pub(crate) fn new(
        user_context: &UserContext,
        supp_context: &SuppUserContext,
        orig_syscall_ret: Option<usize>,
    ) -> Self {
        Self {
            user_context: user_context.clone(),
            fs_base: supp_context.fs_base().get(),
            gs_base: supp_context.gs_base().get(),
            orig_syscall_ret: orig_syscall_ret.unwrap_or(Self::NOT_A_SYSCALL),
            fpu_context: supp_context.fpu().get(),
            is_fpu_dirty: false,
        }
    }

    pub(crate) fn restore(self, user_context: &mut UserContext, supp_context: &SuppUserContext) {
        *user_context = self.user_context;
        supp_context.fs_base().set(self.fs_base);
        supp_context.gs_base().set(self.gs_base);
        if self.is_fpu_dirty {
            supp_context.fpu().set(self.fpu_context);
        }
    }

    pub(crate) fn orig_syscall_ret(&self) -> Option<usize> {
        (self.orig_syscall_ret != Self::NOT_A_SYSCALL).then_some(self.orig_syscall_ret)
    }

    pub(crate) fn get_user_regs(&self) -> CUserRegsStruct {
        let mut regs = CUserRegsStruct::from_regs(&self.user_context, self.fs_base, self.gs_base);
        regs.orig_rax = self.orig_syscall_ret;
        regs
    }

    pub(crate) fn set_user_regs(&mut self, regs: CUserRegsStruct) -> Result<()> {
        self.set_user_regs_prefix(regs, size_of::<CUserRegsStruct>())
    }

    fn set_user_regs_prefix(&mut self, regs: CUserRegsStruct, len: usize) -> Result<()> {
        regs.apply_prefix_to(
            &mut self.user_context,
            &mut self.fs_base,
            &mut self.gs_base,
            len,
        )?;
        if core::mem::offset_of!(CUserRegsStruct, orig_rax) < len {
            self.orig_syscall_ret = regs.orig_rax;
        }
        Ok(())
    }

    pub(crate) fn get_regset(&self, regset: Regset, writer: &mut VmWriter) -> Result<()> {
        match regset {
            Regset::General => {
                let regs = self.get_user_regs();
                writer.write_fallible(&mut VmReader::from(regs.as_bytes()))?;
            }
            Regset::Fp => {
                let regs = CUserFpregsStruct::from_context(&self.fpu_context);
                writer.write_fallible(&mut VmReader::from(regs.as_bytes()))?;
            }
        }
        Ok(())
    }

    pub(crate) fn set_regset(&mut self, regset: Regset, reader: &mut VmReader) -> Result<()> {
        match regset {
            Regset::General => {
                if !reader.has_remain() {
                    return Ok(());
                }
                let mut regs = CUserRegsStruct::default();
                let len = reader.read_fallible(&mut VmWriter::from(regs.as_mut_bytes()))?;
                self.set_user_regs_prefix(regs, len)
            }
            Regset::Fp => {
                if reader.remain() != size_of::<CUserFpregsStruct>() {
                    return Err(errno!(EINVAL, "invalid FPU register set length"));
                }
                let regs = reader.read_val::<CUserFpregsStruct>()?;
                regs.apply_to(&mut self.fpu_context)?;
                self.is_fpu_dirty = true;
                Ok(())
            }
        }
    }

    pub(crate) fn peek_user(&self, offset: usize) -> Result<usize> {
        check_user_offset(offset)?;
        if offset == core::mem::offset_of!(CUserRegsStruct, rflags) {
            return Ok(self.user_context.rflags());
        }
        if offset == core::mem::offset_of!(CUserRegsStruct, fsbase) {
            return Ok(self.fs_base.addr());
        }
        if offset == core::mem::offset_of!(CUserRegsStruct, gsbase) {
            return Ok(self.gs_base.addr());
        }
        if offset == core::mem::offset_of!(CUserRegsStruct, orig_rax) {
            return Ok(self.orig_syscall_ret);
        }

        if let Some(index) = debug_register_index(offset) {
            let value = match index {
                6 => DEBUG_STATUS_DEFAULT_VALUE,
                0..=5 | 7 => 0,
                _ => unreachable!(),
            };
            return Ok(value);
        }

        let rule =
            RegRule::for_offset(offset).expect("offset has been validated by `check_user_offset`");
        Ok(match rule.policy {
            Policy::Fixed(value) => value,
            _ => (rule.get.unwrap())(self.user_context.general_regs()),
        })
    }

    pub(crate) fn poke_user(&mut self, offset: usize, value: usize) -> Result<()> {
        check_user_offset(offset)?;
        if offset == core::mem::offset_of!(CUserRegsStruct, rflags) {
            self.user_context.set_rflags(value);
            return Ok(());
        }
        if offset == core::mem::offset_of!(CUserRegsStruct, fsbase) {
            if !is_user_addr(value) {
                return Err(errno!(EIO, "invalid register value"));
            }
            self.fs_base = FsBase::new(value);
            return Ok(());
        }
        if offset == core::mem::offset_of!(CUserRegsStruct, gsbase) {
            if !is_user_addr(value) {
                return Err(errno!(EIO, "invalid register value"));
            }
            self.gs_base = GsBase::new(value);
            return Ok(());
        }
        if offset == core::mem::offset_of!(CUserRegsStruct, orig_rax) {
            self.orig_syscall_ret = value;
            return Ok(());
        }
        if debug_register_index(offset).is_some() {
            return Err(errno!(
                EOPNOTSUPP,
                "writing x86 debug registers is not supported currently"
            ));
        }

        let rule =
            RegRule::for_offset(offset).expect("offset has been validated by `check_user_offset`");
        rule.apply(self.user_context.general_regs_mut(), value)
    }

    pub(crate) fn enable_single_step(&mut self) {
        self.user_context
            .set_rflags(self.user_context.rflags() | RFlags::TRAP_FLAG.bits() as usize);
    }

    pub(crate) fn disable_single_step(&mut self) {
        self.user_context
            .set_rflags(self.user_context.rflags() & !(RFlags::TRAP_FLAG.bits() as usize));
    }
}

macro_rules! off {
    ($field:ident) => {
        core::mem::offset_of!(CUserRegsStruct, $field)
    };
}

const REG_RULES: &[RegRule] = &[
    RegRule::rw(off!(rax), |r| r.rax, |r, v| r.rax = v, Policy::Set),
    RegRule::rw(off!(rbx), |r| r.rbx, |r, v| r.rbx = v, Policy::Set),
    RegRule::rw(off!(rcx), |r| r.rcx, |r, v| r.rcx = v, Policy::Set),
    RegRule::rw(off!(rdx), |r| r.rdx, |r, v| r.rdx = v, Policy::Set),
    RegRule::rw(off!(rsi), |r| r.rsi, |r, v| r.rsi = v, Policy::Set),
    RegRule::rw(off!(rdi), |r| r.rdi, |r, v| r.rdi = v, Policy::Set),
    RegRule::rw(off!(rbp), |r| r.rbp, |r, v| r.rbp = v, Policy::Set),
    RegRule::rw(off!(r8), |r| r.r8, |r, v| r.r8 = v, Policy::Set),
    RegRule::rw(off!(r9), |r| r.r9, |r, v| r.r9 = v, Policy::Set),
    RegRule::rw(off!(r10), |r| r.r10, |r, v| r.r10 = v, Policy::Set),
    RegRule::rw(off!(r11), |r| r.r11, |r, v| r.r11 = v, Policy::Set),
    RegRule::rw(off!(r12), |r| r.r12, |r, v| r.r12 = v, Policy::Set),
    RegRule::rw(off!(r13), |r| r.r13, |r, v| r.r13 = v, Policy::Set),
    RegRule::rw(off!(r14), |r| r.r14, |r, v| r.r14 = v, Policy::Set),
    RegRule::rw(off!(r15), |r| r.r15, |r, v| r.r15 = v, Policy::Set),
    RegRule::rw(
        off!(rsp),
        |r| r.rsp,
        |r, v| r.rsp = v,
        Policy::SetIf(is_user_addr),
    ),
    RegRule::rw(
        off!(rip),
        |r| r.rip,
        |r, v| r.rip = v,
        Policy::SetIf(is_user_addr),
    ),
    RegRule::fixed(off!(cs), USER_CS_VALUE),
    RegRule::fixed(off!(ss), USER_SS_VALUE),
    RegRule::fixed(off!(ds), 0),
    RegRule::fixed(off!(es), 0),
    RegRule::fixed(off!(fs), 0),
    RegRule::fixed(off!(gs), 0),
];

const _: () = {
    assert!((REG_RULES.len() + 4) * size_of::<usize>() == size_of::<CUserRegsStruct>());
};

struct RegRule {
    offset: usize,
    get: Option<fn(&GeneralRegs) -> usize>,
    set: Option<fn(&mut GeneralRegs, usize)>,
    policy: Policy,
}

impl RegRule {
    const fn rw(
        offset: usize,
        get: fn(&GeneralRegs) -> usize,
        set: fn(&mut GeneralRegs, usize),
        policy: Policy,
    ) -> Self {
        Self {
            offset,
            get: Some(get),
            set: Some(set),
            policy,
        }
    }

    const fn fixed(offset: usize, expected: usize) -> Self {
        Self {
            offset,
            get: None,
            set: None,
            policy: Policy::Fixed(expected),
        }
    }

    fn for_offset(offset: usize) -> Option<&'static Self> {
        REG_RULES.iter().find(|rule| rule.offset == offset)
    }

    fn apply(&self, regs: &mut GeneralRegs, value: usize) -> Result<()> {
        match self.policy {
            Policy::Set => {
                (self.set.unwrap())(regs, value);
            }
            Policy::SetIf(check) => {
                if !check(value) {
                    return Err(errno!(EIO, "invalid register value"));
                }
                (self.set.unwrap())(regs, value);
            }
            Policy::Fixed(expected) => {
                if value != expected {
                    return Err(errno!(EIO, "invalid fixed register value"));
                }
            }
        }

        Ok(())
    }
}

enum Policy {
    Set,
    SetIf(fn(usize) -> bool),
    Fixed(usize),
}

const fn is_user_addr(v: usize) -> bool {
    v < MAX_USERSPACE_VADDR
}

fn check_user_offset(offset: usize) -> Result<()> {
    if !offset.is_multiple_of(size_of::<usize>()) {
        return Err(errno!(EIO, "invalid USER area offset"));
    }

    if offset < size_of::<CUserRegsStruct>() {
        return Ok(());
    }

    if debug_register_index(offset).is_some() {
        return Ok(());
    }

    Err(errno!(
        EOPNOTSUPP,
        "only offsets for general-purpose registers and debug registers are supported currently"
    ))
}

const DEBUG_REGS_OFFSET: usize = 848;
const DEBUG_REGS_COUNT: usize = 8;
const DEBUG_STATUS_DEFAULT_VALUE: usize = 0xffff0ff0;

const fn debug_register_index(offset: usize) -> Option<usize> {
    if !offset.is_multiple_of(size_of::<usize>()) {
        return None;
    }

    if offset < DEBUG_REGS_OFFSET
        || offset >= DEBUG_REGS_OFFSET + DEBUG_REGS_COUNT * size_of::<usize>()
    {
        return None;
    }

    Some((offset - DEBUG_REGS_OFFSET) / size_of::<usize>())
}

fn read_word(bytes: &[u8], offset: usize) -> usize {
    usize::from_ne_bytes(bytes[word_range(offset)].try_into().unwrap())
}

fn write_word(bytes: &mut [u8], offset: usize, value: usize) {
    bytes[word_range(offset)].copy_from_slice(&value.to_ne_bytes());
}

const fn word_range(offset: usize) -> Range<usize> {
    offset..offset + size_of::<usize>()
}

const LINUX_USER_CS: usize = 0x33;
const LINUX_USER_SS: usize = 0x2b;

ostd::const_assert!(LINUX_USER_CS == USER_CS_VALUE);
ostd::const_assert!(LINUX_USER_SS == USER_SS_VALUE);
