use crate::prelude::Vaddr;

#[repr(C)]
struct ExTableItem {
    inst_addr: Vaddr,
    recovery_inst_addr: Vaddr,
}

unsafe extern "C" {
    fn __ex_table();
    fn __ex_table_end();
}

pub(super) struct ExTable;

impl ExTable {
    pub(super) fn find_recovery_inst_addr(inst_addr: Vaddr) -> Option<Vaddr> {
        let table_size = (__ex_table_end as *const () as usize - __ex_table as *const () as usize)
            / size_of::<ExTableItem>();
        let ex_table =
            unsafe { core::slice::from_raw_parts(__ex_table as *const ExTableItem, table_size) };
        for item in ex_table {
            if item.inst_addr == inst_addr {
                return Some(item.recovery_inst_addr);
            }
        }
        None
    }
}
