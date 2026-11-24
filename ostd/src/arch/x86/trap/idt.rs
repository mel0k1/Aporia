//! Configure the Interrupt Descriptor Table (IDT).

use core::arch::global_asm;

use spin::Once;
use x86_64::{
    PrivilegeLevel, VirtAddr,
    instructions::tables::lidt,
    structures::{DescriptorTablePointer, idt::Entry},
};

use crate::mm::Vaddr;

global_asm!(include_str!("trap.S"));

const NUM_INTERRUPTS: usize = 256;

unsafe extern "C" {
    #[link_name = "trap_handler_table"]
    static VECTORS: [usize; NUM_INTERRUPTS];
}

static GLOBAL_IDT: Once<[Entry<()>; NUM_INTERRUPTS]> = Once::new();

/// Returns the IDT base address.
pub(in crate::arch) fn idt_base() -> Vaddr {
    GLOBAL_IDT.get().unwrap().as_ptr() as Vaddr
}

/// Initializes and loads the IDT.
///
/// The caller should only call this method once in the boot context for each available processor.
/// This is not a safety requirement, however, because calling this method again will do nothing
/// more than load the same IDT.
pub(super) fn init_on_cpu() {
    let idt = GLOBAL_IDT.call_once(|| {
        let mut idt = [const { Entry::missing() }; NUM_INTERRUPTS];

        let vectors = unsafe { &VECTORS };

        for (intr_no, &handler) in vectors.iter().enumerate() {
            let handler = VirtAddr::new(handler as u64);

            let entry = &mut idt[intr_no];

            let opt = unsafe { entry.set_handler_addr(handler) };

            if intr_no == 3 || intr_no == 4 {
                opt.set_privilege_level(PrivilegeLevel::Ring3);
            }
        }

        idt
    });

    let idtr = DescriptorTablePointer {
        limit: (size_of_val(idt) - 1) as u16,
        base: VirtAddr::new(idt.as_ptr().addr() as u64),
    };

    unsafe { lidt(&idtr) };
}
