#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VcpuRegs {
    pub rax: usize,
    pub rbx: usize,
    pub rcx: usize,
    pub rdx: usize,
    pub rsi: usize,
    pub rdi: usize,
    pub rbp: usize,
    pub rsp: usize,
    pub r8: usize,
    pub r9: usize,
    pub r10: usize,
    pub r11: usize,
    pub r12: usize,
    pub r13: usize,
    pub r14: usize,
    pub r15: usize,
    pub rip: usize,
    pub rflags: usize,
}

impl VcpuRegs {
    pub const fn new() -> Self {
        const RFLAGS_FIXED_BIT: usize = 1 << 1;

        Self {
            rax: 0,
            rbx: 0,
            rcx: 0,
            rdx: 0,
            rsi: 0,
            rdi: 0,
            rbp: 0,
            rsp: 0,
            r8: 0,
            r9: 0,
            r10: 0,
            r11: 0,
            r12: 0,
            r13: 0,
            r14: 0,
            r15: 0,
            rip: 0,
            rflags: RFLAGS_FIXED_BIT,
        }
    }
}

impl Default for VcpuRegs {
    fn default() -> Self {
        Self::new()
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct VcpuSregs {
    pub cs: VcpuSegment,
    pub ds: VcpuSegment,
    pub es: VcpuSegment,
    pub fs: VcpuSegment,
    pub gs: VcpuSegment,
    pub ss: VcpuSegment,
    pub tr: VcpuSegment,
    pub ldt: VcpuSegment,
    pub gdt: VcpuDescTable,
    pub idt: VcpuDescTable,
    pub cr0: u64,
    pub cr2: u64,
    pub cr3: u64,
    pub cr4: u64,
    pub efer: u64,
    pub apic_base: u64,
    pub interrupt_bitmap: [u64; 4],
}

impl VcpuSregs {
    pub fn new_real_mode() -> Self {
        const CR0_EXTENSION_TYPE: u64 = 1 << 4;
        const DATA_READ_WRITE_ACCESSED: u8 = 0x3;
        const CODE_EXEC_READ_ACCESSED: u8 = 0xb;
        const BUSY_TSS: u8 = 0xb;

        let data = VcpuSegment {
            limit: 0xffff,
            type_: DATA_READ_WRITE_ACCESSED,
            present: 1,
            s: 1,
            ..VcpuSegment::default()
        };

        Self {
            cs: VcpuSegment {
                type_: CODE_EXEC_READ_ACCESSED,
                ..data
            },
            ds: data,
            es: data,
            fs: data,
            gs: data,
            ss: data,
            tr: VcpuSegment {
                type_: BUSY_TSS,
                s: 0,
                ..data
            },
            ldt: VcpuSegment {
                unusable: 1,
                ..VcpuSegment::default()
            },
            cr0: CR0_EXTENSION_TYPE,
            ..Self::default()
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct VcpuSegment {
    pub base: u64,
    pub limit: u32,
    pub selector: u16,
    pub type_: u8,
    pub present: u8,
    pub dpl: u8,
    pub db: u8,
    pub s: u8,
    pub l: u8,
    pub g: u8,
    pub avl: u8,
    pub unusable: u8,
    pub padding: u8,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct VcpuDescTable {
    pub base: u64,
    pub limit: u16,
    pub padding: [u16; 3],
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct VcpuMsrs {
    pub pat: u64,
    pub kernel_gs_base: u64,
    pub star: u64,
    pub lstar: u64,
    pub syscall_mask: u64,
    pub tsc_adjust: u64,
    pub tsc_aux: u64,
    pub sysenter_cs: u64,
    pub sysenter_esp: u64,
    pub sysenter_eip: u64,
    pub misc_enable: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GuestInterrupt {
    pub vector: u8,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GuestTimerInstant {
    pub tsc: u64,
}
