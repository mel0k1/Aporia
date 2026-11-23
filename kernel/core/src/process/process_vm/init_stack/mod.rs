use core::{
    ops::Range,
    sync::atomic::{AtomicUsize, Ordering},
};

use self::aux_vec::{AuxKey, AuxVec};
use crate::{
    prelude::*,
    util::random::getrandom,
    vm::{
        page_cache::{Vmo, VmoOptions},
        perms::VmPerms,
        vmar::{VMAR_CAP_ADDR, Vmar, VmarMapOffset},
    },
};

pub(crate) mod aux_vec;

pub(crate) const INIT_STACK_SIZE: usize = 8 * 1024 * 1024;

pub(crate) const MAX_NR_STRING_ARGS: usize = i32::MAX as usize;

pub(crate) const MAX_LEN_STRING_ARG: usize = PAGE_SIZE * 32;

pub(crate) struct InitStack {
    initial_top: Vaddr,
    max_size: usize,
    pos: AtomicUsize,
    auxv_range: SpinLock<Range<Vaddr>>,
    argv_range: SpinLock<Range<Vaddr>>,
    envp_range: SpinLock<Range<Vaddr>>,
}

impl Clone for InitStack {
    fn clone(&self) -> Self {
        Self {
            initial_top: self.initial_top,
            max_size: self.max_size,
            pos: AtomicUsize::new(self.pos.load(Ordering::Relaxed)),
            auxv_range: SpinLock::new(self.auxv_range.lock().clone()),
            argv_range: SpinLock::new(self.argv_range.lock().clone()),
            envp_range: SpinLock::new(self.envp_range.lock().clone()),
        }
    }
}

impl InitStack {
    pub(crate) fn new() -> Self {
        let nr_pages_padding = {
            const NR_FIXED_PADDING_PAGES: usize = 7;

            let mut nr_random_padding_pages: u8 = 0;
            getrandom(nr_random_padding_pages.as_mut_bytes());

            nr_random_padding_pages as usize + NR_FIXED_PADDING_PAGES
        };
        let initial_top = VMAR_CAP_ADDR - PAGE_SIZE * nr_pages_padding;
        let max_size = INIT_STACK_SIZE;

        Self {
            initial_top,
            max_size,
            pos: AtomicUsize::new(initial_top),
            auxv_range: SpinLock::new(0..0),
            argv_range: SpinLock::new(0..0),
            envp_range: SpinLock::new(0..0),
        }
    }

    pub(crate) fn user_stack_top(&self) -> Vaddr {
        debug_assert!(self.is_initialized());

        self.pos()
    }

    pub(crate) fn argv_range(&self) -> Range<Vaddr> {
        self.argv_range.lock().clone()
    }

    pub(crate) fn envp_range(&self) -> Range<Vaddr> {
        self.envp_range.lock().clone()
    }

    pub(super) fn map_and_write(
        &self,
        vmar: &Vmar,
        argv: Vec<CString>,
        envp: Vec<CString>,
        auxvec: AuxVec,
    ) -> Result<()> {
        self.set_uninitialized();

        let vmo = {
            let vmo_options = VmoOptions::new(self.max_size);
            vmo_options.alloc()?
        };
        let vmar_map_options = {
            let perms = VmPerms::READ | VmPerms::WRITE;
            let map_addr = self.initial_top - self.max_size;
            debug_assert!(map_addr.is_multiple_of(PAGE_SIZE));
            vmar.new_map(self.max_size, perms)
                .offset(VmarMapOffset::FixedNoReplace(map_addr))
                .vmo(vmo.clone())
        };
        vmar_map_options.build()?;

        let writer = InitStackWriter {
            pos: &self.pos,
            vmo,
            argv,
            envp,
            auxvec,
            map_addr: self.initial_top - self.max_size,
        };
        let (auxv_range, argv_range, envp_range) = writer.write()?;

        *self.auxv_range.lock() = auxv_range;
        *self.argv_range.lock() = argv_range;
        *self.envp_range.lock() = envp_range;

        Ok(())
    }

    pub(super) fn reader<'a>(&self, vmar: &'a Vmar) -> InitStackReader<'a> {
        debug_assert!(self.is_initialized());

        InitStackReader {
            vmar,
            auxv_range: self.auxv_range.lock().clone(),
            argv_range: self.argv_range.lock().clone(),
            envp_range: self.envp_range.lock().clone(),
        }
    }

    fn is_initialized(&self) -> bool {
        self.pos() != self.initial_top
    }

    fn set_uninitialized(&self) {
        self.pos.store(self.initial_top, Ordering::Relaxed);
    }

    fn pos(&self) -> Vaddr {
        self.pos.load(Ordering::Relaxed)
    }
}

struct InitStackWriter<'a> {
    pos: &'a AtomicUsize,
    vmo: Arc<Vmo>,
    argv: Vec<CString>,
    envp: Vec<CString>,
    auxvec: AuxVec,
    map_addr: usize,
}

impl InitStackWriter<'_> {
    fn write(mut self) -> Result<(Range<Vaddr>, Range<Vaddr>, Range<Vaddr>)> {
        let envp_end = self.pos();
        let envp_pointers = self.write_envp_strings()?;
        let envp_start = self.pos();

        let argv_end = self.pos();
        let argv_pointers = self.write_argv_strings()?;
        let argv_start = self.pos();

        let random_value_pointer = {
            let random_value = generate_random_for_aux_vec();
            self.write_bytes(&random_value)?
        };
        self.auxvec.set(AuxKey::AT_RANDOM, random_value_pointer);

        self.adjust_stack_alignment(&envp_pointers, &argv_pointers)?;
        let auxv_end = self.pos();
        self.write_aux_vec()?;
        let auxv_start = self.pos();
        self.write_envp_pointers(envp_pointers)?;
        self.write_argv_pointers(argv_pointers)?;

        let argc = self.argv.len();
        self.write_u64(argc as u64)?;

        debug_assert_eq!(self.pos() & !0xf, self.pos());

        Ok((
            auxv_start..auxv_end,
            argv_start..argv_end,
            envp_start..envp_end,
        ))
    }

    fn write_envp_strings(&self) -> Result<Vec<u64>> {
        let mut envp_pointers = Vec::with_capacity(self.envp.len());
        for envp in self.envp.iter() {
            let pointer = self.write_cstring(envp)?;
            envp_pointers.push(pointer);
        }
        Ok(envp_pointers)
    }

    fn write_argv_strings(&self) -> Result<Vec<u64>> {
        let mut argv_pointers = Vec::with_capacity(self.argv.len());
        for argv in self.argv.iter().rev() {
            let pointer = self.write_cstring(argv)?;
            debug!("argv address = 0x{:x}", pointer);
            argv_pointers.push(pointer);
        }
        argv_pointers.reverse();
        Ok(argv_pointers)
    }

    fn adjust_stack_alignment(&self, envp_pointers: &[u64], argv_pointers: &[u64]) -> Result<()> {
        self.write_u64(0)?;
        let auxvec_size = (self.auxvec.table().len() + 1) * (size_of::<u64>() * 2);
        let envp_pointers_size = (envp_pointers.len() + 1) * size_of::<u64>();
        let argv_pointers_size = (argv_pointers.len() + 1) * size_of::<u64>();
        let argc_size = size_of::<u64>();
        let to_write_size = auxvec_size + envp_pointers_size + argv_pointers_size + argc_size;
        if !(self.pos() - to_write_size).is_multiple_of(16) {
            self.write_u64(0)?;
        }
        Ok(())
    }

    fn write_aux_vec(&self) -> Result<()> {
        self.write_u64(0)?;
        self.write_u64(AuxKey::AT_NULL as u64)?;
        let aux_vec: Vec<_> = self
            .auxvec
            .table()
            .iter()
            .map(|(aux_key, aux_value)| (*aux_key, *aux_value))
            .collect();
        for (aux_key, aux_value) in aux_vec.iter() {
            self.write_u64(*aux_value)?;
            self.write_u64(*aux_key as u64)?;
        }
        Ok(())
    }

    fn write_envp_pointers(&self, mut envp_pointers: Vec<u64>) -> Result<()> {
        self.write_u64(0)?;
        envp_pointers.reverse();
        for envp_pointer in envp_pointers {
            self.write_u64(envp_pointer)?;
        }
        Ok(())
    }

    fn write_argv_pointers(&self, mut argv_pointers: Vec<u64>) -> Result<()> {
        self.write_u64(0)?;
        argv_pointers.reverse();
        for argv_pointer in argv_pointers {
            self.write_u64(argv_pointer)?;
        }
        Ok(())
    }

    fn write_u64(&self, val: u64) -> Result<u64> {
        let new_pos = self.reserve_pos(size_of::<u64>(), align_of::<u64>())?;
        let bytes = val.to_ne_bytes();
        let mut reader = VmReader::from(bytes.as_slice()).to_fallible();
        self.vmo.write(new_pos - self.map_addr, &mut reader)?;
        Ok(new_pos as u64)
    }

    fn write_cstring(&self, val: &CString) -> Result<u64> {
        let bytes = val.as_bytes_with_nul();
        self.write_bytes(bytes)
    }

    fn write_bytes(&self, bytes: &[u8]) -> Result<u64> {
        let new_pos = self.reserve_pos(bytes.len(), align_of::<u8>())?;
        let mut reader = VmReader::from(bytes).to_fallible();
        self.vmo.write(new_pos - self.map_addr, &mut reader)?;
        Ok(new_pos as u64)
    }

    fn reserve_pos(&self, size: usize, align: usize) -> Result<Vaddr> {
        if let Some(pos) = self.pos().checked_sub(size)
            && let new_pos = pos & !(align - 1)
            && new_pos >= self.map_addr
        {
            self.pos.store(new_pos, Ordering::Relaxed);
            return Ok(new_pos);
        }
        Err(errno!(E2BIG, "the init stack overflows"))
    }

    fn pos(&self) -> Vaddr {
        self.pos.load(Ordering::Relaxed)
    }
}

fn generate_random_for_aux_vec() -> [u8; 16] {
    let mut rand_val = [0; 16];
    getrandom(&mut rand_val);
    rand_val
}

pub(crate) struct InitStackReader<'a> {
    vmar: &'a Vmar,
    auxv_range: Range<Vaddr>,
    argv_range: Range<Vaddr>,
    envp_range: Range<Vaddr>,
}

impl InitStackReader<'_> {
    pub(crate) fn auxv(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        self.read_range(offset, writer, &self.auxv_range)
    }

    pub(crate) fn argv(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        self.read_range(offset, writer, &self.argv_range)
    }

    pub(crate) fn envp(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        self.read_range(offset, writer, &self.envp_range)
    }

    fn read_range(
        &self,
        offset: usize,
        writer: &mut VmWriter,
        range: &Range<Vaddr>,
    ) -> Result<usize> {
        if offset >= range.end - range.start {
            return Ok(0);
        }

        let read_at = range.start + offset;
        writer.limit(range.end - read_at);
        let bytes_read = self.vmar.read_alien(read_at, writer)?;

        Ok(bytes_read)
    }
}
